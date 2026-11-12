-- Exact-source financial correction workflow. Native command accepts SYNTHETIC_FUNDING only.
ALTER TABLE controls.change_proposals DROP CONSTRAINT change_proposals_kind_check;
ALTER TABLE controls.change_proposals ADD CONSTRAINT change_proposals_kind_check CHECK(kind IN ('PRODUCT','TERMS','CORRECTION','MASTER_REFERENCE','RAIL_POLICY','FINANCE_CHART','FINANCE_REPORT','PERIODIC_POLICY','FINANCE_CORRECTION'));
CREATE TABLE core.finance_correction_receipts(
 execution_id uuid PRIMARY KEY,approval_id uuid NOT NULL UNIQUE,original_batch_id uuid NOT NULL UNIQUE REFERENCES core.journal_batches(id),batch_id uuid NOT NULL UNIQUE REFERENCES core.journal_batches(id),
 details jsonb NOT NULL,details_digest text NOT NULL CHECK(details_digest~'^[0-9a-f]{64}$'),maker_person uuid NOT NULL,checker_person uuid NOT NULL,
 recorded_at timestamptz NOT NULL DEFAULT clock_timestamp(),CHECK(maker_person<>checker_person)
);
REVOKE ALL ON core.finance_correction_receipts FROM bank_core;
GRANT SELECT ON core.finance_correction_receipts TO bank_core;
CREATE TRIGGER immutable_finance_correction_receipts BEFORE UPDATE OR DELETE OR TRUNCATE ON core.finance_correction_receipts FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal();
CREATE FUNCTION core.execute_finance_correction(p_details jsonb,p_digest text,p_approval uuid,p_maker uuid,p_checker uuid)
RETURNS TABLE(batch_id uuid,created boolean) LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,core AS $$
DECLARE execution uuid:=(p_details->>'executionId')::uuid; original uuid:=(p_details->>'originalBatchId')::uuid;
 source core.journal_batches; saved core.finance_correction_receipts; legs jsonb; target uuid; made boolean;
