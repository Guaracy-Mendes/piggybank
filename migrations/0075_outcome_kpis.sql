-- owner: data (analytics schema). WP-41, local capacity template (user decision 2026-11-19).
-- KPI-01..08 with explicit numerators, denominators, exclusions and segments, computed from local synthetic records
-- and stored as immutable digested snapshots. Every snapshot is labelled local synthetic: production outcomes, ratified
-- targets and SLAs need DEC-07 and DEC-12. Reads only; no function here writes a record outside analytics.

CREATE TABLE analytics.kpi_snapshots(id uuid PRIMARY KEY,definitions_version text NOT NULL,window_from timestamptz NOT NULL,window_to timestamptz NOT NULL,
 document jsonb NOT NULL,digest text NOT NULL,requested_by jsonb NOT NULL,requested_person uuid NOT NULL,created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 CHECK(window_from<window_to));
CREATE TRIGGER immutable_kpi_snapshots BEFORE UPDATE OR DELETE OR TRUNCATE ON analytics.kpi_snapshots FOR EACH STATEMENT EXECUTE FUNCTION data.immutable_history();
REVOKE ALL ON analytics.kpi_snapshots FROM bank_data;

CREATE FUNCTION analytics.kpi_ratio(p_num bigint,p_den bigint) RETURNS text LANGUAGE sql IMMUTABLE SET search_path=pg_catalog AS
$$ SELECT CASE WHEN p_den>0 THEN round(p_num::numeric/p_den,4)::text END $$;
CREATE FUNCTION analytics.kpi_entry(p_code text,p_definition text,p_num bigint,p_den bigint,p_target numeric,p_extra jsonb) RETURNS jsonb LANGUAGE sql IMMUTABLE SET search_path=pg_catalog AS $$
 SELECT jsonb_build_object('code',p_code,'definition',p_definition,'numerator',p_num,'denominator',p_den,'value',analytics.kpi_ratio(p_num,p_den),
  'target',p_target::text,'meetsTarget',CASE WHEN p_den>0 THEN p_num::numeric/p_den>=p_target END,'status',CASE WHEN p_den>0 THEN 'MEASURED_LOCAL' ELSE 'NO_DENOMINATOR' END)||p_extra $$;

