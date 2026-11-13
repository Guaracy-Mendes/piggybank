-- Durable Finance audit progress and native Controls receipt confirmation. No financial commands.
CREATE INDEX finance_pending_audit_ids ON finance.audit_pending(id) WHERE delivered_at IS NULL;
CREATE TABLE finance.audit_relay_cursor(
 id boolean PRIMARY KEY DEFAULT true CHECK(id),
 last_id uuid NOT NULL DEFAULT '00000000-0000-0000-0000-000000000000',
 scanned_at timestamptz
);
INSERT INTO finance.audit_relay_cursor DEFAULT VALUES;
CREATE TABLE finance.audit_delivery_receipts(
 audit_id uuid PRIMARY KEY REFERENCES finance.audit_pending(id),
 controls_digest text NOT NULL CHECK(controls_digest~'^[0-9a-f]{64}$'),
 controls_sequence bigint NOT NULL CHECK(controls_sequence>0),
 confirmed_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 origin text NOT NULL CHECK(origin IN ('NATIVE_CONFIRMATION','LEGACY_VERIFIED'))
);
REVOKE ALL ON finance.audit_delivery_receipts FROM bank_finance;
GRANT SELECT ON finance.audit_delivery_receipts TO bank_finance;
CREATE TRIGGER immutable_finance_audit_receipts BEFORE UPDATE OR DELETE OR TRUNCATE ON finance.audit_delivery_receipts FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal();
CREATE TABLE finance.audit_delivery_attempts(
 id uuid PRIMARY KEY,audit_id uuid NOT NULL REFERENCES finance.audit_pending(id),
 source_digest text NOT NULL CHECK(source_digest~'^[0-9a-f]{64}$'),
 outcome text NOT NULL CHECK(outcome IN ('NATIVE_RECEIPT_CONFIRMED','INVALID_RECORD','NON_CANONICAL_RECORD','AUDIT_IDENTITY_CONFLICT','CONFIRMATION_UNAVAILABLE','APPEND_UNCONFIRMED')),
 attempted_at timestamptz NOT NULL DEFAULT clock_timestamp()
);
CREATE INDEX finance_audit_attempt_history ON finance.audit_delivery_attempts(audit_id,attempted_at DESC,id);
REVOKE UPDATE,DELETE,TRUNCATE ON finance.audit_delivery_attempts FROM bank_finance;
CREATE TRIGGER immutable_finance_audit_attempts BEFORE UPDATE OR DELETE OR TRUNCATE ON finance.audit_delivery_attempts FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal();
-- Historical delivery timestamps remain unchanged. Verify original native evidence before backfill.
INSERT INTO finance.audit_delivery_receipts(audit_id,controls_digest,controls_sequence,origin)
 SELECT p.id,e.digest,e.sequence,'LEGACY_VERIFIED' FROM finance.audit_pending p JOIN controls.audit_events e ON e.id=p.id
 WHERE p.delivered_at IS NOT NULL AND e.stream='finance' AND e.payload=p.record
 AND e.digest=encode(sha256(convert_to(e.previous_digest||E'\n'||e.sequence::text||E'\n'||e.payload::text,'UTF8')),'hex');
DO $$ BEGIN
 IF EXISTS(SELECT 1 FROM finance.audit_pending p LEFT JOIN finance.audit_delivery_receipts r ON r.audit_id=p.id WHERE p.delivered_at IS NOT NULL AND r.audit_id IS NULL) THEN
  RAISE EXCEPTION 'Finance audit backfill requires original matching Controls evidence';
 END IF;
