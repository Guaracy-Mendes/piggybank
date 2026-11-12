-- A one-use server-held navigation target lets browser JS complete issuer logout without reading tokens.
CREATE TABLE identity.logout_navigation(ticket_hash text PRIMARY KEY,trust text NOT NULL,location text NOT NULL,expires_at timestamptz NOT NULL DEFAULT now()+interval '2 minutes');
CREATE INDEX logout_navigation_expiry ON identity.logout_navigation(expires_at);
