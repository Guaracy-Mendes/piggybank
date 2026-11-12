ALTER TABLE controls.change_proposals DROP CONSTRAINT change_proposals_kind_check;
ALTER TABLE controls.change_proposals ADD CONSTRAINT change_proposals_kind_check CHECK(kind IN ('PRODUCT','TERMS','CORRECTION','MASTER_REFERENCE','RAIL_POLICY'));
ALTER TABLE payments.beneficiaries ADD COLUMN rail text NOT NULL DEFAULT 'INTERNAL' CHECK(rail IN ('INTERNAL','SIMULATED'));
CREATE OR REPLACE FUNCTION payments.guard_beneficiary() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF TG_OP<>'UPDATE' OR NEW.id<>OLD.id OR NEW.issuer<>OLD.issuer OR NEW.subject<>OLD.subject OR NEW.request_key<>OLD.request_key
 OR NEW.label<>OLD.label OR NEW.receiving_reference<>OLD.receiving_reference OR NEW.payee_account_id<>OLD.payee_account_id OR NEW.rail<>OLD.rail
 OR NEW.fingerprint<>OLD.fingerprint OR NEW.verified_at<>OLD.verified_at OR OLD.state<>'VERIFIED' OR NEW.state<>'REVOKED'
 OR NEW.version<>OLD.version+1 THEN RAISE EXCEPTION 'Immutable verified beneficiary'; END IF;
 RETURN NEW;
END $$;
ALTER TABLE payments.intents DROP CONSTRAINT intents_state_check;
ALTER TABLE payments.intents ADD CONSTRAINT intents_state_check CHECK(state IN ('DRAFT','ACCEPTED','CONTROLS_PENDING','CONTROL_REVIEW','EXPIRED','DECLINED','BLOCKED','SETTLED','RESERVED','SUBMITTING','SUBMITTED','OUTCOME_UNKNOWN','RAIL_PAUSED','RECONCILING'));
CREATE TABLE payments.rail_policy_head(rail text PRIMARY KEY CHECK(rail='SIMULATED'),paused boolean NOT NULL DEFAULT true,revision bigint NOT NULL DEFAULT 1);
INSERT INTO payments.rail_policy_head VALUES('SIMULATED',true,1);
CREATE TABLE payments.rail_policies(id uuid PRIMARY KEY,rail text NOT NULL CHECK(rail='SIMULATED'),version integer NOT NULL CHECK(version>0),document jsonb NOT NULL,approval_id uuid NOT NULL UNIQUE,maker_person uuid NOT NULL,checker_person uuid NOT NULL,published_at timestamptz NOT NULL DEFAULT now(),UNIQUE(rail,version),CHECK(maker_person<>checker_person));
CREATE TRIGGER immutable_rail_policies BEFORE UPDATE OR DELETE OR TRUNCATE ON payments.rail_policies FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal();
CREATE TABLE payments.rail_changes(proposal_id uuid PRIMARY KEY,kind text NOT NULL CHECK(kind IN ('POLICY','RESUME')),resource_id uuid NOT NULL,details jsonb NOT NULL,maker_person uuid NOT NULL,acted_at timestamptz,created_at timestamptz NOT NULL DEFAULT now());
CREATE FUNCTION payments.guard_rail_change() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF TG_OP<>'UPDATE' OR NEW.proposal_id<>OLD.proposal_id OR NEW.kind<>OLD.kind OR NEW.resource_id<>OLD.resource_id OR NEW.details<>OLD.details OR NEW.maker_person<>OLD.maker_person OR NEW.created_at<>OLD.created_at OR OLD.acted_at IS NOT NULL OR NEW.acted_at IS NULL THEN RAISE EXCEPTION 'Immutable rail change'; END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER immutable_rail_change BEFORE UPDATE OR DELETE ON payments.rail_changes FOR EACH ROW EXECUTE FUNCTION payments.guard_rail_change();
CREATE TRIGGER immutable_rail_change_set BEFORE TRUNCATE ON payments.rail_changes FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal();
CREATE TABLE payments.rail_submissions(
 payment_id uuid PRIMARY KEY REFERENCES payments.intents(id),instruction_id uuid NOT NULL UNIQUE,document jsonb NOT NULL,fingerprint text NOT NULL,
 attempted_at timestamptz,last_status text,attempts integer NOT NULL DEFAULT 0,inquiries integer NOT NULL DEFAULT 0,
 lease_id uuid,lease_until timestamptz,lease_fence bigint NOT NULL DEFAULT 0,created_at timestamptz NOT NULL DEFAULT now()
);
CREATE FUNCTION payments.guard_rail_work() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF TG_OP<>'UPDATE' OR NEW.payment_id<>OLD.payment_id OR NEW.instruction_id<>OLD.instruction_id OR NEW.document<>OLD.document OR NEW.fingerprint<>OLD.fingerprint OR NEW.created_at<>OLD.created_at OR (OLD.attempted_at IS NOT NULL AND NEW.attempted_at IS DISTINCT FROM OLD.attempted_at) OR NEW.attempts<OLD.attempts OR NEW.inquiries<OLD.inquiries OR NEW.lease_fence<OLD.lease_fence THEN RAISE EXCEPTION 'Immutable provider work identity'; END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER immutable_rail_work BEFORE UPDATE OR DELETE ON payments.rail_submissions FOR EACH ROW EXECUTE FUNCTION payments.guard_rail_work();
CREATE TRIGGER immutable_rail_work_set BEFORE TRUNCATE ON payments.rail_submissions FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal();
CREATE TABLE payments.rail_receipts(evidence_id uuid PRIMARY KEY,payment_id uuid NOT NULL REFERENCES payments.intents(id),provider_version bigint NOT NULL,digest text NOT NULL,document jsonb NOT NULL,recorded_at timestamptz NOT NULL DEFAULT now(),UNIQUE(payment_id,provider_version));
CREATE TRIGGER immutable_bank_rail_receipts BEFORE UPDATE OR DELETE OR TRUNCATE ON payments.rail_receipts FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal();
CREATE TABLE payments.rail_rejections(id uuid PRIMARY KEY,payment_id uuid NOT NULL,digest text NOT NULL,reason_code text NOT NULL,recorded_at timestamptz NOT NULL DEFAULT now(),UNIQUE(payment_id,digest,reason_code));
CREATE TRIGGER immutable_bank_rail_rejections BEFORE UPDATE OR DELETE OR TRUNCATE ON payments.rail_rejections FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal();
