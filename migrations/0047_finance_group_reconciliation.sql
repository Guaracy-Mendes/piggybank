-- Closed provider membership and immutable grouped observations; no financial command authority.
CREATE TABLE simulator.settlement_sets(id uuid PRIMARY KEY,document jsonb NOT NULL,digest text NOT NULL,published_at timestamptz NOT NULL DEFAULT now());
CREATE TRIGGER immutable_settlement_sets BEFORE UPDATE OR DELETE OR TRUNCATE ON simulator.settlement_sets FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal();
CREATE TABLE finance.group_reconciliation_runs(
 id uuid PRIMARY KEY,report_id uuid NOT NULL REFERENCES finance.reports(id),set_id uuid NOT NULL,
 issuer text NOT NULL,subject text NOT NULL,client_id text NOT NULL,request_key uuid NOT NULL,request_digest text NOT NULL,
 document jsonb NOT NULL,digest text NOT NULL,manifest_packet jsonb NOT NULL,file_packets jsonb NOT NULL,core_snapshot jsonb NOT NULL,payments_snapshot jsonb NOT NULL,
 created_at timestamptz NOT NULL DEFAULT now(),UNIQUE(issuer,subject,client_id,request_key)
);
CREATE TRIGGER immutable_group_reconciliation_runs BEFORE UPDATE OR DELETE OR TRUNCATE ON finance.group_reconciliation_runs FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal();
CREATE TABLE finance.group_reconciliation_items(run_id uuid NOT NULL REFERENCES finance.group_reconciliation_runs(id),ordinal integer NOT NULL CHECK(ordinal>0),operation_id uuid NOT NULL,document jsonb NOT NULL,digest text NOT NULL,PRIMARY KEY(run_id,ordinal),UNIQUE(run_id,operation_id));
CREATE TRIGGER immutable_group_reconciliation_items BEFORE UPDATE OR DELETE OR TRUNCATE ON finance.group_reconciliation_items FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal();
CREATE TABLE finance.group_reconciliation_issues(id uuid PRIMARY KEY,run_id uuid NOT NULL,ordinal integer NOT NULL,code text NOT NULL,case_id uuid,UNIQUE(run_id,ordinal,code),FOREIGN KEY(run_id,ordinal) REFERENCES finance.group_reconciliation_items(run_id,ordinal));
CREATE TRIGGER immutable_group_reconciliation_issue BEFORE UPDATE OR DELETE ON finance.group_reconciliation_issues FOR EACH ROW EXECUTE FUNCTION finance.guard_credit_reconciliation_issue();
CREATE TRIGGER immutable_group_reconciliation_issue_set BEFORE TRUNCATE ON finance.group_reconciliation_issues FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal();
CREATE TABLE finance.group_reconciliation_case_cursor(id boolean PRIMARY KEY DEFAULT true CHECK(id),last_issue_id uuid NOT NULL);
INSERT INTO finance.group_reconciliation_case_cursor VALUES(true,'00000000-0000-0000-0000-000000000000');
