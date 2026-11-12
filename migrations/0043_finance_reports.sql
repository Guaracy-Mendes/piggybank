CREATE TABLE finance.reports(
 id uuid PRIMARY KEY,issuer text NOT NULL,subject text NOT NULL,client_id text NOT NULL,request_key uuid NOT NULL,
 request_digest text NOT NULL,document jsonb NOT NULL,digest text NOT NULL,created_at timestamptz NOT NULL DEFAULT now(),
 UNIQUE(issuer,subject,client_id,request_key)
);
CREATE TRIGGER immutable_finance_reports BEFORE UPDATE OR DELETE OR TRUNCATE ON finance.reports FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal();
CREATE TABLE finance.report_items(
 report_id uuid NOT NULL REFERENCES finance.reports(id),ordinal integer NOT NULL CHECK(ordinal>0),source_batch_id uuid NOT NULL,
 document jsonb NOT NULL,digest text NOT NULL,gl_packet jsonb,PRIMARY KEY(report_id,ordinal),UNIQUE(report_id,source_batch_id)
);
CREATE TRIGGER immutable_finance_report_items BEFORE UPDATE OR DELETE OR TRUNCATE ON finance.report_items FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal();
CREATE TABLE finance.report_issues(
 id uuid PRIMARY KEY,report_id uuid NOT NULL REFERENCES finance.reports(id),ordinal integer,code text NOT NULL,resource_id uuid NOT NULL,case_id uuid,
 UNIQUE(report_id,ordinal,code),FOREIGN KEY(report_id,ordinal) REFERENCES finance.report_items(report_id,ordinal)
);
CREATE FUNCTION finance.guard_report_issue() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF TG_OP<>'UPDATE' OR (to_jsonb(NEW)-'case_id')<>(to_jsonb(OLD)-'case_id') OR OLD.case_id IS NOT NULL OR NEW.case_id IS NULL THEN RAISE EXCEPTION 'Immutable report observation'; END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER immutable_report_issue BEFORE UPDATE OR DELETE ON finance.report_issues FOR EACH ROW EXECUTE FUNCTION finance.guard_report_issue();
CREATE TRIGGER immutable_report_issue_set BEFORE TRUNCATE ON finance.report_issues FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal();
CREATE TABLE finance.report_liquidity(report_id uuid PRIMARY KEY REFERENCES finance.reports(id),document jsonb,observation jsonb NOT NULL);
CREATE TRIGGER immutable_report_liquidity BEFORE UPDATE OR DELETE OR TRUNCATE ON finance.report_liquidity FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal();
CREATE TABLE simulator.liquidity_positions(evidence_id uuid PRIMARY KEY,version bigint NOT NULL UNIQUE CHECK(version>0),document jsonb NOT NULL,digest text NOT NULL,published_at timestamptz NOT NULL DEFAULT now());
CREATE TRIGGER immutable_simulated_liquidity BEFORE UPDATE OR DELETE OR TRUNCATE ON simulator.liquidity_positions FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal();
