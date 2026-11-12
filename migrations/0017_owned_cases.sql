ALTER TABLE controls.check_requests ADD COLUMN customer_issuer text;
ALTER TABLE controls.check_requests ADD COLUMN customer_subject text;
CREATE TABLE controls.case_queue_members(queue text NOT NULL,person_id uuid NOT NULL,capability text NOT NULL CHECK(capability IN ('REVIEW','APPROVE')),revoked_at timestamptz,PRIMARY KEY(queue,person_id,capability));
CREATE TABLE controls.cases(
 id uuid PRIMARY KEY,kind text NOT NULL CHECK(kind IN ('CONTROL','SUPPORT','PAYMENT','DATA','FINANCE')),
 origin_id uuid NOT NULL,resource_id uuid NOT NULL,customer_issuer text,customer_subject text,
 queue text NOT NULL,owner_person uuid,severity text NOT NULL DEFAULT 'HIGH',
 state text NOT NULL DEFAULT 'OPEN' CHECK(state IN ('OPEN','INFORMATION_REQUIRED','PROPOSED','CLOSED')),
 next_action text NOT NULL DEFAULT 'ASSIGN_REVIEWER',evidence_refs jsonb NOT NULL DEFAULT '[]',
 version bigint NOT NULL DEFAULT 1,deadline timestamptz NOT NULL,escalated_at timestamptz,
 created_at timestamptz NOT NULL DEFAULT now(),updated_at timestamptz NOT NULL DEFAULT now(),UNIQUE(kind,origin_id)
);
CREATE TABLE controls.case_actions(id uuid PRIMARY KEY,case_id uuid NOT NULL REFERENCES controls.cases(id),actor_issuer text NOT NULL,actor_subject text NOT NULL,action text NOT NULL,reason_code text NOT NULL,created_at timestamptz NOT NULL DEFAULT now());
CREATE TRIGGER immutable_case_actions BEFORE UPDATE OR DELETE OR TRUNCATE ON controls.case_actions FOR EACH STATEMENT EXECUTE FUNCTION controls.immutable_check_result();
CREATE TABLE controls.case_proposals(id uuid PRIMARY KEY,case_id uuid NOT NULL REFERENCES controls.cases(id),case_version bigint NOT NULL,disposition text NOT NULL,maker_person uuid NOT NULL,checker_person uuid,expires_at timestamptz NOT NULL,state text NOT NULL DEFAULT 'PENDING',CHECK(checker_person IS NULL OR checker_person<>maker_person));
CREATE TABLE controls.check_reviews(request_id uuid PRIMARY KEY REFERENCES controls.check_requests(id),case_id uuid NOT NULL UNIQUE REFERENCES controls.cases(id),result_digest text NOT NULL,maker_person uuid NOT NULL,checker_person uuid NOT NULL,decision text NOT NULL CHECK(decision IN ('ALLOW','DENY')),expires_at timestamptz NOT NULL,CHECK(maker_person<>checker_person));
CREATE TRIGGER immutable_check_reviews BEFORE UPDATE OR DELETE OR TRUNCATE ON controls.check_reviews FOR EACH STATEMENT EXECUTE FUNCTION controls.immutable_check_result();
