ALTER TABLE payments.intents DROP CONSTRAINT intents_state_check;
ALTER TABLE payments.intents ADD CONSTRAINT intents_state_check CHECK(state IN ('DRAFT','ACCEPTED','CONTROLS_PENDING','CONTROL_REVIEW','EXPIRED','DECLINED','BLOCKED','SETTLED','RESERVED','SUBMITTING','SUBMITTED','OUTCOME_UNKNOWN','RAIL_PAUSED','RECONCILING','CANCELLED','CANCELLATION_PENDING'));
ALTER TABLE payments.rail_submissions ADD COLUMN cancellations integer NOT NULL DEFAULT 0 CHECK(cancellations>=0);

CREATE TABLE payments.recovery_requests(
 id uuid PRIMARY KEY,payment_id uuid NOT NULL REFERENCES payments.intents(id),
 issuer text NOT NULL,subject text NOT NULL,client_id text NOT NULL,request_key uuid NOT NULL,actor_context jsonb NOT NULL,
 action text NOT NULL CHECK(action IN ('INQUIRE','CANCEL')),operator_reason text NOT NULL,fingerprint text NOT NULL,
 state text NOT NULL DEFAULT 'REQUESTED' CHECK(state IN ('REQUESTED','WAITING','REVIEW_REQUIRED','COMPLETED')),
 reason_code text NOT NULL DEFAULT 'RECOVERY_REQUEST_ACCEPTED',operation_id uuid,version bigint NOT NULL DEFAULT 1,
 created_at timestamptz NOT NULL DEFAULT now(),updated_at timestamptz NOT NULL DEFAULT now(),
 UNIQUE(issuer,subject,client_id,request_key)
);
CREATE UNIQUE INDEX one_active_recovery_action ON payments.recovery_requests(payment_id,action) WHERE state IN ('REQUESTED','WAITING');
CREATE INDEX recovery_work ON payments.recovery_requests(updated_at,id) WHERE state<>'COMPLETED';
CREATE FUNCTION payments.guard_recovery_request() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF TG_OP<>'UPDATE' OR NEW.id<>OLD.id OR NEW.payment_id<>OLD.payment_id OR NEW.issuer<>OLD.issuer OR NEW.subject<>OLD.subject
 OR NEW.client_id<>OLD.client_id OR NEW.request_key<>OLD.request_key OR NEW.actor_context<>OLD.actor_context
 OR NEW.action<>OLD.action OR NEW.operator_reason<>OLD.operator_reason OR NEW.fingerprint<>OLD.fingerprint OR NEW.created_at<>OLD.created_at
 OR (OLD.operation_id IS NOT NULL AND NEW.operation_id IS DISTINCT FROM OLD.operation_id)
 OR (OLD.state='COMPLETED' AND (NEW.state<>OLD.state OR NEW.reason_code<>OLD.reason_code)) THEN RAISE EXCEPTION 'Immutable recovery command identity'; END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER immutable_recovery_request BEFORE UPDATE OR DELETE ON payments.recovery_requests FOR EACH ROW EXECUTE FUNCTION payments.guard_recovery_request();
CREATE TRIGGER immutable_recovery_request_set BEFORE TRUNCATE ON payments.recovery_requests FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal();
CREATE TABLE payments.recovery_history(request_id uuid NOT NULL REFERENCES payments.recovery_requests(id),version bigint NOT NULL,state text NOT NULL,reason_code text NOT NULL,recorded_at timestamptz NOT NULL DEFAULT now(),PRIMARY KEY(request_id,version));
REVOKE ALL ON payments.recovery_history FROM bank_payments;
GRANT SELECT ON payments.recovery_history TO bank_payments;
CREATE TRIGGER immutable_recovery_history BEFORE UPDATE OR DELETE OR TRUNCATE ON payments.recovery_history FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal();
CREATE FUNCTION payments.capture_recovery_history() RETURNS trigger LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,payments AS $$
BEGIN
 IF TG_OP='INSERT' OR NEW.state<>OLD.state OR NEW.reason_code<>OLD.reason_code THEN
  INSERT INTO payments.recovery_history VALUES(NEW.id,NEW.version,NEW.state,NEW.reason_code,now());
 END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER recovery_history AFTER INSERT OR UPDATE ON payments.recovery_requests FOR EACH ROW EXECUTE FUNCTION payments.capture_recovery_history();

