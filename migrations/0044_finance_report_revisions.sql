ALTER TABLE controls.change_proposals DROP CONSTRAINT change_proposals_kind_check;
ALTER TABLE controls.change_proposals ADD CONSTRAINT change_proposals_kind_check CHECK(kind IN ('PRODUCT','TERMS','CORRECTION','MASTER_REFERENCE','RAIL_POLICY','FINANCE_CHART','FINANCE_REPORT'));
CREATE TABLE finance.report_revision_requests(
 id uuid PRIMARY KEY,original_report_id uuid NOT NULL REFERENCES finance.reports(id),replacement_report_id uuid NOT NULL REFERENCES finance.reports(id),
 issuer text NOT NULL,subject text NOT NULL,client_id text NOT NULL,request_key uuid NOT NULL,actor_context jsonb NOT NULL,maker_person uuid NOT NULL,
 details jsonb NOT NULL,details_digest text NOT NULL,expires_at timestamptz NOT NULL,created_at timestamptz NOT NULL DEFAULT now(),
 registered_at timestamptz,registration_attempts integer NOT NULL DEFAULT 0,registration_reason text NOT NULL DEFAULT 'REGISTRATION_PENDING',
 UNIQUE(issuer,subject,client_id,request_key),CHECK(original_report_id<>replacement_report_id)
);
CREATE FUNCTION finance.guard_report_revision_request() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF TG_OP<>'UPDATE' OR (to_jsonb(NEW)-ARRAY['registered_at','registration_attempts','registration_reason'])<>(to_jsonb(OLD)-ARRAY['registered_at','registration_attempts','registration_reason'])
 OR (OLD.registered_at IS NOT NULL AND NEW.registered_at IS DISTINCT FROM OLD.registered_at) OR NEW.registration_attempts<OLD.registration_attempts THEN RAISE EXCEPTION 'Immutable report revision command'; END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER immutable_report_revision_request BEFORE UPDATE OR DELETE ON finance.report_revision_requests FOR EACH ROW EXECUTE FUNCTION finance.guard_report_revision_request();
CREATE TRIGGER immutable_report_revision_request_set BEFORE TRUNCATE ON finance.report_revision_requests FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal();
CREATE TABLE finance.report_revisions(
 proposal_id uuid PRIMARY KEY REFERENCES finance.report_revision_requests(id),original_report_id uuid NOT NULL UNIQUE REFERENCES finance.reports(id),replacement_report_id uuid NOT NULL UNIQUE REFERENCES finance.reports(id),
 details_digest text NOT NULL,maker_person uuid NOT NULL,checker_person uuid NOT NULL,published_at timestamptz NOT NULL DEFAULT now(),
 CHECK(original_report_id<>replacement_report_id),CHECK(maker_person<>checker_person)
);
CREATE TRIGGER immutable_published_report_revision BEFORE UPDATE OR DELETE OR TRUNCATE ON finance.report_revisions FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal();
CREATE TABLE finance.revision_cursor(id boolean PRIMARY KEY DEFAULT true CHECK(id),last_id uuid NOT NULL DEFAULT '00000000-0000-0000-0000-000000000000');
INSERT INTO finance.revision_cursor DEFAULT VALUES;
