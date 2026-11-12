-- Immutable incoming/return clearing and separate V1 Finance observations; no money commands.
CREATE TABLE simulator.credit_settlement_files(id uuid PRIMARY KEY,document jsonb NOT NULL,digest text NOT NULL,published_at timestamptz NOT NULL DEFAULT now());
CREATE TRIGGER immutable_credit_settlement_files BEFORE UPDATE OR DELETE OR TRUNCATE ON simulator.credit_settlement_files FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal();
CREATE TABLE finance.credit_reconciliation_runs(
 id uuid PRIMARY KEY,report_id uuid NOT NULL REFERENCES finance.reports(id),file_id uuid NOT NULL,
 issuer text NOT NULL,subject text NOT NULL,client_id text NOT NULL,request_key uuid NOT NULL,request_digest text NOT NULL,
 document jsonb NOT NULL,digest text NOT NULL,provider_packet jsonb NOT NULL,core_snapshot jsonb NOT NULL,payments_snapshot jsonb NOT NULL,
 created_at timestamptz NOT NULL DEFAULT now(),UNIQUE(issuer,subject,client_id,request_key)
);
CREATE TRIGGER immutable_credit_reconciliation_runs BEFORE UPDATE OR DELETE OR TRUNCATE ON finance.credit_reconciliation_runs FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal();
CREATE TABLE finance.credit_reconciliation_items(run_id uuid NOT NULL REFERENCES finance.credit_reconciliation_runs(id),ordinal integer NOT NULL CHECK(ordinal>0),movement_id uuid NOT NULL,document jsonb NOT NULL,digest text NOT NULL,PRIMARY KEY(run_id,ordinal),UNIQUE(run_id,movement_id));
CREATE TRIGGER immutable_credit_reconciliation_items BEFORE UPDATE OR DELETE OR TRUNCATE ON finance.credit_reconciliation_items FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal();
CREATE TABLE finance.credit_reconciliation_issues(id uuid PRIMARY KEY,run_id uuid NOT NULL,ordinal integer NOT NULL,code text NOT NULL,case_id uuid,UNIQUE(run_id,ordinal,code),FOREIGN KEY(run_id,ordinal) REFERENCES finance.credit_reconciliation_items(run_id,ordinal));
CREATE FUNCTION finance.guard_credit_reconciliation_issue() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF TG_OP<>'UPDATE' OR (to_jsonb(NEW)-'case_id')<>(to_jsonb(OLD)-'case_id') OR OLD.case_id IS NOT NULL OR NEW.case_id IS NULL THEN RAISE EXCEPTION 'Immutable credit reconciliation observation'; END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER immutable_credit_reconciliation_issue BEFORE UPDATE OR DELETE ON finance.credit_reconciliation_issues FOR EACH ROW EXECUTE FUNCTION finance.guard_credit_reconciliation_issue();
CREATE TRIGGER immutable_credit_reconciliation_issue_set BEFORE TRUNCATE ON finance.credit_reconciliation_issues FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal();
CREATE TABLE finance.credit_reconciliation_case_cursor(id boolean PRIMARY KEY DEFAULT true CHECK(id),last_issue_id uuid NOT NULL);
INSERT INTO finance.credit_reconciliation_case_cursor VALUES(true,'00000000-0000-0000-0000-000000000000');
