-- owner: identity. OIDC provider data is in its separate database.
CREATE TABLE identity.login_attempts (
  state_hash text PRIMARY KEY,
  trust text NOT NULL CHECK(trust IN ('customer','workforce')),
  verifier text NOT NULL,
  nonce text NOT NULL,
  expires_at timestamptz NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now()
);
CREATE TABLE identity.subjects (
  issuer text NOT NULL,
  subject text NOT NULL,
  trust text NOT NULL CHECK(trust IN ('customer','workforce','service','partner')),
  created_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY(issuer,subject)
);
CREATE TABLE identity.sessions (
  id_hash text PRIMARY KEY,
  issuer text NOT NULL,
  subject text NOT NULL,
  trust text NOT NULL CHECK(trust IN ('customer','workforce')),
  access_token text NOT NULL,
  refresh_token text NOT NULL,
  id_token text NOT NULL,
  csrf_hash text NOT NULL,
  expires_at timestamptz NOT NULL,
  refresh_lease uuid,
  refresh_until timestamptz,
  created_at timestamptz NOT NULL DEFAULT now(),
  FOREIGN KEY(issuer,subject) REFERENCES identity.subjects(issuer,subject)
);
CREATE INDEX sessions_subject ON identity.sessions(issuer,subject);
CREATE TABLE identity.audit (
  id uuid PRIMARY KEY,
  issuer text NOT NULL,
  subject text NOT NULL,
  action text NOT NULL,
  occurred_at timestamptz NOT NULL DEFAULT now()
);
REVOKE UPDATE,DELETE,TRUNCATE ON identity.audit FROM bank_identity;
