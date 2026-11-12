-- owner: platform / shared migration security.
-- Function EXECUTE is globally granted to PUBLIC by PostgreSQL by default;
-- a per-schema revoke alone does not remove that global default.
ALTER DEFAULT PRIVILEGES FOR ROLE bank_migrator REVOKE EXECUTE ON FUNCTIONS FROM PUBLIC;
