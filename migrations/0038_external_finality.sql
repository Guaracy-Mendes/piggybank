-- Core alone records final financial evidence and its matching booking/release.
CREATE TABLE core.external_finalities(
 operation_id uuid PRIMARY KEY REFERENCES core.external_reservations(operation_id),request_digest text NOT NULL,
 outcome text NOT NULL CHECK(outcome IN ('SETTLED','DECLINED','CANCELLED')),evidence_id uuid NOT NULL UNIQUE,
 evidence_digest text NOT NULL,document jsonb NOT NULL,batch_id uuid REFERENCES core.journal_batches(id),recorded_at timestamptz NOT NULL DEFAULT now(),
 CHECK((outcome='SETTLED')=(batch_id IS NOT NULL))
);
CREATE TRIGGER immutable_external_finalities BEFORE UPDATE OR DELETE OR TRUNCATE ON core.external_finalities FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal();
REVOKE ALL ON core.external_finalities FROM bank_core;
GRANT SELECT ON core.external_finalities TO bank_core;
CREATE FUNCTION core.finalize_external(p_operation uuid,p_digest text,p_packet jsonb,p_evidence_digest text)
RETURNS TABLE(outcome text,batch_id uuid,created boolean)
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,core AS $$
DECLARE r core.external_reservations; h core.holds; previous core.external_finalities; e jsonb:=p_packet->'body';
DECLARE payer uuid; clearing uuid:='4f1a780b-3e31-48ef-a294-000000000003'; fees uuid:='4f1a780b-3e31-48ef-a294-000000000004';
DECLARE principal bigint; fee bigint; total bigint; target uuid:=NULL; final_state text; day date; product uuid; account core.ledger_accounts;
BEGIN
 PERFORM pg_advisory_xact_lock(hashtextextended(p_operation::text,0));
 SELECT * INTO previous FROM core.external_finalities WHERE operation_id=p_operation;
 IF FOUND THEN
  IF previous.request_digest<>p_digest OR previous.evidence_digest<>p_evidence_digest THEN RAISE EXCEPTION 'Conflicting immutable financial finality'; END IF;
  RETURN QUERY SELECT previous.outcome,previous.batch_id,false; RETURN;
 END IF;
 SELECT * INTO STRICT r FROM core.external_reservations WHERE operation_id=p_operation AND request_digest=p_digest;
 payer:=(r.instruction->'details'->>'payerAccountId')::uuid;
 principal:=(r.instruction->'details'->'amount'->>'minorUnits')::bigint;
 fee:=(r.instruction->'details'->'fee'->>'minorUnits')::bigint;total:=principal+fee;
 day:=(r.instruction->'details'->>'requestedDate')::date;product:=(r.instruction->>'productVersionId')::uuid;
 IF (e->>'provider') IS DISTINCT FROM 'SYNTHETIC_RAIL' OR (e->>'instructionId') IS DISTINCT FROM p_operation::text
 OR (e->>'paymentId') IS DISTINCT FROM p_operation::text OR (e->>'inputDigest') IS DISTINCT FROM p_digest
 OR (e->'amount') IS DISTINCT FROM (r.instruction->'details'->'amount') OR coalesce((e->>'version')::bigint,0)<=0
 OR e->>'status' IS NULL OR e->>'status' NOT IN ('SETTLED','REJECTED','CANCELLED') OR e->>'evidenceId' IS NULL
 THEN RAISE EXCEPTION 'Exact authenticated final financial evidence required'; END IF;
 PERFORM 1 FROM core.ledger_accounts WHERE id IN(payer,clearing) OR (id=fees AND fee>0) ORDER BY id FOR UPDATE;
 SELECT * INTO STRICT h FROM core.holds WHERE operation_id=p_operation FOR UPDATE;
 IF h.state<>'ACTIVE' OR NOT h.submitted OR h.account_id<>payer OR h.amount<>total THEN RAISE EXCEPTION 'Original submitted reservation required'; END IF;
 IF e->>'status'='SETTLED' THEN
  IF e->>'settlementDate' IS NULL OR (e->>'settlementDate')::date<day OR (e->>'settlementDate')::date>(clock_timestamp() AT TIME ZONE 'UTC')::date THEN RAISE EXCEPTION 'Invalid authenticated settlement date'; END IF;
  -- This exact, previously authorized submitted hold remains payable after a subsequent debit freeze.
  SELECT * INTO STRICT account FROM core.ledger_accounts WHERE id=payer;
  IF account.kind<>'CUSTOMER' OR account.state='CLOSED' OR account.held<total OR account.booked<total THEN RAISE EXCEPTION 'Reserved payer funds unavailable'; END IF;
  IF EXISTS(SELECT 1 FROM core.ledger_accounts WHERE (id=clearing OR (id=fees AND fee>0)) AND (state<>'ACTIVE' OR credit_blocked)) THEN RAISE EXCEPTION 'Settlement control account unavailable'; END IF;
  target:=gen_random_uuid();
  INSERT INTO core.journal_batches(id,operation_id,posting_kind,fingerprint,business_date) VALUES(target,p_operation,'EXTERNAL_PAYMENT',encode(sha256(convert_to(jsonb_build_object('requestDigest',p_digest,'evidenceDigest',p_evidence_digest)::text,'UTF8')),'hex'),(e->>'settlementDate')::date);
  UPDATE core.ledger_accounts SET booked=booked-total,held=held-total,version=version+1 WHERE id=payer;
  UPDATE core.holds SET state='CONSUMED',version=version+1,updated_at=now() WHERE operation_id=p_operation;
  UPDATE core.ledger_accounts SET booked=booked+principal,version=version+1 WHERE id=clearing;
  INSERT INTO core.journal_entries VALUES(target,payer,-total),(target,clearing,principal);
  IF fee>0 THEN
   UPDATE core.ledger_accounts SET booked=booked+fee,version=version+1 WHERE id=fees;
   INSERT INTO core.journal_entries VALUES(target,fees,fee);
  END IF;
  INSERT INTO core.payment_executions VALUES(p_operation,payer,clearing,principal,fee,product,day,p_digest,target,now());
  UPDATE core.payment_limit_usage SET state='POSTED' WHERE operation_id=p_operation AND state='ACTIVE';
  final_state:='SETTLED';
 ELSE
  IF e->>'settlementDate' IS NOT NULL THEN RAISE EXCEPTION 'Nonexecution cannot claim settlement'; END IF;
  PERFORM core.release_hold(p_operation,(e->>'evidenceId')::uuid);
  UPDATE core.payment_limit_usage SET state='RELEASED' WHERE operation_id=p_operation AND state='ACTIVE';
  final_state:=CASE WHEN e->>'status'='REJECTED' THEN 'DECLINED' ELSE 'CANCELLED' END;
 END IF;
 UPDATE core.obligations SET state='RESOLVED' WHERE operation_id=p_operation AND state='OPEN';
 INSERT INTO core.external_finalities(operation_id,request_digest,outcome,evidence_id,evidence_digest,document,batch_id) VALUES(p_operation,p_digest,final_state,(e->>'evidenceId')::uuid,p_evidence_digest,p_packet,target);
 RETURN QUERY SELECT final_state,target,true;
