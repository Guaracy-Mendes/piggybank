-- owner: platform / migration registry. Runtime roles have DML, never schema ownership.
REVOKE CREATE ON SCHEMA public FROM PUBLIC;
CREATE SCHEMA platform AUTHORIZATION bank_migrator;
CREATE SCHEMA identity AUTHORIZATION bank_migrator;
CREATE SCHEMA core AUTHORIZATION bank_migrator;
CREATE SCHEMA controls AUTHORIZATION bank_migrator;
CREATE SCHEMA journeys AUTHORIZATION bank_migrator;
CREATE SCHEMA payments AUTHORIZATION bank_migrator;
CREATE SCHEMA finance AUTHORIZATION bank_migrator;
CREATE SCHEMA documents AUTHORIZATION bank_migrator;
CREATE SCHEMA data AUTHORIZATION bank_migrator;
CREATE SCHEMA simulator AUTHORIZATION bank_migrator;
DO $$
DECLARE module_name text;
BEGIN
  FOREACH module_name IN ARRAY ARRAY['platform','identity','core','controls','journeys','payments','finance','documents','data','simulator']
  LOOP
    EXECUTE format('GRANT USAGE ON SCHEMA %I TO %I',module_name,'bank_'||module_name);
    EXECUTE format('ALTER DEFAULT PRIVILEGES FOR ROLE bank_migrator IN SCHEMA %I GRANT SELECT,INSERT,UPDATE,DELETE ON TABLES TO %I',module_name,'bank_'||module_name);
    EXECUTE format('ALTER DEFAULT PRIVILEGES FOR ROLE bank_migrator IN SCHEMA %I GRANT USAGE,SELECT ON SEQUENCES TO %I',module_name,'bank_'||module_name);
    EXECUTE format('ALTER DEFAULT PRIVILEGES FOR ROLE bank_migrator IN SCHEMA %I REVOKE EXECUTE ON FUNCTIONS FROM PUBLIC',module_name);
  END LOOP;
END $$;
