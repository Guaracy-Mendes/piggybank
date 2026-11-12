-- Synthetic local identities only. Never use these credentials in a real environment.
CREATE ROLE bank_migrator LOGIN PASSWORD 'synthetic-local-only';
ALTER DATABASE piggybank OWNER TO bank_migrator;
CREATE ROLE keycloak LOGIN PASSWORD 'synthetic-keycloak-only';
CREATE DATABASE keycloak OWNER keycloak;
REVOKE CREATE ON SCHEMA public FROM PUBLIC;
