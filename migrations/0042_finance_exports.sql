ALTER TABLE controls.change_proposals DROP CONSTRAINT change_proposals_kind_check;
ALTER TABLE controls.change_proposals ADD CONSTRAINT change_proposals_kind_check CHECK(kind IN ('PRODUCT','TERMS','CORRECTION','MASTER_REFERENCE','RAIL_POLICY','FINANCE_CHART'));
CREATE TABLE finance.chart_versions(
 id uuid PRIMARY KEY,chart_code text NOT NULL,version integer NOT NULL CHECK(version>0),document jsonb NOT NULL,
 approval_id uuid NOT NULL UNIQUE,maker_person uuid NOT NULL,checker_person uuid NOT NULL,
 published_at timestamptz NOT NULL DEFAULT now(),UNIQUE(chart_code,version),CHECK(maker_person<>checker_person)
);
CREATE TRIGGER immutable_finance_charts BEFORE UPDATE OR DELETE OR TRUNCATE ON finance.chart_versions FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal();
CREATE TABLE finance.gl_exports(
 id uuid PRIMARY KEY,source_batch_id uuid NOT NULL UNIQUE,chart_id uuid NOT NULL REFERENCES finance.chart_versions(id),
 document jsonb NOT NULL,fingerprint text NOT NULL,operation_id uuid,
 state text NOT NULL DEFAULT 'READY' CHECK(state IN ('READY','OUTCOME_UNKNOWN','BOOKED','REVIEW_REQUIRED')),
 retry_generation bigint NOT NULL DEFAULT 0,attempt_limit integer NOT NULL DEFAULT 8 CHECK(attempt_limit>=8),
 reason_code text NOT NULL DEFAULT 'EXPORT_STAGED',attempted_at timestamptz,attempts integer NOT NULL DEFAULT 0,
 lease_id uuid,lease_until timestamptz,lease_fence bigint NOT NULL DEFAULT 0,
 created_at timestamptz NOT NULL DEFAULT now(),updated_at timestamptz NOT NULL DEFAULT now()
);
CREATE FUNCTION finance.guard_gl_export() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF TG_OP<>'UPDATE' OR NEW.id<>OLD.id OR NEW.source_batch_id<>OLD.source_batch_id OR NEW.chart_id<>OLD.chart_id OR NEW.document<>OLD.document OR NEW.fingerprint<>OLD.fingerprint OR NEW.created_at<>OLD.created_at
 OR (OLD.operation_id IS NOT NULL AND NEW.operation_id IS DISTINCT FROM OLD.operation_id) OR (OLD.attempted_at IS NOT NULL AND NEW.attempted_at IS DISTINCT FROM OLD.attempted_at)
 OR NEW.attempts<OLD.attempts OR NEW.attempt_limit<OLD.attempt_limit OR NEW.retry_generation<OLD.retry_generation OR NEW.lease_fence<OLD.lease_fence OR (OLD.state='BOOKED' AND NEW.state<>'BOOKED') THEN RAISE EXCEPTION 'Immutable GL export identity'; END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER immutable_gl_export BEFORE UPDATE OR DELETE ON finance.gl_exports FOR EACH ROW EXECUTE FUNCTION finance.guard_gl_export();
