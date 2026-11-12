-- owner: platform. API cache lifetime is independent of durable business identity.
CREATE TABLE platform.operations(
  id uuid PRIMARY KEY,issuer text NOT NULL,subject text NOT NULL,client_id text NOT NULL,
  kind text NOT NULL,idempotency_key text NOT NULL,fingerprint text NOT NULL,
  state text NOT NULL CHECK(state IN ('ACCEPTED','RUNNING','SUCCEEDED','FAILED','REVIEW_REQUIRED')),
  version bigint NOT NULL DEFAULT 1 CHECK(version>0),result jsonb,
  created_at timestamptz NOT NULL DEFAULT now(),updated_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE(issuer,subject,client_id,kind,idempotency_key)
);
CREATE TABLE platform.jobs(
  id uuid PRIMARY KEY,operation_id uuid NOT NULL REFERENCES platform.operations(id),
  kind text NOT NULL,step_key text NOT NULL,payload jsonb NOT NULL,
  state text NOT NULL CHECK(state IN ('READY','RUNNING','DONE','DEAD')),
  available_at timestamptz NOT NULL DEFAULT now(),lease_owner uuid,lease_until timestamptz,
  fence bigint NOT NULL DEFAULT 0,attempts integer NOT NULL DEFAULT 0,max_attempts integer NOT NULL DEFAULT 8,
  last_error_code text,completion_fingerprint text,created_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE(operation_id,step_key),CHECK(max_attempts BETWEEN 1 AND 100)
);
CREATE INDEX jobs_ready ON platform.jobs(available_at) WHERE state IN ('READY','RUNNING');