-- The window is [from, to). Immature items (whose deadline has not passed by the computation time) are excluded and counted.
CREATE FUNCTION analytics.kpi_compute(p_from timestamptz,p_to timestamptz,p_at timestamptz) RETURNS jsonb LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE k1 jsonb;k2 jsonb;k3 jsonb;k4 jsonb;k5 jsonb;k6 jsonb;k7 jsonb;k8 jsonb;sla interval:=interval '300 seconds';BEGIN
 -- KPI-01 / KPI-02: onboarding, by first submission in the window.
 WITH sub AS (SELECT application_id,min(created_at) AS submitted_at FROM journeys.submissions GROUP BY application_id
   HAVING min(created_at)>=p_from AND min(created_at)<p_to),
 h AS (SELECT s.application_id,s.submitted_at,
   (SELECT min(x.occurred_at) FROM journeys.history x WHERE x.application_id=s.application_id AND x.state IN ('ACTIVE','DECLINED') AND x.occurred_at<=p_at) AS decided_at,
   EXISTS(SELECT 1 FROM journeys.history x WHERE x.application_id=s.application_id AND x.state IN ('MANUAL_REVIEW','INFORMATION_REQUIRED') AND x.occurred_at<=p_at) AS manual,
   EXISTS(SELECT 1 FROM journeys.history x WHERE x.application_id=s.application_id AND x.state='ACTIVE' AND x.occurred_at<=p_at) AS approved,
   EXISTS(SELECT 1 FROM journeys.history x WHERE x.application_id=s.application_id AND x.state='EXPIRED' AND x.occurred_at<=p_at) AS abandoned FROM sub s),
 m AS (SELECT * FROM h WHERE decided_at IS NOT NULL OR submitted_at+interval '24 hours'<=p_at)
 SELECT analytics.kpi_entry('KPI-01','Submitted applications decided (approved or declined) within 24 hours of first submission, divided by submitted applications',
   (SELECT count(*) FROM m WHERE decided_at<=submitted_at+interval '24 hours'),(SELECT count(*) FROM m),0.90,
   jsonb_build_object('exclusions',jsonb_build_object('immatureUnder24h',(SELECT count(*) FROM h)-(SELECT count(*) FROM m)),
    'breakdown',jsonb_build_object('manualReview',(SELECT count(*) FROM m WHERE manual),'abandoned',(SELECT count(*) FROM m WHERE abandoned),
     'undecidedAfter24h',(SELECT count(*) FROM m WHERE decided_at IS NULL)))),
  analytics.kpi_entry('KPI-02','Approved applications with no manual review or information request, divided by approved applications',
   (SELECT count(*) FROM h WHERE approved AND NOT manual),(SELECT count(*) FROM h WHERE approved),0.80,
   jsonb_build_object('segments',jsonb_build_object('riskTier','NOT_RECORDED_LOCALLY')))
 INTO k1,k2;
 -- KPI-03: approved opening requests (registry rows) activated without remediation.
 SELECT analytics.kpi_entry('KPI-03','Approved opening requests activated without remediation (no opening revision), divided by approved opening requests',
   count(*) FILTER (WHERE a.state='ACTIVE' AND NOT EXISTS(SELECT 1 FROM core.opening_revisions o WHERE o.account_id=r.id)),count(*),0.99,
   jsonb_build_object('breakdown',jsonb_build_object('remediated',count(*) FILTER (WHERE EXISTS(SELECT 1 FROM core.opening_revisions o WHERE o.account_id=r.id)),
    'notActive',count(*) FILTER (WHERE a.state<>'ACTIVE'))))
  INTO k3 FROM core.account_registry r JOIN core.ledger_accounts a ON a.id=r.id WHERE r.created_at>=p_from AND r.created_at<p_to;
 -- KPI-04: confirmed payment intents; customer, control and funding outcomes are excluded; success = settled within the SLA.
 WITH c AS (SELECT i.id,i.state,i.reason_code,i.confirmed_at,i.updated_at,
   EXISTS(SELECT 1 FROM payments.rail_receipts x WHERE x.payment_id=i.id AND x.document->'body'->>'status'='REJECTED') AS provider_rejected
  FROM payments.intents i WHERE i.confirmed_at>=p_from AND i.confirmed_at<p_to),
 e AS (SELECT * FROM c WHERE state IN ('CANCELLED','EXPIRED','BLOCKED') OR (state='DECLINED' AND NOT provider_rejected)),
 v AS (SELECT * FROM c WHERE id NOT IN (SELECT id FROM e) AND (state='SETTLED' OR state='DECLINED' OR confirmed_at+sla<=p_at))
 SELECT analytics.kpi_entry('KPI-04','Valid, funded, approved payment instructions settled within the SLA, divided by such instructions',
   (SELECT count(*) FROM v WHERE state='SETTLED' AND updated_at<=confirmed_at+sla),(SELECT count(*) FROM v),0.995,
   jsonb_build_object('slaSeconds',300,'slaBasis','LOCAL_ASSUMED_PENDING_DEC-04',
    'exclusions',jsonb_build_object('customerControlOrFunding',(SELECT count(*) FROM e),'immatureWithinSla',(SELECT count(*) FROM c)-(SELECT count(*) FROM e)-(SELECT count(*) FROM v)),
    'breakdown',jsonb_build_object('providerFailures',(SELECT count(*) FROM v WHERE state='DECLINED' AND provider_rejected),
     'bankFailures',(SELECT count(*) FROM v WHERE NOT (state='SETTLED' AND updated_at<=confirmed_at+sla) AND NOT (state='DECLINED' AND provider_rejected)))))
 INTO k4;
 -- KPI-05: unbalanced committed batches and duplicate economic executions (a count; target zero).
 WITH b AS (SELECT id,operation_id,posting_kind FROM core.journal_batches WHERE created_at>=p_from AND created_at<p_to),
 u AS (SELECT b.id FROM b JOIN core.journal_entries e ON e.batch_id=b.id GROUP BY b.id HAVING sum(e.delta::numeric)<>0),
 d AS (SELECT operation_id FROM core.journal_batches WHERE posting_kind IN ('INTERNAL_PAYMENT','EXTERNAL_PAYMENT','ATM_WITHDRAWAL','BATCH_CREDIT','LIQUIDITY_PREFUNDING')
   AND operation_id IN (SELECT operation_id FROM b) GROUP BY operation_id HAVING count(*)>1)
 SELECT jsonb_build_object('code','KPI-05','definition','Duplicate economic executions or unbalanced committed journal batches caused by the bank (count)',
   'value',((SELECT count(*) FROM u)+(SELECT count(*) FROM d))::text,'target','0','meetsTarget',(SELECT count(*) FROM u)+(SELECT count(*) FROM d)=0,
   'status','MEASURED_LOCAL','denominator',(SELECT count(*) FROM b),'breakdown',jsonb_build_object('unbalanced',(SELECT count(*) FROM u),'duplicateExecutions',(SELECT count(*) FROM d)))
 INTO k5;
 -- KPI-06: reconciliation issues from runs in the window that have an owned case.
 WITH i AS (SELECT x.case_id FROM finance.reconciliation_issues x JOIN finance.reconciliation_runs r ON r.id=x.run_id WHERE r.created_at>=p_from AND r.created_at<p_to
  UNION ALL SELECT x.case_id FROM finance.credit_reconciliation_issues x JOIN finance.credit_reconciliation_runs r ON r.id=x.run_id WHERE r.created_at>=p_from AND r.created_at<p_to
  UNION ALL SELECT x.case_id FROM finance.group_reconciliation_issues x JOIN finance.group_reconciliation_runs r ON r.id=x.run_id WHERE r.created_at>=p_from AND r.created_at<p_to)
 SELECT analytics.kpi_entry('KPI-06','Reconciliation differences assigned an owned exception case, divided by reconciliation differences',
   count(*) FILTER (WHERE case_id IS NOT NULL),count(*),1,jsonb_build_object('deadline','BUSINESS_DAY_DEADLINE_PENDING_DEC-04'))
  INTO k6 FROM i;
 -- KPI-07: a sampled human review of journeys; not computable from records.
 k7:=jsonb_build_object('code','KPI-07','definition','Sampled journeys showing current state, next action and correct balance semantics','numerator',NULL,'denominator',NULL,
  'value',NULL,'target','1','meetsTarget',NULL,'status','MANUAL_SAMPLE_REQUIRED');
 -- KPI-08: maker/checker decisions with two attributable, distinct people.
 WITH d AS (SELECT maker_person AS maker,checker_person AS checker FROM controls.change_proposals WHERE state='APPROVED' AND created_at>=p_from AND created_at<p_to
  UNION ALL SELECT c.proposed_person,x.decided_person FROM payments.route_policy_changes c JOIN payments.route_policy_decisions x ON x.change_id=c.id WHERE x.decided_at>=p_from AND x.decided_at<p_to
  UNION ALL SELECT p.proposed_person,x.decided_person FROM finance.liquidity_policies p JOIN finance.liquidity_policy_decisions x ON x.version=p.version WHERE x.decided_at>=p_from AND x.decided_at<p_to
  UNION ALL SELECT c.maker_person,x.checker_person FROM finance.liquidity_action_confirmations c JOIN finance.liquidity_action_decisions x ON x.action_id=c.action_id WHERE x.decided_at>=p_from AND x.decided_at<p_to)
 SELECT analytics.kpi_entry('KPI-08','Privileged changes and risk decisions with two attributable, distinct people, divided by such decisions',
   count(*) FILTER (WHERE maker IS NOT NULL AND checker IS NOT NULL AND maker<>checker),count(*),1,
   jsonb_build_object('sources',jsonb_build_array('controls.change_proposals','payments.route_policy_decisions','finance.liquidity_policy_decisions','finance.liquidity_action_decisions')))
  INTO k8 FROM d;
 RETURN jsonb_build_object('definitionsVersion','kpi-v1','windowFrom',analytics.utc(p_from),'windowTo',analytics.utc(p_to),'computedAt',analytics.utc(p_at),
  'evidence','LOCAL_SYNTHETIC_NOT_PRODUCTION_OUTCOMES','targetsRatification','PENDING_DEC-07_DEC-12','kpis',jsonb_build_array(k1,k2,k3,k4,k5,k6,k7,k8));