CREATE TRIGGER immutable_gl_export_set BEFORE TRUNCATE ON finance.gl_exports FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal();
CREATE TABLE finance.gl_receipts(export_id uuid PRIMARY KEY REFERENCES finance.gl_exports(id),evidence_id uuid NOT NULL UNIQUE,digest text NOT NULL,document jsonb NOT NULL,recorded_at timestamptz NOT NULL DEFAULT now());
CREATE TRIGGER immutable_gl_receipts BEFORE UPDATE OR DELETE OR TRUNCATE ON finance.gl_receipts FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal();
CREATE TABLE simulator.gl_batches(source_batch_id uuid PRIMARY KEY,export_id uuid NOT NULL UNIQUE,fingerprint text NOT NULL,document jsonb NOT NULL,receipt jsonb NOT NULL,booked_at timestamptz NOT NULL DEFAULT now());
CREATE TRIGGER immutable_simulated_gl BEFORE UPDATE OR DELETE OR TRUNCATE ON simulator.gl_batches FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal();
CREATE TABLE simulator.gl_scenarios(source_batch_id uuid PRIMARY KEY,mode text NOT NULL CHECK(mode IN ('ACCEPT','LOST_RESPONSE','BEFORE_ACCEPT_TIMEOUT','CONFLICT')));
CREATE TABLE finance.audit_pending(id uuid PRIMARY KEY,record jsonb NOT NULL,delivered_at timestamptz);
CREATE FUNCTION finance.guard_audit_pending() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF TG_OP<>'UPDATE' OR NEW.id<>OLD.id OR NEW.record<>OLD.record OR OLD.delivered_at IS NOT NULL OR NEW.delivered_at IS NULL THEN RAISE EXCEPTION 'Immutable finance audit'; END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER immutable_finance_audit BEFORE UPDATE OR DELETE ON finance.audit_pending FOR EACH ROW EXECUTE FUNCTION finance.guard_audit_pending();
CREATE TRIGGER immutable_finance_audit_set BEFORE TRUNCATE ON finance.audit_pending FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal();
CREATE TABLE finance.export_cursor(id boolean PRIMARY KEY DEFAULT true CHECK(id),last_source_id uuid NOT NULL DEFAULT '00000000-0000-0000-0000-000000000000',last_export_id uuid NOT NULL DEFAULT '00000000-0000-0000-0000-000000000000');
INSERT INTO finance.export_cursor DEFAULT VALUES;
CREATE TABLE finance.gl_observations(id uuid PRIMARY KEY,export_id uuid NOT NULL REFERENCES finance.gl_exports(id),lease_fence bigint NOT NULL,kind text NOT NULL,document jsonb NOT NULL,digest text NOT NULL,recorded_at timestamptz NOT NULL DEFAULT now(),UNIQUE(export_id,digest));
CREATE TRIGGER immutable_gl_observations BEFORE UPDATE OR DELETE OR TRUNCATE ON finance.gl_observations FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal();
CREATE TABLE finance.gl_history(export_id uuid NOT NULL REFERENCES finance.gl_exports(id),ordinal bigint GENERATED ALWAYS AS IDENTITY,state text NOT NULL,reason_code text NOT NULL,attempts integer NOT NULL,lease_fence bigint NOT NULL,recorded_at timestamptz NOT NULL DEFAULT now(),PRIMARY KEY(export_id,ordinal));
REVOKE ALL ON finance.gl_history FROM bank_finance;
GRANT SELECT ON finance.gl_history TO bank_finance;
CREATE TRIGGER immutable_gl_history BEFORE UPDATE OR DELETE OR TRUNCATE ON finance.gl_history FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal();
CREATE FUNCTION finance.capture_gl_history() RETURNS trigger LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,finance AS $$
BEGIN
 IF TG_OP='INSERT' OR NEW.state<>OLD.state OR NEW.reason_code<>OLD.reason_code OR NEW.attempts<>OLD.attempts THEN
  INSERT INTO finance.gl_history(export_id,state,reason_code,attempts,lease_fence) VALUES(NEW.id,NEW.state,NEW.reason_code,NEW.attempts,NEW.lease_fence);
 END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER gl_history AFTER INSERT OR UPDATE ON finance.gl_exports FOR EACH ROW EXECUTE FUNCTION finance.capture_gl_history();
CREATE TABLE finance.export_breaks(id uuid PRIMARY KEY,source_batch_id uuid NOT NULL,code text NOT NULL,state text NOT NULL CHECK(state IN ('OPEN','MATCHED')),case_id uuid,first_seen_at timestamptz NOT NULL DEFAULT now(),last_seen_at timestamptz NOT NULL DEFAULT now(),matched_at timestamptz,UNIQUE(source_batch_id,code));
CREATE FUNCTION finance.guard_export_break() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF TG_OP<>'UPDATE' OR NEW.id<>OLD.id OR NEW.source_batch_id<>OLD.source_batch_id OR NEW.code<>OLD.code OR NEW.first_seen_at<>OLD.first_seen_at OR (OLD.case_id IS NOT NULL AND NEW.case_id IS DISTINCT FROM OLD.case_id) THEN RAISE EXCEPTION 'Immutable finance break identity'; END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER immutable_export_break BEFORE UPDATE OR DELETE ON finance.export_breaks FOR EACH ROW EXECUTE FUNCTION finance.guard_export_break();
CREATE TRIGGER immutable_export_break_set BEFORE TRUNCATE ON finance.export_breaks FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal();
CREATE TABLE finance.retry_commands(id uuid PRIMARY KEY,export_id uuid NOT NULL REFERENCES finance.gl_exports(id),generation bigint NOT NULL,issuer text NOT NULL,subject text NOT NULL,client_id text NOT NULL,request_key uuid NOT NULL,reason text NOT NULL,fingerprint text NOT NULL,operation_id uuid,created_at timestamptz NOT NULL DEFAULT now(),UNIQUE(export_id,generation),UNIQUE(issuer,subject,client_id,request_key));
CREATE FUNCTION finance.guard_retry_command() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF TG_OP<>'UPDATE' OR (to_jsonb(NEW)-'operation_id')<>(to_jsonb(OLD)-'operation_id') OR OLD.operation_id IS NOT NULL OR NEW.operation_id IS NULL THEN RAISE EXCEPTION 'Immutable finance recovery command'; END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER immutable_retry_command BEFORE UPDATE OR DELETE ON finance.retry_commands FOR EACH ROW EXECUTE FUNCTION finance.guard_retry_command();
CREATE TRIGGER immutable_retry_command_set BEFORE TRUNCATE ON finance.retry_commands FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal();
