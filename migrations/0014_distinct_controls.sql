CREATE TABLE controls.check_requests(
 id uuid PRIMARY KEY,resource_id uuid NOT NULL,kind text NOT NULL CHECK(kind IN ('IDENTITY','SANCTIONS','RISK','FRAUD')),
 input_digest text NOT NULL,policy_version text NOT NULL,generation integer NOT NULL,
 state text NOT NULL DEFAULT 'PENDING',revision bigint NOT NULL DEFAULT 1,
 created_at timestamptz NOT NULL DEFAULT now(),deadline timestamptz NOT NULL DEFAULT now()+interval '3 minutes',
 UNIQUE(resource_id,kind,generation)
);
CREATE TABLE controls.check_heads(resource_id uuid NOT NULL,kind text NOT NULL,request_id uuid NOT NULL REFERENCES controls.check_requests(id),PRIMARY KEY(resource_id,kind));
CREATE TABLE controls.check_results(request_id uuid PRIMARY KEY REFERENCES controls.check_requests(id),fingerprint text NOT NULL,document jsonb NOT NULL,outcome text NOT NULL CHECK(outcome IN ('ALLOW','DENY','REVIEW')),evidence_id uuid NOT NULL,expires_at timestamptz NOT NULL,recorded_at timestamptz NOT NULL DEFAULT now());
CREATE FUNCTION controls.immutable_check_result() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'Immutable control evidence'; END $$;
CREATE TRIGGER immutable_check_results BEFORE UPDATE OR DELETE OR TRUNCATE ON controls.check_results FOR EACH STATEMENT EXECUTE FUNCTION controls.immutable_check_result();
-- Trigger function is shared immutable behavior owned by migration role, never runtime execution.
CREATE TABLE controls.check_rejections(id uuid PRIMARY KEY DEFAULT gen_random_uuid(),request_id uuid NOT NULL,reason_code text NOT NULL,digest text NOT NULL,created_at timestamptz NOT NULL DEFAULT now(),UNIQUE(request_id,reason_code,digest));
CREATE TABLE simulator.checks(id uuid PRIMARY KEY,request_fingerprint text NOT NULL,request jsonb NOT NULL,document jsonb NOT NULL,created_at timestamptz NOT NULL DEFAULT now());
CREATE TABLE simulator.scenarios(request_id uuid PRIMARY KEY,mode text NOT NULL CHECK(mode IN ('ALLOW','DENY','REVIEW','PENDING','TIMEOUT','LOST_RESPONSE','LATE','CONFLICT')));

CREATE TABLE controls.check_attempts(id uuid PRIMARY KEY,request_id uuid NOT NULL REFERENCES controls.check_requests(id),outcome text NOT NULL,created_at timestamptz NOT NULL DEFAULT now());