END $$;
CREATE FUNCTION analytics.kpi_snapshot_document(s analytics.kpi_snapshots) RETURNS jsonb LANGUAGE sql STABLE SET search_path=pg_catalog AS $$
 SELECT jsonb_build_object('id',s.id,'digest',s.digest,'requestedBy',s.requested_by->>'subject','createdAt',analytics.utc(s.created_at))||s.document $$;
CREATE FUNCTION analytics.kpi_snapshot(p_actor jsonb,p_person uuid,p_from timestamptz,p_to timestamptz) RETURNS jsonb LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE d jsonb;s analytics.kpi_snapshots;BEGIN
 PERFORM analytics.require(p_actor,p_person,ARRAY['data_steward','finance_approver','finance_adjuster','product_approver']);
 IF p_from IS NULL OR p_to IS NULL OR p_from>=p_to OR p_to-p_from>interval '93 days' OR p_to>clock_timestamp()+interval '1 day' THEN
  RAISE EXCEPTION 'KPI window must be at most 93 days' USING ERRCODE='22023'; END IF;
 d:=analytics.kpi_compute(p_from,p_to,clock_timestamp());
 INSERT INTO analytics.kpi_snapshots VALUES(gen_random_uuid(),'kpi-v1',p_from,p_to,d,encode(sha256(convert_to(core.periodic_policy_canonical(d),'UTF8')),'hex'),p_actor,p_person,clock_timestamp())
  RETURNING * INTO s;
 PERFORM analytics.audit(p_actor,p_person,'analytics.kpi-snapshot',s.id::text,'RECORDED','KPI_V1');
 RETURN analytics.kpi_snapshot_document(s);
END $$;
CREATE FUNCTION analytics.kpi_snapshot_list(p_actor jsonb,p_person uuid) RETURNS jsonb LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog AS $$ BEGIN
 PERFORM analytics.require(p_actor,p_person,ARRAY['data_steward','finance_approver','finance_adjuster','product_approver']);
 RETURN (SELECT coalesce(jsonb_agg(analytics.kpi_snapshot_document(s) ORDER BY s.created_at DESC),'[]'::jsonb)
  FROM (SELECT * FROM analytics.kpi_snapshots ORDER BY created_at DESC LIMIT 20) s);
END $$;
GRANT EXECUTE ON FUNCTION analytics.kpi_snapshot(jsonb,uuid,timestamptz,timestamptz),analytics.kpi_snapshot_list(jsonb,uuid) TO bank_data;
