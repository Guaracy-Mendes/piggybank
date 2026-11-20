-- owner: identity. WP-27 native action-bound reauthentication. A native PKCE client obtains a fresh ID token
-- carrying the server-issued challenge nonce; the API verifies it and records the same step-up proof as the BFF.
CREATE TABLE identity.native_step_ups(
 challenge_id uuid PRIMARY KEY,issuer text NOT NULL,subject text NOT NULL,client_id text NOT NULL CHECK(client_id='bank-mobile'),
 nonce text NOT NULL,intent_digest text NOT NULL,created_at timestamptz NOT NULL DEFAULT clock_timestamp(),expires_at timestamptz NOT NULL,
 CHECK(expires_at>created_at AND expires_at<=created_at+interval '3 minutes')
);