END $$;
GRANT EXECUTE ON FUNCTION core.finalize_external(uuid,text,jsonb,text) TO bank_core;

-- Authenticated rail movements use linked evidence-backed corrections; a generic reversal cannot bypass refund caps.
CREATE OR REPLACE FUNCTION core.reverse_batch(p_operation uuid,p_original uuid,p_approval uuid,p_maker uuid,p_checker uuid)
RETURNS TABLE(batch_id uuid,created boolean) LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,core AS $$
DECLARE legs jsonb;
BEGIN
 IF p_maker IS NULL OR p_checker IS NULL OR p_maker=p_checker THEN RAISE EXCEPTION 'Independent correction approval required'; END IF;
 PERFORM pg_advisory_xact_lock(hashtextextended(p_original::text,1));
 IF NOT EXISTS(SELECT 1 FROM core.journal_batches WHERE id=p_original AND posting_kind NOT IN ('CORRECTION','EXTERNAL_PAYMENT','EXTERNAL_RETURN','INBOUND_CREDIT','INBOUND_SUSPENSE','SUSPENSE_RELEASE')) THEN RAISE EXCEPTION 'Original journal batch required'; END IF;
 INSERT INTO core.correction_receipts VALUES(p_approval,p_original,p_maker,p_checker) ON CONFLICT(approval_id) DO NOTHING;
 IF NOT EXISTS(SELECT 1 FROM core.correction_receipts WHERE approval_id=p_approval AND original_batch=p_original AND maker_person=p_maker AND checker_person=p_checker) THEN RAISE EXCEPTION 'Correction approval conflict'; END IF;
 SELECT jsonb_agg(jsonb_build_object('accountId',account_id,'delta',-delta) ORDER BY account_id) INTO legs FROM core.journal_entries WHERE core.journal_entries.batch_id=p_original;
 RETURN QUERY SELECT * FROM core.post(p_operation,'CORRECTION',legs,NULL,p_original);
END $$;
