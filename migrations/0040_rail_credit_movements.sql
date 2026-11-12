-- Incoming and returned principal is first accounted to suspense; allocation is a separate guarded credit.
INSERT INTO core.ledger_accounts(id,kind,state,debit_blocked) VALUES('4f1a780b-3e31-48ef-a294-000000000006','CONTROL','ACTIVE',false);
CREATE TABLE core.credit_movements(
 movement_id uuid PRIMARY KEY,evidence_id uuid NOT NULL UNIQUE,kind text NOT NULL CHECK(kind IN ('INBOUND_SETTLED','RETURN_SETTLED')),
 original_payment_id uuid REFERENCES core.payment_executions(operation_id),target_account_id uuid REFERENCES core.ledger_accounts(id),receiving_reference text,
 principal bigint NOT NULL CHECK(principal>0),business_date date NOT NULL,request_digest text NOT NULL,document jsonb NOT NULL,
 batch_id uuid NOT NULL UNIQUE REFERENCES core.journal_batches(id),recorded_at timestamptz NOT NULL DEFAULT now(),
 CHECK((kind='RETURN_SETTLED')=(original_payment_id IS NOT NULL)),CHECK((kind='INBOUND_SETTLED')=(receiving_reference IS NOT NULL))
);
CREATE INDEX linked_return_principal ON core.credit_movements(original_payment_id) WHERE kind='RETURN_SETTLED';
CREATE TABLE core.credit_allocations(movement_id uuid PRIMARY KEY REFERENCES core.credit_movements(movement_id),request_digest text NOT NULL,batch_id uuid NOT NULL UNIQUE REFERENCES core.journal_batches(id),allocated_at timestamptz NOT NULL DEFAULT now());
CREATE TRIGGER immutable_credit_movements BEFORE UPDATE OR DELETE OR TRUNCATE ON core.credit_movements FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal();
CREATE TRIGGER immutable_credit_allocations BEFORE UPDATE OR DELETE OR TRUNCATE ON core.credit_allocations FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal();
REVOKE ALL ON core.credit_movements,core.credit_allocations FROM bank_core;
GRANT SELECT ON core.credit_movements,core.credit_allocations TO bank_core;
CREATE FUNCTION core.record_rail_credit(p_packet jsonb,p_digest text) RETURNS TABLE(batch_id uuid,created boolean)
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,core AS $$
DECLARE e jsonb:=p_packet->'body'; movement uuid:=(e->>'movementId')::uuid; evidence uuid:=(e->>'evidenceId')::uuid;
DECLARE kind text:=e->>'eventType'; principal bigint:=(e->'amount'->>'minorUnits')::bigint; original uuid:=(e->>'originalPaymentId')::uuid;
DECLARE destination uuid; source_batch uuid; original_principal bigint; used numeric; prior core.credit_movements; posted record;
DECLARE clearing uuid:='4f1a780b-3e31-48ef-a294-000000000003'; suspense uuid:='4f1a780b-3e31-48ef-a294-000000000006';
BEGIN
 PERFORM pg_advisory_xact_lock(hashtextextended(movement::text,0));
 SELECT * INTO prior FROM core.credit_movements WHERE movement_id=movement;
 IF FOUND THEN
  IF prior.request_digest<>p_digest OR prior.evidence_id<>evidence THEN RAISE EXCEPTION 'Credit movement identity conflict'; END IF;
  RETURN QUERY SELECT prior.batch_id,false; RETURN;
 END IF;
 IF (e->>'provider') IS DISTINCT FROM 'SYNTHETIC_RAIL' OR (e->>'rail') IS DISTINCT FROM 'SIMULATED' OR principal IS NULL OR principal<=0 OR coalesce((e->>'version')::integer,0)<>1
 OR (e->'amount'->'currency'->>'code') IS DISTINCT FROM 'TST' OR (e->'amount'->'currency'->>'scale') IS DISTINCT FROM '2'
 OR kind IS NULL OR kind NOT IN ('INBOUND_SETTLED','RETURN_SETTLED') OR (e->>'businessDate')::date>(clock_timestamp() AT TIME ZONE 'UTC')::date THEN RAISE EXCEPTION 'Valid final credit evidence required'; END IF;
 IF kind='RETURN_SETTLED' THEN
  IF original IS NULL OR original=movement OR e->>'receivingReference' IS NOT NULL THEN RAISE EXCEPTION 'Exact original payment required'; END IF;
  PERFORM pg_advisory_xact_lock(hashtextextended(original::text,151));
  SELECT x.payer_account,x.batch_id,x.principal INTO STRICT destination,source_batch,original_principal FROM core.payment_executions x JOIN core.external_finalities f USING(operation_id) WHERE x.operation_id=original AND f.outcome='SETTLED';
  IF (e->>'businessDate')::date<(SELECT business_date FROM core.journal_batches WHERE id=source_batch) THEN RAISE EXCEPTION 'Return predates original settlement'; END IF;
  SELECT coalesce(sum(c.principal::numeric),0) INTO used FROM core.credit_movements c WHERE c.original_payment_id=original;
  IF used+principal::numeric>original_principal::numeric THEN RAISE EXCEPTION 'Return principal cap exceeded'; END IF;
 ELSE
  IF original IS NOT NULL OR e->>'receivingReference' IS NULL THEN RAISE EXCEPTION 'Explicit receiving reference required'; END IF;
  SELECT account_id INTO destination FROM core.receiving_references WHERE reference=e->>'receivingReference';
 END IF;
 IF EXISTS(SELECT 1 FROM core.payment_limit_usage WHERE operation_id=movement) THEN RAISE EXCEPTION 'Movement identity already used by an outgoing payment'; END IF;
 SELECT * INTO STRICT posted FROM core.post(movement,CASE WHEN kind='RETURN_SETTLED' THEN 'RETURN_SUSPENSE' ELSE 'INBOUND_SUSPENSE' END,jsonb_build_array(jsonb_build_object('accountId',clearing,'delta',-principal),jsonb_build_object('accountId',suspense,'delta',principal)),NULL,source_batch);
 INSERT INTO core.credit_movements VALUES(movement,evidence,kind,original,destination,e->>'receivingReference',principal,(e->>'businessDate')::date,p_digest,p_packet,posted.batch_id,now());
 IF destination IS NOT NULL THEN INSERT INTO core.obligations VALUES(movement,destination,'INCOMING_CREDIT','OPEN'); END IF;
 RETURN QUERY SELECT posted.batch_id,posted.created;
