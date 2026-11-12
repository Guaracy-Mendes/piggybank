-- owner: controls. Human identity links are explicit, never email inferred.
CREATE TABLE controls.person_links(issuer text NOT NULL,subject text NOT NULL,person_id uuid NOT NULL,PRIMARY KEY(issuer,subject));
CREATE TABLE controls.role_proposals(
  id uuid PRIMARY KEY,target_person uuid NOT NULL,role text NOT NULL,scope uuid,
  maker_person uuid NOT NULL,checker_person uuid,valid_until timestamptz NOT NULL,
  state text NOT NULL CHECK(state IN ('PENDING','APPROVED','REJECTED')),
  created_at timestamptz NOT NULL DEFAULT now(),CHECK(checker_person IS NULL OR checker_person<>maker_person)
);
CREATE TABLE controls.role_grants(
  id uuid PRIMARY KEY,person_id uuid NOT NULL,role text NOT NULL,scope uuid,
  valid_from timestamptz NOT NULL DEFAULT now(),valid_until timestamptz NOT NULL,revoked_at timestamptz,
  proposal_id uuid UNIQUE REFERENCES controls.role_proposals(id),
  CHECK(valid_until>valid_from)
);
CREATE TABLE controls.approval_challenges(
  id uuid PRIMARY KEY,operation_id uuid NOT NULL UNIQUE,issuer text NOT NULL,subject text NOT NULL,
  person_id uuid NOT NULL,intent_digest text NOT NULL,nonce text NOT NULL,
  state text NOT NULL CHECK(state IN ('PENDING','APPROVED')),
  expires_at timestamptz NOT NULL,created_at timestamptz NOT NULL DEFAULT now()
);
ALTER TABLE identity.login_attempts ADD COLUMN challenge_id uuid;
ALTER TABLE identity.login_attempts ADD COLUMN intent_digest text;
ALTER TABLE identity.login_attempts ADD COLUMN expected_issuer text;
ALTER TABLE identity.login_attempts ADD COLUMN expected_subject text;
CREATE TABLE identity.step_up_proofs(
  id uuid PRIMARY KEY,challenge_id uuid NOT NULL UNIQUE,issuer text NOT NULL,subject text NOT NULL,
  intent_digest text NOT NULL,authenticated_at timestamptz NOT NULL,expires_at timestamptz NOT NULL,
  use_operation_id uuid,
  FOREIGN KEY(issuer,subject) REFERENCES identity.subjects(issuer,subject)
);