BEGIN
 PERFORM pg_advisory_xact_lock(hashtextextended(original::text,1));
 IF NOT EXISTS(SELECT 1 FROM controls.role_grants WHERE person_id=p_maker AND role='finance_adjuster' AND scope IS NULL AND revoked_at IS NULL AND valid_from<=clock_timestamp() AND valid_until>clock_timestamp()) OR NOT EXISTS(SELECT 1 FROM controls.case_queue_members WHERE person_id=p_maker AND queue='finance-operations' AND capability='REVIEW' AND revoked_at IS NULL) THEN RAISE EXCEPTION 'Current Finance maker required'; END IF;
 SELECT * INTO saved FROM core.finance_correction_receipts WHERE execution_id=execution;
 IF FOUND THEN
  IF saved.details<>p_details OR saved.details_digest<>p_digest OR saved.approval_id<>p_approval OR saved.maker_person<>p_maker OR saved.checker_person<>p_checker THEN RAISE EXCEPTION 'Correction receipt identity conflict'; END IF;
  RETURN QUERY SELECT saved.batch_id,false;RETURN;
 END IF;
 IF EXISTS(SELECT 1 FROM core.journal_batches WHERE original_batch=original AND posting_kind='CORRECTION') THEN RAISE EXCEPTION 'Source already corrected'; END IF;
 SELECT * INTO STRICT source FROM core.journal_batches WHERE id=original;
 IF source.posting_kind<>'SYNTHETIC_FUNDING' OR source.original_batch IS NOT NULL OR source.operation_id IS DISTINCT FROM (p_details->>'sourceOperationId')::uuid OR source.business_date IS DISTINCT FROM (p_details->>'sourceBusinessDate')::date OR source.fingerprint IS DISTINCT FROM p_details->>'sourceFingerprint' THEN RAISE EXCEPTION 'Exact synthetic funding source required'; END IF;
 PERFORM 1 FROM core.ledger_accounts a WHERE id IN(SELECT account_id FROM core.journal_entries WHERE core.journal_entries.batch_id=original) ORDER BY id FOR UPDATE;
 SELECT jsonb_agg(jsonb_build_object('accountId',e.account_id,'accountKind',a.kind,'originalMinor',e.delta::text,'reversingMinor',(-e.delta::numeric)::text) ORDER BY e.account_id) INTO legs FROM core.journal_entries e JOIN core.ledger_accounts a ON a.id=e.account_id WHERE e.batch_id=original;
 IF jsonb_array_length(legs) IS DISTINCT FROM 2 OR legs IS DISTINCT FROM p_details->'legs' OR (SELECT count(*) FROM core.journal_entries e JOIN core.ledger_accounts a ON a.id=e.account_id WHERE e.batch_id=original AND a.kind='CUSTOMER' AND e.delta>0)<>1 OR (SELECT count(*) FROM core.journal_entries e JOIN core.ledger_accounts a ON a.id=e.account_id WHERE e.batch_id=original AND a.kind='SETTLEMENT' AND e.delta<0)<>1 THEN RAISE EXCEPTION 'Original funding legs required'; END IF;
 IF p_details->>'kind' IS DISTINCT FROM 'FULL_SYNTHETIC_FUNDING_REVERSAL_V1' OR p_details->>'label' IS DISTINCT FROM 'SYNTHETIC' OR (p_details->>'formatVersion')::integer IS DISTINCT FROM 1 OR execution IS NULL OR execution='00000000-0000-0000-0000-000000000000'::uuid OR execution=original OR execution IS DISTINCT FROM p_approval OR p_details->'currency' IS DISTINCT FROM jsonb_build_object('code','TST','scale',2) OR length(p_details->>'reason') NOT BETWEEN 4 AND 240 OR encode(sha256(convert_to(core.periodic_policy_canonical(p_details),'UTF8')),'hex')<>p_digest THEN RAISE EXCEPTION 'Exact correction command digest required'; END IF;
 -- Both original-source and all financial account locks are held before checking live authority.
 IF p_maker=p_checker OR NOT EXISTS(SELECT 1 FROM controls.change_proposals WHERE id=p_approval AND kind='FINANCE_CORRECTION' AND resource_id=original AND digest=p_digest AND maker_person=p_maker AND checker_person=p_checker AND state='APPROVED' AND expires_at>clock_timestamp()) OR NOT EXISTS(SELECT 1 FROM controls.role_grants WHERE person_id=p_maker AND role='finance_adjuster' AND scope IS NULL AND revoked_at IS NULL AND valid_from<=clock_timestamp() AND valid_until>clock_timestamp()) OR NOT EXISTS(SELECT 1 FROM controls.role_grants WHERE person_id=p_checker AND role='finance_approver' AND scope IS NULL AND revoked_at IS NULL AND valid_from<=clock_timestamp() AND valid_until>clock_timestamp()) OR NOT EXISTS(SELECT 1 FROM controls.case_queue_members WHERE person_id=p_maker AND queue='finance-operations' AND capability='REVIEW' AND revoked_at IS NULL) OR NOT EXISTS(SELECT 1 FROM controls.case_queue_members WHERE person_id=p_checker AND queue='finance-operations' AND capability='APPROVE' AND revoked_at IS NULL) THEN RAISE EXCEPTION 'Current independent exact correction approval required'; END IF;
 IF (now() AT TIME ZONE 'UTC')::date<>(clock_timestamp() AT TIME ZONE 'UTC')::date THEN RAISE EXCEPTION 'Retry original correction in current UTC date'; END IF;
 SELECT r.batch_id,r.created INTO target,made FROM core.reverse_batch(execution,original,p_approval,p_maker,p_checker) r;
 IF NOT made THEN RAISE EXCEPTION 'Unrecorded existing correction requires investigation'; END IF;
 IF (now() AT TIME ZONE 'UTC')::date<>(clock_timestamp() AT TIME ZONE 'UTC')::date THEN RAISE EXCEPTION 'Retry original correction in current UTC date'; END IF;
 INSERT INTO core.finance_correction_receipts(execution_id,approval_id,original_batch_id,batch_id,details,details_digest,maker_person,checker_person) VALUES(execution,p_approval,original,target,p_details,p_digest,p_maker,p_checker);
 RETURN QUERY SELECT target,true;
END $$;
GRANT EXECUTE ON FUNCTION core.execute_finance_correction(jsonb,text,uuid,uuid,uuid) TO bank_core;
CREATE TABLE finance.correction_requests(
 id uuid PRIMARY KEY,original_batch_id uuid NOT NULL,issuer text NOT NULL,subject text NOT NULL,client_id text NOT NULL,request_key uuid NOT NULL,
 actor_context jsonb NOT NULL,maker_person uuid NOT NULL,source jsonb NOT NULL,details jsonb NOT NULL,details_digest text NOT NULL,
 expires_at timestamptz NOT NULL,created_at timestamptz NOT NULL DEFAULT clock_timestamp(),registered_at timestamptz,
 registration_attempts integer NOT NULL DEFAULT 0,registration_reason text NOT NULL DEFAULT 'REGISTRATION_PENDING',UNIQUE(issuer,subject,client_id,request_key)
);
CREATE TRIGGER immutable_finance_correction_request BEFORE UPDATE OR DELETE ON finance.correction_requests FOR EACH ROW EXECUTE FUNCTION finance.guard_periodic_policy_request();
CREATE TRIGGER immutable_finance_correction_request_set BEFORE TRUNCATE ON finance.correction_requests FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal();
CREATE TABLE finance.correction_attempts(id uuid PRIMARY KEY,request_id uuid NOT NULL REFERENCES finance.correction_requests(id),actor_context jsonb NOT NULL,observed_reason text NOT NULL,created_at timestamptz NOT NULL DEFAULT clock_timestamp());
CREATE TRIGGER immutable_finance_correction_attempts BEFORE UPDATE OR DELETE OR TRUNCATE ON finance.correction_attempts FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal();
CREATE TABLE finance.correction_cursor(id boolean PRIMARY KEY DEFAULT true CHECK(id),last_id uuid NOT NULL DEFAULT '00000000-0000-0000-0000-000000000000');
INSERT INTO finance.correction_cursor DEFAULT VALUES;