END $$;
CREATE FUNCTION core.allocate_rail_credit(p_movement uuid,p_digest text,p_valid_until timestamptz) RETURNS TABLE(batch_id uuid,created boolean)
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,core AS $$
DECLARE c core.credit_movements; prior core.credit_allocations; posted record; suspense uuid:='4f1a780b-3e31-48ef-a294-000000000006';
BEGIN
 PERFORM pg_advisory_xact_lock(hashtextextended(p_movement::text,0));
 SELECT * INTO STRICT c FROM core.credit_movements WHERE movement_id=p_movement AND request_digest=p_digest;
 SELECT * INTO prior FROM core.credit_allocations WHERE movement_id=p_movement;
 IF FOUND THEN
  IF prior.request_digest<>p_digest THEN RAISE EXCEPTION 'Credit allocation identity conflict'; END IF;
  RETURN QUERY SELECT prior.batch_id,false; RETURN;
 END IF;
 PERFORM 1 FROM core.ledger_accounts WHERE id IN(suspense,c.target_account_id) ORDER BY id FOR UPDATE;
 IF p_valid_until IS NULL OR p_valid_until<=clock_timestamp() OR c.target_account_id IS NULL
 OR NOT EXISTS(SELECT 1 FROM core.ledger_accounts WHERE id=c.target_account_id AND kind='CUSTOMER' AND state='ACTIVE' AND NOT credit_blocked) THEN RAISE EXCEPTION 'Current permitted credit destination and controls required'; END IF;
 SELECT * INTO STRICT posted FROM core.post(p_movement,'CREDIT_ALLOCATION',jsonb_build_array(jsonb_build_object('accountId',suspense,'delta',-c.principal),jsonb_build_object('accountId',c.target_account_id,'delta',c.principal)),NULL,c.batch_id);
 INSERT INTO core.credit_allocations VALUES(p_movement,p_digest,posted.batch_id,now());
 UPDATE core.obligations SET state='RESOLVED' WHERE operation_id=p_movement AND state='OPEN';
 RETURN QUERY SELECT posted.batch_id,posted.created;
END $$;
GRANT EXECUTE ON FUNCTION core.record_rail_credit(jsonb,text),core.allocate_rail_credit(uuid,text,timestamptz) TO bank_core;
CREATE TABLE simulator.rail_credits(movement_id uuid PRIMARY KEY,evidence_id uuid NOT NULL UNIQUE,fingerprint text NOT NULL,document jsonb NOT NULL);
CREATE TRIGGER immutable_simulated_credits BEFORE UPDATE OR DELETE OR TRUNCATE ON simulator.rail_credits FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal();
CREATE TABLE payments.credit_intents(
 movement_id uuid PRIMARY KEY,evidence_id uuid NOT NULL UNIQUE,document jsonb NOT NULL,fingerprint text NOT NULL,
 customer_issuer text,customer_subject text,target_account_id uuid,operation_id uuid,
 state text NOT NULL DEFAULT 'RECEIVED' CHECK(state IN ('RECEIVED','CORE_SUSPENSE','CONTROLS_PENDING','CONTROL_REVIEW','DESTINATION_RESTRICTED','CREDITED','QUARANTINED')),
 reason_code text NOT NULL DEFAULT 'AUTHENTICATED_RECEIPT',version bigint NOT NULL DEFAULT 1,created_at timestamptz NOT NULL DEFAULT now(),updated_at timestamptz NOT NULL DEFAULT now()
);
CREATE FUNCTION payments.guard_credit_intent() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF TG_OP<>'UPDATE' OR NEW.movement_id<>OLD.movement_id OR NEW.evidence_id<>OLD.evidence_id OR NEW.document<>OLD.document OR NEW.fingerprint<>OLD.fingerprint OR NEW.created_at<>OLD.created_at
 OR NEW.customer_issuer IS DISTINCT FROM OLD.customer_issuer OR NEW.customer_subject IS DISTINCT FROM OLD.customer_subject OR NEW.target_account_id IS DISTINCT FROM OLD.target_account_id
 THEN RAISE EXCEPTION 'Immutable credit receipt identity'; END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER immutable_credit_intent_identity BEFORE UPDATE OR DELETE ON payments.credit_intents FOR EACH ROW EXECUTE FUNCTION payments.guard_credit_intent();
