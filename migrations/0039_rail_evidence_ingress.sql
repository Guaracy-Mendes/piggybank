ALTER TABLE payments.intents DROP CONSTRAINT intents_state_check;
ALTER TABLE payments.intents ADD CONSTRAINT intents_state_check CHECK(state IN ('DRAFT','ACCEPTED','CONTROLS_PENDING','CONTROL_REVIEW','EXPIRED','DECLINED','BLOCKED','SETTLED','RESERVED','SUBMITTING','SUBMITTED','OUTCOME_UNKNOWN','RAIL_PAUSED','RECONCILING','CANCELLED'));
CREATE TABLE payments.rail_quarantine(id uuid PRIMARY KEY,payment_id uuid,digest text NOT NULL,reason_code text NOT NULL,document jsonb NOT NULL,recorded_at timestamptz NOT NULL DEFAULT now(),UNIQUE(digest,reason_code));
CREATE TRIGGER immutable_rail_quarantine BEFORE UPDATE OR DELETE OR TRUNCATE ON payments.rail_quarantine FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal();