CREATE TABLE payments.reconciliation_cursor(id boolean PRIMARY KEY DEFAULT true CHECK(id),
 last_id uuid NOT NULL DEFAULT '00000000-0000-0000-0000-000000000000',
 last_intent_id uuid NOT NULL DEFAULT '00000000-0000-0000-0000-000000000000',
 last_request_id uuid NOT NULL DEFAULT '00000000-0000-0000-0000-000000000000',
 last_credit_id uuid NOT NULL DEFAULT '00000000-0000-0000-0000-000000000000');
INSERT INTO payments.reconciliation_cursor DEFAULT VALUES;
CREATE TABLE payments.reconciliation_breaks(
 id uuid PRIMARY KEY,payment_id uuid NOT NULL,code text NOT NULL,
 state text NOT NULL CHECK(state IN ('OPEN','MATCHED')),case_id uuid,
 observation jsonb NOT NULL,fingerprint text NOT NULL,version bigint NOT NULL DEFAULT 1,
 first_seen_at timestamptz NOT NULL DEFAULT now(),last_seen_at timestamptz NOT NULL DEFAULT now(),matched_at timestamptz,
 UNIQUE(payment_id,code)
);
CREATE TABLE payments.reconciliation_history(break_id uuid NOT NULL REFERENCES payments.reconciliation_breaks(id),version bigint NOT NULL,state text NOT NULL,observation jsonb NOT NULL,recorded_at timestamptz NOT NULL DEFAULT now(),PRIMARY KEY(break_id,version));
REVOKE ALL ON payments.reconciliation_history FROM bank_payments;
GRANT SELECT ON payments.reconciliation_history TO bank_payments;
CREATE TRIGGER immutable_reconciliation_history BEFORE UPDATE OR DELETE OR TRUNCATE ON payments.reconciliation_history FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal();
CREATE FUNCTION payments.capture_reconciliation_history() RETURNS trigger LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,payments AS $$
BEGIN
 IF TG_OP='INSERT' OR NEW.version<>OLD.version THEN
  INSERT INTO payments.reconciliation_history VALUES(NEW.id,NEW.version,NEW.state,NEW.observation,now());
 END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER reconciliation_history AFTER INSERT OR UPDATE ON payments.reconciliation_breaks FOR EACH ROW EXECUTE FUNCTION payments.capture_reconciliation_history();
CREATE FUNCTION payments.guard_reconciliation_break() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF TG_OP<>'UPDATE' OR NEW.id<>OLD.id OR NEW.payment_id<>OLD.payment_id OR NEW.code<>OLD.code OR NEW.first_seen_at<>OLD.first_seen_at
 OR (OLD.case_id IS NOT NULL AND NEW.case_id IS DISTINCT FROM OLD.case_id) OR NEW.version<OLD.version
 THEN RAISE EXCEPTION 'Immutable reconciliation break identity'; END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER immutable_reconciliation_identity BEFORE UPDATE OR DELETE ON payments.reconciliation_breaks FOR EACH ROW EXECUTE FUNCTION payments.guard_reconciliation_break();
CREATE TRIGGER immutable_reconciliation_set BEFORE TRUNCATE ON payments.reconciliation_breaks FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal();
CREATE OR REPLACE FUNCTION payments.guard_rail_work() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF TG_OP<>'UPDATE' OR NEW.payment_id<>OLD.payment_id OR NEW.instruction_id<>OLD.instruction_id OR NEW.document<>OLD.document OR NEW.fingerprint<>OLD.fingerprint OR NEW.created_at<>OLD.created_at
 OR (OLD.attempted_at IS NOT NULL AND NEW.attempted_at IS DISTINCT FROM OLD.attempted_at) OR NEW.attempts<OLD.attempts OR NEW.inquiries<OLD.inquiries OR NEW.cancellations<OLD.cancellations OR NEW.lease_fence<OLD.lease_fence THEN RAISE EXCEPTION 'Immutable provider work identity'; END IF;
 RETURN NEW;
END $$;