CREATE TRIGGER immutable_credit_intent_set BEFORE TRUNCATE ON payments.credit_intents FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal();

-- A provider evidence ID has one financial meaning across outgoing finality and incoming/return credit.
CREATE TABLE core.rail_evidence_keys(evidence_id uuid PRIMARY KEY,kind text NOT NULL,operation_id uuid NOT NULL,digest text NOT NULL);
INSERT INTO core.rail_evidence_keys SELECT evidence_id,'OUTGOING_FINALITY',operation_id,evidence_digest FROM core.external_finalities;
REVOKE ALL ON core.rail_evidence_keys FROM bank_core;
GRANT SELECT ON core.rail_evidence_keys TO bank_core;
CREATE TRIGGER immutable_financial_evidence_keys BEFORE UPDATE OR DELETE OR TRUNCATE ON core.rail_evidence_keys FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal();
CREATE FUNCTION core.claim_rail_evidence() RETURNS trigger LANGUAGE plpgsql SET search_path=pg_catalog,core AS $$
DECLARE op uuid; claimed_digest text; claimed_kind text;
BEGIN
 IF TG_TABLE_NAME='external_finalities' THEN op:=NEW.operation_id;claimed_digest:=NEW.evidence_digest;claimed_kind:='OUTGOING_FINALITY';
 ELSE op:=NEW.movement_id;claimed_digest:=NEW.request_digest;claimed_kind:='CREDIT_MOVEMENT'; END IF;
 INSERT INTO core.rail_evidence_keys VALUES(NEW.evidence_id,claimed_kind,op,claimed_digest) ON CONFLICT DO NOTHING;
 IF NOT EXISTS(SELECT 1 FROM core.rail_evidence_keys k WHERE k.evidence_id=NEW.evidence_id AND k.kind=claimed_kind AND k.operation_id=op AND k.digest=claimed_digest) THEN RAISE EXCEPTION 'Provider evidence identity reused across financial meanings'; END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER claim_outgoing_evidence BEFORE INSERT ON core.external_finalities FOR EACH ROW EXECUTE FUNCTION core.claim_rail_evidence();
CREATE TRIGGER claim_credit_evidence BEFORE INSERT ON core.credit_movements FOR EACH ROW EXECUTE FUNCTION core.claim_rail_evidence();

CREATE OR REPLACE FUNCTION core.reverse_batch(p_operation uuid,p_original uuid,p_approval uuid,p_maker uuid,p_checker uuid)
RETURNS TABLE(batch_id uuid,created boolean) LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,core AS $$
DECLARE legs jsonb;
BEGIN
 IF p_maker IS NULL OR p_checker IS NULL OR p_maker=p_checker THEN RAISE EXCEPTION 'Independent correction approval required'; END IF;
 PERFORM pg_advisory_xact_lock(hashtextextended(p_original::text,1));
 IF NOT EXISTS(SELECT 1 FROM core.journal_batches WHERE id=p_original AND posting_kind NOT IN ('CORRECTION','EXTERNAL_PAYMENT','EXTERNAL_RETURN','INBOUND_CREDIT','INBOUND_SUSPENSE','SUSPENSE_RELEASE','RETURN_SUSPENSE','CREDIT_ALLOCATION')) THEN RAISE EXCEPTION 'Original journal batch required'; END IF;
 INSERT INTO core.correction_receipts VALUES(p_approval,p_original,p_maker,p_checker) ON CONFLICT(approval_id) DO NOTHING;
 IF NOT EXISTS(SELECT 1 FROM core.correction_receipts WHERE approval_id=p_approval AND original_batch=p_original AND maker_person=p_maker AND checker_person=p_checker) THEN RAISE EXCEPTION 'Correction approval conflict'; END IF;
 SELECT jsonb_agg(jsonb_build_object('accountId',account_id,'delta',-delta) ORDER BY account_id) INTO legs FROM core.journal_entries WHERE core.journal_entries.batch_id=p_original;
 RETURN QUERY SELECT * FROM core.post(p_operation,'CORRECTION',legs,NULL,p_original);
END $$;