END $$;
CREATE OR REPLACE FUNCTION finance.guard_audit_pending() RETURNS trigger LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,finance AS $$
BEGIN
 IF TG_OP='INSERT' THEN
  IF NEW.delivered_at IS NOT NULL THEN RAISE EXCEPTION 'Native audit receipt required before delivery'; END IF;
  RETURN NEW;
 END IF;
 IF TG_OP<>'UPDATE' OR NEW.id<>OLD.id OR NEW.record<>OLD.record OR OLD.delivered_at IS NOT NULL OR NEW.delivered_at IS NULL THEN RAISE EXCEPTION 'Immutable finance audit'; END IF;
 IF NOT EXISTS(SELECT 1 FROM finance.audit_delivery_receipts r JOIN controls.audit_events e ON e.id=r.audit_id
  WHERE r.audit_id=OLD.id AND e.stream='finance' AND e.payload=OLD.record AND e.digest=r.controls_digest AND e.sequence=r.controls_sequence
  AND e.digest=encode(sha256(convert_to(e.previous_digest||E'\n'||e.sequence::text||E'\n'||e.payload::text,'UTF8')),'hex')) THEN
  RAISE EXCEPTION 'Original native Controls receipt required for Finance audit acknowledgment';
 END IF;
 RETURN NEW;
END $$;
CREATE FUNCTION finance.audit_delivery_receipt(p_id uuid) RETURNS jsonb LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,finance AS $$
DECLARE p finance.audit_pending; r finance.audit_delivery_receipts;
BEGIN
 SELECT * INTO STRICT p FROM finance.audit_pending WHERE id=p_id;
 SELECT * INTO r FROM finance.audit_delivery_receipts WHERE audit_id=p_id;
 IF NOT FOUND THEN
  IF p.delivered_at IS NOT NULL THEN RAISE EXCEPTION 'Finance audit acknowledgment lacks native receipt'; END IF;
  RETURN NULL;
 END IF;
 IF p.delivered_at IS NULL OR NOT EXISTS(SELECT 1 FROM controls.audit_events e WHERE e.id=p_id AND e.stream='finance' AND e.payload=p.record AND e.digest=r.controls_digest AND e.sequence=r.controls_sequence
  AND e.digest=encode(sha256(convert_to(e.previous_digest||E'\n'||e.sequence::text||E'\n'||e.payload::text,'UTF8')),'hex')) THEN RAISE EXCEPTION 'Finance audit native receipt integrity conflict'; END IF;
 RETURN jsonb_build_object('auditId',r.audit_id,'controlsDigest',r.controls_digest,'controlsSequence',r.controls_sequence::text,'confirmedAt',r.confirmed_at,'origin',r.origin);
END $$;
CREATE FUNCTION finance.confirm_audit_delivery(p_id uuid) RETURNS boolean LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,finance AS $$
DECLARE p finance.audit_pending; e controls.audit_events;
BEGIN
 SELECT * INTO STRICT p FROM finance.audit_pending WHERE id=p_id FOR UPDATE;
 IF finance.audit_delivery_receipt(p_id) IS NOT NULL THEN RETURN true; END IF;
 SELECT * INTO e FROM controls.audit_events WHERE id=p_id;
 IF NOT FOUND THEN RETURN false; END IF;
 IF e.stream<>'finance' OR e.payload<>p.record OR e.digest<>encode(sha256(convert_to(e.previous_digest||E'\n'||e.sequence::text||E'\n'||e.payload::text,'UTF8')),'hex') THEN RAISE EXCEPTION 'Finance audit identity conflict'; END IF;
 INSERT INTO finance.audit_delivery_receipts(audit_id,controls_digest,controls_sequence,origin) VALUES(p_id,e.digest,e.sequence,'NATIVE_CONFIRMATION');
 UPDATE finance.audit_pending SET delivered_at=clock_timestamp() WHERE id=p_id;
 RETURN true;
END $$;
GRANT EXECUTE ON FUNCTION finance.confirm_audit_delivery(uuid),finance.audit_delivery_receipt(uuid) TO bank_finance;

CREATE TRIGGER guard_new_finance_audit BEFORE INSERT ON finance.audit_pending FOR EACH ROW EXECUTE FUNCTION finance.guard_audit_pending();
