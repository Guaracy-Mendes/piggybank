-- owner: finance (liquidity forecasts, versioned limits, restrictions, automated actions), analytics (read-only flow
-- history), core (the approved settlement-prefunding command), payments (reads the restriction gate only).
-- WP-40. Forecasts are reproducible from stored inputs and a warehouse fact cut. Only fresh signed liquidity evidence
-- counts as confirmed funds. A breach restricts the policy's restricted actions and opens an owned Finance case. An
-- automated action is proposed by a forecast, confirmed by a Finance maker, approved by an independent approver and
-- booked once by Core, which re-checks the approval and the approved limits itself.

-- ===== Warehouse flow history (WP-37 facts at a fixed cut; deterministic for reproduction) =====
-- Daily outflow = sum of settlement-account entries for the listed posting kinds, per business date, for the p_days
-- business dates before p_business_date. Called only from Finance definer functions; no role gains analytics access.
CREATE FUNCTION analytics.liquidity_history(p_business_date date,p_days integer,p_kinds text[],p_fact_cut bigint) RETURNS jsonb
LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
 WITH c AS (SELECT coalesce(p_fact_cut,(SELECT max(seq) FROM analytics.facts),0) AS cut),
 d AS (SELECT (p_business_date-g)::date AS day FROM generate_series(1,p_days) g),
 f AS (SELECT x.business_date AS day,
   sum(CASE WHEN x.record->>'accountKind'='SETTLEMENT' AND x.record->>'postingKind'=ANY(p_kinds) THEN (x.record->>'amountMinor')::numeric ELSE 0 END) AS outflow,
   count(*) AS facts
  FROM analytics.facts x,c WHERE x.dataset='ledger_entries' AND x.seq<=c.cut AND x.business_date>=p_business_date-p_days AND x.business_date<p_business_date
  GROUP BY x.business_date)
 SELECT jsonb_build_object('factCut',(SELECT cut FROM c),'businessDate',p_business_date,'days',p_days,'kinds',to_jsonb(p_kinds),
  'series',coalesce((SELECT jsonb_agg(jsonb_build_object('date',d.day,'outflowMinor',coalesce(f.outflow,0)::text,'facts',coalesce(f.facts,0)) ORDER BY d.day)
   FROM d LEFT JOIN f ON f.day=d.day),'[]'::jsonb))
$$;

-- ===== Versioned policies: model, limits, restricted actions, automation, escalation =====
CREATE FUNCTION finance.minor_text(p jsonb) RETURNS boolean LANGUAGE sql IMMUTABLE SET search_path=pg_catalog AS $$
 SELECT jsonb_typeof(p)='string' AND p#>>'{}' ~ '^(0|[1-9][0-9]{0,15})$' $$;
CREATE FUNCTION finance.valid_liquidity_policy(d jsonb) RETURNS boolean LANGUAGE plpgsql IMMUTABLE SET search_path=pg_catalog AS $$
DECLARE m jsonb:=d->'model';l jsonb:=d->'limits';a jsonb:=d->'automation';e jsonb:=d->'escalation';BEGIN
 IF jsonb_typeof(d)<>'object' OR (SELECT array_agg(k ORDER BY k) FROM jsonb_object_keys(d) k)<>ARRAY['automation','escalation','limits','model','restrictedActions'] THEN RETURN false; END IF;
 IF (SELECT array_agg(k ORDER BY k) FROM jsonb_object_keys(m) k)<>ARRAY['backtestDays','k','lookbackDays','method','minHistoryDays','outflowKinds']
  OR m->>'method'<>'TRAILING_MEAN_PLUS_K_SIGMA' OR jsonb_typeof(m->'lookbackDays')<>'number' OR (m->>'lookbackDays')::numeric NOT BETWEEN 3 AND 60 OR (m->>'lookbackDays')::numeric<>trunc((m->>'lookbackDays')::numeric)
  OR jsonb_typeof(m->'minHistoryDays')<>'number' OR (m->>'minHistoryDays')::numeric NOT BETWEEN 1 AND (m->>'lookbackDays')::numeric OR (m->>'minHistoryDays')::numeric<>trunc((m->>'minHistoryDays')::numeric)
  OR jsonb_typeof(m->'backtestDays')<>'number' OR (m->>'backtestDays')::numeric NOT BETWEEN 0 AND 30 OR (m->>'backtestDays')::numeric<>trunc((m->>'backtestDays')::numeric)
  OR jsonb_typeof(m->'k')<>'number' OR (m->>'k')::numeric NOT BETWEEN 0 AND 5
  OR jsonb_typeof(m->'outflowKinds')<>'array' OR jsonb_array_length(m->'outflowKinds')=0
  OR EXISTS(SELECT 1 FROM jsonb_array_elements(m->'outflowKinds') x WHERE x#>>'{}' NOT IN ('EXTERNAL_PAYMENT','BATCH_CREDIT','ATM_WITHDRAWAL')) THEN RETURN false; END IF;
 IF (SELECT array_agg(k ORDER BY k) FROM jsonb_object_keys(l) k)<>ARRAY['maxActionMinor','maxDailyActionMinor','maxForecastAgeSeconds','maxWarehouseLagSeconds','minBufferMinor']
  OR NOT finance.minor_text(l->'minBufferMinor') OR NOT finance.minor_text(l->'maxActionMinor') OR NOT finance.minor_text(l->'maxDailyActionMinor')
  OR (l->>'maxActionMinor')::numeric NOT BETWEEN 1 AND 100000000 OR (l->>'maxDailyActionMinor')::numeric<(l->>'maxActionMinor')::numeric OR (l->>'maxDailyActionMinor')::numeric>100000000
  OR jsonb_typeof(l->'maxWarehouseLagSeconds')<>'number' OR (l->>'maxWarehouseLagSeconds')::numeric NOT BETWEEN 60 AND 86400
  OR jsonb_typeof(l->'maxForecastAgeSeconds')<>'number' OR (l->>'maxForecastAgeSeconds')::numeric NOT BETWEEN 30 AND 3600 THEN RETURN false; END IF;
 IF jsonb_typeof(d->'restrictedActions')<>'array' OR d->'restrictedActions'<>'["EXTERNAL_SUBMISSION"]'::jsonb THEN RETURN false; END IF;
 IF (SELECT array_agg(k ORDER BY k) FROM jsonb_object_keys(a) k)<>ARRAY['proposePrefunding','scheduleMinutes','targetBufferMinor']
  OR jsonb_typeof(a->'proposePrefunding')<>'boolean' OR NOT finance.minor_text(a->'targetBufferMinor') OR (a->>'targetBufferMinor')::numeric<(l->>'minBufferMinor')::numeric
  OR jsonb_typeof(a->'scheduleMinutes')<>'number' OR (a->>'scheduleMinutes')::numeric NOT BETWEEN 0 AND 1440 OR (a->>'scheduleMinutes')::numeric<>trunc((a->>'scheduleMinutes')::numeric) THEN RETURN false; END IF;
 IF (SELECT array_agg(k ORDER BY k) FROM jsonb_object_keys(e) k)<>ARRAY['queue','severity'] OR e->>'queue'<>'finance-operations' OR e->>'severity' NOT IN ('HIGH','CRITICAL') THEN RETURN false; END IF;
 RETURN true;
EXCEPTION WHEN others THEN RETURN false;
END $$;
CREATE FUNCTION finance.digest(p jsonb) RETURNS text LANGUAGE sql IMMUTABLE SET search_path=pg_catalog AS $$
 SELECT encode(sha256(convert_to(core.periodic_policy_canonical(p),'UTF8')),'hex') $$;
CREATE TABLE finance.liquidity_policies(version integer PRIMARY KEY CHECK(version>0),document jsonb NOT NULL,digest text NOT NULL,proposed_by jsonb,proposed_person uuid,
 created_at timestamptz NOT NULL DEFAULT clock_timestamp());
INSERT INTO finance.liquidity_policies(version,document,digest) SELECT 1,d,finance.digest(d) FROM (SELECT
 '{"model":{"method":"TRAILING_MEAN_PLUS_K_SIGMA","lookbackDays":14,"minHistoryDays":5,"k":2,"backtestDays":7,"outflowKinds":["EXTERNAL_PAYMENT","BATCH_CREDIT","ATM_WITHDRAWAL"]},"limits":{"minBufferMinor":"100000","maxActionMinor":"5000000","maxDailyActionMinor":"10000000","maxWarehouseLagSeconds":900,"maxForecastAgeSeconds":600},"restrictedActions":["EXTERNAL_SUBMISSION"],"automation":{"proposePrefunding":true,"targetBufferMinor":"250000","scheduleMinutes":0},"escalation":{"queue":"finance-operations","severity":"CRITICAL"}}'::jsonb d) x;
CREATE TABLE finance.liquidity_policy_head(id boolean PRIMARY KEY DEFAULT true CHECK(id),active_version integer NOT NULL REFERENCES finance.liquidity_policies,
 revision bigint NOT NULL,updated_at timestamptz NOT NULL DEFAULT clock_timestamp());
INSERT INTO finance.liquidity_policy_head(active_version,revision) VALUES(1,1);
CREATE TABLE finance.liquidity_policy_decisions(version integer PRIMARY KEY REFERENCES finance.liquidity_policies,outcome text NOT NULL CHECK(outcome IN ('APPLIED','REJECTED')),
 decided_by jsonb NOT NULL,decided_person uuid NOT NULL,decided_at timestamptz NOT NULL DEFAULT clock_timestamp());

-- ===== Forecasts, reproductions, escalations, restrictions, lifts, actions =====
CREATE TABLE finance.liquidity_forecasts(id uuid PRIMARY KEY,policy_version integer NOT NULL REFERENCES finance.liquidity_policies,business_date date NOT NULL,
 inputs jsonb NOT NULL,history jsonb NOT NULL,output jsonb NOT NULL,status text NOT NULL CHECK(status IN ('CONFIRMED','UNCERTAIN','UNCONFIRMED')),
 outcome text NOT NULL CHECK(outcome IN ('WITHIN_LIMITS','BREACH','DEGRADED')),digest text NOT NULL,requested_by jsonb NOT NULL,requested_person uuid,
 created_at timestamptz NOT NULL DEFAULT clock_timestamp());
CREATE INDEX liquidity_forecasts_recent ON finance.liquidity_forecasts(created_at DESC);
CREATE TABLE finance.liquidity_reproductions(id uuid PRIMARY KEY,forecast_id uuid NOT NULL REFERENCES finance.liquidity_forecasts,matched boolean NOT NULL,
 recomputed_digest text NOT NULL,requested_by jsonb NOT NULL,requested_person uuid NOT NULL,created_at timestamptz NOT NULL DEFAULT clock_timestamp());
CREATE TABLE finance.liquidity_escalations(id uuid PRIMARY KEY,forecast_id uuid NOT NULL UNIQUE REFERENCES finance.liquidity_forecasts,
 kind text NOT NULL CHECK(kind IN ('BREACH','DEGRADED')),case_id uuid NOT NULL,reused boolean NOT NULL,created_at timestamptz NOT NULL DEFAULT clock_timestamp());
CREATE TABLE finance.liquidity_restrictions(id uuid PRIMARY KEY,forecast_id uuid NOT NULL UNIQUE REFERENCES finance.liquidity_forecasts,actions text[] NOT NULL,
 case_id uuid NOT NULL,created_at timestamptz NOT NULL DEFAULT clock_timestamp());
CREATE TABLE finance.liquidity_lifts(id uuid PRIMARY KEY,restriction_id uuid NOT NULL REFERENCES finance.liquidity_restrictions,proposed_by jsonb NOT NULL,
 proposed_person uuid NOT NULL,created_at timestamptz NOT NULL DEFAULT clock_timestamp());
CREATE TABLE finance.liquidity_lift_decisions(lift_id uuid PRIMARY KEY REFERENCES finance.liquidity_lifts,restriction_id uuid NOT NULL REFERENCES finance.liquidity_restrictions,
 outcome text NOT NULL CHECK(outcome IN ('APPLIED','REJECTED')),forecast_id uuid REFERENCES finance.liquidity_forecasts,decided_by jsonb NOT NULL,decided_person uuid NOT NULL,
 decided_at timestamptz NOT NULL DEFAULT clock_timestamp());
CREATE UNIQUE INDEX one_applied_lift ON finance.liquidity_lift_decisions(restriction_id) WHERE outcome='APPLIED';
CREATE TABLE finance.liquidity_actions(id uuid PRIMARY KEY,forecast_id uuid NOT NULL UNIQUE REFERENCES finance.liquidity_forecasts,
 kind text NOT NULL CHECK(kind='SETTLEMENT_PREFUNDING'),details jsonb NOT NULL,digest text NOT NULL,amount_minor bigint NOT NULL CHECK(amount_minor>0),
 created_at timestamptz NOT NULL DEFAULT clock_timestamp());
CREATE TABLE finance.liquidity_action_confirmations(action_id uuid PRIMARY KEY REFERENCES finance.liquidity_actions,maker jsonb NOT NULL,maker_person uuid NOT NULL,
 confirmed_at timestamptz NOT NULL DEFAULT clock_timestamp());
CREATE TABLE finance.liquidity_action_decisions(action_id uuid PRIMARY KEY REFERENCES finance.liquidity_action_confirmations,outcome text NOT NULL CHECK(outcome IN ('APPROVED','REJECTED')),
 checker jsonb NOT NULL,checker_person uuid NOT NULL,decided_at timestamptz NOT NULL DEFAULT clock_timestamp());
CREATE TABLE finance.liquidity_action_executions(action_id uuid PRIMARY KEY REFERENCES finance.liquidity_action_decisions,batch_id uuid NOT NULL UNIQUE,
 executed_by jsonb NOT NULL,executed_at timestamptz NOT NULL DEFAULT clock_timestamp());
DO $$ DECLARE t text;BEGIN
 FOREACH t IN ARRAY ARRAY['liquidity_policies','liquidity_policy_decisions','liquidity_forecasts','liquidity_reproductions','liquidity_escalations','liquidity_restrictions',
  'liquidity_lifts','liquidity_lift_decisions','liquidity_actions','liquidity_action_confirmations','liquidity_action_decisions','liquidity_action_executions'] LOOP
  EXECUTE format('CREATE TRIGGER immutable_%s BEFORE UPDATE OR DELETE OR TRUNCATE ON finance.%I FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal()',t,t);
  EXECUTE format('REVOKE ALL ON finance.%I FROM bank_finance',t);
  EXECUTE format('GRANT SELECT ON finance.%I TO bank_finance',t);
 END LOOP;
END $$;
REVOKE ALL ON finance.liquidity_policy_head FROM bank_finance;
GRANT SELECT ON finance.liquidity_policy_head TO bank_finance;

-- ===== Core: the approved settlement-prefunding command (one booking per action, limits re-checked here) =====
INSERT INTO core.ledger_accounts(id,kind,state,debit_blocked) VALUES('4f1a780b-3e31-48ef-a294-00000000f1d0','SETTLEMENT','ACTIVE',false) ON CONFLICT(id) DO NOTHING;
CREATE TABLE core.liquidity_action_receipts(execution_id uuid PRIMARY KEY,batch_id uuid NOT NULL UNIQUE REFERENCES core.journal_batches(id),details jsonb NOT NULL,
 details_digest text NOT NULL,amount_minor bigint NOT NULL CHECK(amount_minor>0),maker_person uuid NOT NULL,checker_person uuid NOT NULL,
 recorded_at timestamptz NOT NULL DEFAULT clock_timestamp(),CHECK(maker_person<>checker_person));
CREATE TRIGGER immutable_liquidity_action_receipts BEFORE UPDATE OR DELETE OR TRUNCATE ON core.liquidity_action_receipts FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal();
REVOKE ALL ON core.liquidity_action_receipts FROM bank_core;
GRANT SELECT ON core.liquidity_action_receipts TO bank_core;
CREATE FUNCTION core.execute_liquidity_action(p_details jsonb,p_digest text,p_maker uuid,p_checker uuid) RETURNS TABLE(batch_id uuid,created boolean)
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,core AS $$
DECLARE execution uuid:=(p_details->>'executionId')::uuid;amount bigint;saved core.liquidity_action_receipts;policy jsonb;today numeric;
 src uuid:='4f1a780b-3e31-48ef-a294-00000000f1d0';dst uuid:='4f1a780b-3e31-48ef-a294-000000000003';target uuid;BEGIN
 IF execution IS NULL THEN RAISE EXCEPTION 'Exact liquidity action details required'; END IF;
 PERFORM pg_advisory_xact_lock(hashtextextended('core.liquidity-actions',0));
 SELECT * INTO saved FROM core.liquidity_action_receipts WHERE execution_id=execution;
 IF FOUND THEN
  IF saved.details<>p_details OR saved.details_digest<>p_digest OR saved.maker_person<>p_maker OR saved.checker_person<>p_checker THEN RAISE EXCEPTION 'Liquidity action receipt identity conflict'; END IF;
  RETURN QUERY SELECT saved.batch_id,false;RETURN;
 END IF;
 IF p_details->>'kind' IS DISTINCT FROM 'SETTLEMENT_PREFUNDING' OR p_details->>'fromAccount' IS DISTINCT FROM src::text OR p_details->>'toAccount' IS DISTINCT FROM dst::text
  OR p_details->>'currency' IS DISTINCT FROM 'TST' OR p_details->>'label' IS DISTINCT FROM 'SYNTHETIC' OR NOT finance.minor_text(p_details->'amountMinor')
  OR p_digest IS DISTINCT FROM finance.digest(p_details) THEN RAISE EXCEPTION 'Exact liquidity action details required'; END IF;
 amount:=(p_details->>'amountMinor')::bigint;
 IF amount<=0 THEN RAISE EXCEPTION 'Exact liquidity action details required'; END IF;
 IF p_maker IS NULL OR p_checker IS NULL OR p_maker=p_checker THEN RAISE EXCEPTION 'Independent liquidity approval required'; END IF;
 IF NOT EXISTS(SELECT 1 FROM controls.role_grants WHERE person_id=p_maker AND role='finance_adjuster' AND scope IS NULL AND revoked_at IS NULL AND valid_from<=clock_timestamp() AND valid_until>clock_timestamp())
  OR NOT EXISTS(SELECT 1 FROM controls.role_grants WHERE person_id=p_checker AND role='finance_approver' AND scope IS NULL AND revoked_at IS NULL AND valid_from<=clock_timestamp() AND valid_until>clock_timestamp()) THEN
  RAISE EXCEPTION 'Current Finance maker and approver required'; END IF;
 -- The Finance approval must bind exactly these details, people and a recent approval.
 IF NOT EXISTS(SELECT 1 FROM finance.liquidity_actions a JOIN finance.liquidity_action_confirmations c ON c.action_id=a.id JOIN finance.liquidity_action_decisions d ON d.action_id=a.id
   WHERE a.id=execution AND a.details=p_details AND a.digest=p_digest AND a.amount_minor=amount AND c.maker_person=p_maker AND d.checker_person=p_checker
    AND d.outcome='APPROVED' AND d.decided_at>clock_timestamp()-interval '15 minutes') THEN RAISE EXCEPTION 'Approved liquidity action required'; END IF;
 -- Approved limits are re-checked at the balance authority.
 SELECT document INTO policy FROM finance.liquidity_policies WHERE version=(p_details->>'policyVersion')::integer;
 IF policy IS NULL OR amount>(policy->'limits'->>'maxActionMinor')::numeric THEN RAISE EXCEPTION 'Liquidity action exceeds the approved per-action limit'; END IF;
 SELECT coalesce(sum(r.amount_minor),0) INTO today FROM core.liquidity_action_receipts r WHERE (r.recorded_at AT TIME ZONE 'UTC')::date=(clock_timestamp() AT TIME ZONE 'UTC')::date;
 IF today+amount>(policy->'limits'->>'maxDailyActionMinor')::numeric THEN RAISE EXCEPTION 'Liquidity action exceeds the approved daily limit'; END IF;
 PERFORM 1 FROM core.ledger_accounts WHERE id IN (src,dst) ORDER BY id FOR UPDATE;
 IF EXISTS(SELECT 1 FROM core.ledger_accounts WHERE id IN (src,dst) AND (state<>'ACTIVE' OR (id=src AND debit_blocked) OR (id=dst AND credit_blocked))) THEN RAISE EXCEPTION 'Liquidity accounts unavailable'; END IF;
 target:=gen_random_uuid();
 INSERT INTO core.journal_batches(id,operation_id,posting_kind,fingerprint,business_date) VALUES(target,execution,'LIQUIDITY_PREFUNDING',p_digest,(clock_timestamp() AT TIME ZONE 'UTC')::date);
 UPDATE core.ledger_accounts SET booked=booked-amount,version=version+1 WHERE id=src;
 UPDATE core.ledger_accounts SET booked=booked+amount,version=version+1 WHERE id=dst;
 INSERT INTO core.journal_entries VALUES(target,src,-amount),(target,dst,amount);
 INSERT INTO core.liquidity_action_receipts VALUES(execution,target,p_details,p_digest,amount,p_maker,p_checker,clock_timestamp());
 RETURN QUERY SELECT target,true;
END $$;
GRANT EXECUTE ON FUNCTION core.execute_liquidity_action(jsonb,text,uuid,uuid) TO bank_core;

-- ===== Forecast computation (pure: same policy, inputs and history give the same output) =====
CREATE FUNCTION finance.forecast_compute(p_policy jsonb,p_inputs jsonb,p_history jsonb) RETURNS jsonb LANGUAGE plpgsql IMMUTABLE SET search_path=pg_catalog AS $$
DECLARE m jsonb:=p_policy->'model';l jsonb:=p_policy->'limits';lb integer:=(m->>'lookbackDays')::integer;bt integer:=(m->>'backtestDays')::integer;k numeric:=(m->>'k')::numeric;
 vals numeric[];counts integer[];n integer;w numeric[];mean numeric;sd numeric;expected numeric;high numeric;days_with integer;flags text[]:='{}';
 liq jsonb:=p_inputs->'liquidity';confirmed numeric;low numeric;projected numeric;buffer numeric:=(l->>'minBufferMinor')::numeric;status text;outcome text;prefund numeric;
 j integer;pm numeric;ps numeric;err_m numeric:=0;err_n numeric:=0;covered integer:=0;backtest jsonb;lag numeric;BEGIN
 SELECT array_agg((e->>'outflowMinor')::numeric ORDER BY o),array_agg((e->>'facts')::integer ORDER BY o) INTO vals,counts
  FROM jsonb_array_elements(p_history->'series') WITH ORDINALITY s(e,o);
 n:=coalesce(array_length(vals,1),0);
 IF n<lb+bt THEN RAISE EXCEPTION 'History does not cover the model window'; END IF;
 w:=vals[n-lb+1:n];
 SELECT avg(v) INTO mean FROM unnest(w) v;SELECT sqrt(avg((v-mean)^2)) INTO sd FROM unnest(w) v;
 SELECT count(*) INTO days_with FROM unnest(counts[n-lb+1:n]) c WHERE c>0;
 expected:=ceil(mean);high:=ceil(mean+k*sd);
 -- Confirmed funds come only from fresh signed evidence against fresh Core positions (WP-24 observation).
 IF liq->>'status'='FRESH' AND finance.minor_text(liq->'available') THEN confirmed:=(liq->>'available')::numeric;
 ELSE flags:=flags||('LIQUIDITY_'||coalesce(liq->>'status','UNAVAILABLE'));
  flags:=flags||ARRAY(SELECT x#>>'{}' FROM jsonb_array_elements(coalesce(liq->'reasonCodes','[]'::jsonb)) x);
 END IF;
 IF p_inputs->'warehouse'->>'checkedAt' IS NULL THEN flags:=flags||'WAREHOUSE_NEVER_INGESTED'::text;
 ELSE lag:=(p_inputs->'warehouse'->>'lagSeconds')::numeric;
  IF lag>(l->>'maxWarehouseLagSeconds')::numeric THEN flags:=flags||'WAREHOUSE_STALE'::text; END IF;
 END IF;
 IF days_with<(m->>'minHistoryDays')::integer THEN flags:=flags||'INSUFFICIENT_HISTORY'::text; END IF;
 IF confirmed IS NOT NULL THEN low:=confirmed-high;projected:=confirmed-expected; END IF;
 status:=CASE WHEN confirmed IS NULL THEN 'UNCONFIRMED' WHEN cardinality(flags)>0 THEN 'UNCERTAIN' ELSE 'CONFIRMED' END;
 outcome:=CASE WHEN confirmed IS NULL THEN 'DEGRADED' WHEN low<buffer THEN 'BREACH' ELSE 'WITHIN_LIMITS' END;
 -- Only a confirmed breach may propose an automated action; the amount is capped by the approved per-action limit.
 IF outcome='BREACH' AND status='CONFIRMED' AND (p_policy->'automation'->>'proposePrefunding')::boolean THEN
  prefund:=least((p_policy->'automation'->>'targetBufferMinor')::numeric-low,(l->>'maxActionMinor')::numeric);
  IF (p_policy->'automation'->>'targetBufferMinor')::numeric-low>(l->>'maxActionMinor')::numeric THEN flags:=flags||'ACTION_CAPPED_BY_LIMIT'::text; END IF;
 END IF;
 -- Rolling backtest against the actual daily outflow; baseline is the previous day's actual (naive).
 IF bt>0 THEN
  FOR j IN n-bt+1..n LOOP
   SELECT avg(v) INTO pm FROM unnest(vals[j-lb:j-1]) v;SELECT sqrt(avg((v-pm)^2)) INTO ps FROM unnest(vals[j-lb:j-1]) v;
   err_m:=err_m+abs(ceil(pm)-vals[j]);err_n:=err_n+abs(vals[j-1]-vals[j]);
   IF vals[j]<=ceil(pm+k*ps) THEN covered:=covered+1; END IF;
  END LOOP;
  backtest:=jsonb_build_object('days',bt,'observedDays',(SELECT count(*) FROM unnest(counts[n-bt+1:n]) c WHERE c>0),'modelMaeMinor',round(err_m/bt,2)::text,'naiveMaeMinor',round(err_n/bt,2)::text,'bandCoverage',round(covered::numeric/bt,4)::text,
   'baseline','PREVIOUS_DAY_ACTUAL','evidence','LOCAL_SYNTHETIC_HISTORY_ONLY');
 END IF;
 RETURN jsonb_build_object('status',status,'outcome',outcome,'flags',to_jsonb(flags),'confirmedAvailableMinor',confirmed::text,
  'expectedOutflowMinor',expected::text,'highOutflowMinor',high::text,'projectedExpectedMinor',projected::text,'projectedLowMinor',low::text,
  'minBufferMinor',buffer::text,'prefundingMinor',prefund::text,'daysWithData',days_with,'meanMinor',round(mean,2)::text,'stddevMinor',round(sd,2)::text,
  'backtest',backtest,'confirmedFundsRule','FRESH_SIGNED_EVIDENCE_MINUS_CORE_COMMITMENTS');
END $$;

CREATE FUNCTION finance.liquidity_audit(p_actor jsonb,p_person uuid,p_action text,p_resource text,p_outcome text,p_reason text) RETURNS void
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog AS $$ BEGIN
 PERFORM controls.append_audit(gen_random_uuid(),'liquidity',jsonb_build_object('actorIssuer',coalesce(p_actor->>'issuer','urn:piggybank:finance'),
  'actorSubject',coalesce(p_actor->>'subject','liquidity-forecast'),'actorPersonId',p_person,'representedParty',NULL,'action',p_action,'resource',p_resource,
  'outcome',p_outcome,'policyVersion','liquidity-v1','reasonCode',p_reason,'correlationId',gen_random_uuid(),
  'occurredAt',to_char(clock_timestamp() AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"')));
END $$;
CREATE FUNCTION finance.utc(p timestamptz) RETURNS text LANGUAGE sql IMMUTABLE SET search_path=pg_catalog AS
$$ SELECT to_char(p AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"') $$;
CREATE FUNCTION finance.require_liquidity(p_actor jsonb,p_person uuid,p_roles text[]) RETURNS void LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog AS $$ BEGIN
 IF p_person IS NULL THEN RAISE EXCEPTION 'Current canonical identity required'; END IF;
 PERFORM controls.require_staff(p_actor,p_person,p_roles);
END $$;
CREATE FUNCTION finance.active_restriction() RETURNS finance.liquidity_restrictions LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
 SELECT r.* FROM finance.liquidity_restrictions r WHERE NOT EXISTS(SELECT 1 FROM finance.liquidity_lift_decisions d WHERE d.restriction_id=r.id AND d.outcome='APPLIED')
 ORDER BY r.created_at LIMIT 1 $$;
-- The gate Payments reads before a new external submission.
CREATE FUNCTION finance.liquidity_restricted(p_action text) RETURNS boolean LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
 SELECT EXISTS(SELECT 1 FROM finance.liquidity_restrictions r WHERE p_action=ANY(r.actions)
  AND NOT EXISTS(SELECT 1 FROM finance.liquidity_lift_decisions d WHERE d.restriction_id=r.id AND d.outcome='APPLIED')) $$;

CREATE FUNCTION finance.action_state(p_action uuid) RETURNS text LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
 SELECT CASE WHEN e.action_id IS NOT NULL THEN 'EXECUTED' WHEN d.outcome='APPROVED' THEN 'APPROVED' WHEN d.outcome='REJECTED' THEN 'REJECTED'
  WHEN a.forecast_id<>(SELECT id FROM finance.liquidity_forecasts ORDER BY created_at DESC,id LIMIT 1)
   OR f.created_at<clock_timestamp()-make_interval(secs=>(p.document->'limits'->>'maxForecastAgeSeconds')::integer) THEN 'SUPERSEDED'
  WHEN c.action_id IS NOT NULL THEN 'CONFIRMED' ELSE 'PROPOSED' END
 FROM finance.liquidity_actions a JOIN finance.liquidity_forecasts f ON f.id=a.forecast_id JOIN finance.liquidity_policies p ON p.version=f.policy_version
  LEFT JOIN finance.liquidity_action_confirmations c ON c.action_id=a.id LEFT JOIN finance.liquidity_action_decisions d ON d.action_id=a.id
  LEFT JOIN finance.liquidity_action_executions e ON e.action_id=a.id WHERE a.id=p_action $$;
CREATE FUNCTION finance.forecast_document(p_id uuid) RETURNS jsonb LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
 SELECT jsonb_build_object('id',f.id,'policyVersion',f.policy_version,'businessDate',f.business_date,'status',f.status,'outcome',f.outcome,'digest',f.digest,
  'inputs',f.inputs,'history',f.history,'output',f.output,'requestedBy',coalesce(f.requested_by->>'subject','service'),'automatic',f.requested_person IS NULL,
  'createdAt',finance.utc(f.created_at),
  'escalation',(SELECT jsonb_build_object('kind',e.kind,'caseId',e.case_id,'reused',e.reused,'caseState',c.state) FROM finance.liquidity_escalations e
   LEFT JOIN controls.cases c ON c.id=e.case_id WHERE e.forecast_id=f.id),
  'restrictionId',(SELECT r.id FROM finance.liquidity_restrictions r WHERE r.forecast_id=f.id),
  'action',(SELECT jsonb_build_object('id',a.id,'amountMinor',a.amount_minor::text,'state',finance.action_state(a.id)) FROM finance.liquidity_actions a WHERE a.forecast_id=f.id),
  'reproductions',(SELECT coalesce(jsonb_agg(jsonb_build_object('matched',r.matched,'at',finance.utc(r.created_at)) ORDER BY r.created_at),'[]'::jsonb)
   FROM finance.liquidity_reproductions r WHERE r.forecast_id=f.id))
 FROM finance.liquidity_forecasts f WHERE f.id=p_id $$;

-- ===== Record a forecast and apply its consequences =====
CREATE FUNCTION finance.record_forecast(p_actor jsonb,p_person uuid,p_liquidity jsonb) RETURNS jsonb LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE h finance.liquidity_policy_head;pol jsonb;pol_digest text;hist jsonb;inputs jsonb;v_out jsonb;target uuid:=gen_random_uuid();v_day date:=(clock_timestamp() AT TIME ZONE 'UTC')::date;
 w analytics.watermarks;active finance.liquidity_restrictions;v_case uuid;v_reused boolean:=false;v_kind text;act uuid;details jsonb;BEGIN
 IF p_person IS NOT NULL THEN PERFORM controls.require_staff(p_actor,p_person,ARRAY['finance_adjuster','finance_approver']);
 ELSIF p_actor->>'trust' IS DISTINCT FROM 'SERVICE' THEN RAISE EXCEPTION 'Current trust context required'; END IF;
 IF jsonb_typeof(p_liquidity)<>'object' OR p_liquidity->>'status' NOT IN ('FRESH','STALE','MISSING','INVALID','UNAVAILABLE')
  OR (p_liquidity->>'status'='FRESH' AND NOT finance.minor_text(p_liquidity->'available')) THEN RAISE EXCEPTION 'Invalid liquidity observation' USING ERRCODE='22023'; END IF;
 -- Same lock as warehouse ingestion and snapshots, so the fact cut is consistent; then the forecast lock.
 PERFORM pg_advisory_xact_lock(hashtext('analytics.warehouse'));
 PERFORM pg_advisory_xact_lock(hashtext('finance.liquidity'));
 SELECT * INTO STRICT h FROM finance.liquidity_policy_head;
 SELECT document,digest INTO STRICT pol,pol_digest FROM finance.liquidity_policies WHERE version=h.active_version;
 hist:=analytics.liquidity_history(v_day,(pol->'model'->>'lookbackDays')::integer+(pol->'model'->>'backtestDays')::integer,
  ARRAY(SELECT jsonb_array_elements_text(pol->'model'->'outflowKinds')),NULL);
 SELECT * INTO w FROM analytics.watermarks WHERE dataset='ledger_entries';
 inputs:=jsonb_build_object('asOf',finance.utc(clock_timestamp()),'businessDate',v_day,'liquidity',p_liquidity,
  'warehouse',jsonb_build_object('checkedAt',finance.utc(w.checked_at),'highWater',CASE WHEN w.high_water IS NULL OR w.high_water='-infinity' THEN NULL ELSE finance.utc(w.high_water) END,
   'lagSeconds',CASE WHEN w.checked_at IS NULL THEN NULL ELSE floor(extract(epoch FROM clock_timestamp()-w.checked_at)) END));
 v_out:=finance.forecast_compute(pol,inputs,hist);
 INSERT INTO finance.liquidity_forecasts VALUES(target,h.active_version,v_day,inputs,hist,v_out,v_out->>'status',v_out->>'outcome',
  finance.digest(jsonb_build_object('policyVersion',h.active_version,'policyDigest',pol_digest,'inputs',inputs,'history',hist,'output',v_out)),p_actor,p_person,clock_timestamp());
 PERFORM finance.liquidity_audit(p_actor,p_person,'liquidity.forecast-recorded',target::text,v_out->>'outcome',v_out->>'status');
 v_kind:=CASE v_out->>'outcome' WHEN 'BREACH' THEN 'BREACH' WHEN 'DEGRADED' THEN 'DEGRADED' END;
 IF v_kind IS NOT NULL THEN
  active:=finance.active_restriction();
  -- One owned case per episode: reuse an open case of the same kind (or the active restriction's case).
  IF v_kind='BREACH' AND active.id IS NOT NULL THEN v_case:=active.case_id;v_reused:=true;
  ELSE
   SELECT e.case_id INTO v_case FROM finance.liquidity_escalations e JOIN controls.cases c ON c.id=e.case_id
    WHERE e.kind=v_kind AND c.state<>'CLOSED' ORDER BY e.created_at DESC LIMIT 1;
   v_reused:=v_case IS NOT NULL;
   IF v_case IS NULL THEN
    v_case:=controls.open_case('FINANCE',target,target,pol->'escalation'->>'queue',
     CASE v_kind WHEN 'BREACH' THEN 'RESTORE_LIQUIDITY_BUFFER' ELSE 'RESTORE_LIQUIDITY_EVIDENCE' END,pol->'escalation'->>'severity',
     jsonb_build_object('issuer','urn:piggybank:finance','subject','liquidity-forecast'));
   END IF;
  END IF;
  INSERT INTO finance.liquidity_escalations VALUES(gen_random_uuid(),target,v_kind,v_case,v_reused,clock_timestamp());
  PERFORM finance.liquidity_audit(p_actor,p_person,'liquidity.escalated',v_case::text,'RECORDED',v_kind);
  IF v_kind='BREACH' AND active.id IS NULL THEN
   INSERT INTO finance.liquidity_restrictions VALUES(gen_random_uuid(),target,ARRAY(SELECT jsonb_array_elements_text(pol->'restrictedActions')),v_case,clock_timestamp());
   PERFORM finance.liquidity_audit(p_actor,p_person,'liquidity.restricted',target::text,'RECORDED','POLICY_BREACH');
  END IF;
 END IF;
 IF v_out->>'prefundingMinor' IS NOT NULL AND (v_out->>'prefundingMinor')::numeric>0 THEN
  act:=gen_random_uuid();
  details:=jsonb_build_object('kind','SETTLEMENT_PREFUNDING','executionId',act,'forecastId',target,'policyVersion',h.active_version,'amountMinor',v_out->>'prefundingMinor',
   'currency','TST','fromAccount','4f1a780b-3e31-48ef-a294-00000000f1d0','toAccount','4f1a780b-3e31-48ef-a294-000000000003','label','SYNTHETIC','formatVersion',1);
  INSERT INTO finance.liquidity_actions VALUES(act,target,'SETTLEMENT_PREFUNDING',details,finance.digest(details),(v_out->>'prefundingMinor')::bigint,clock_timestamp());
  PERFORM finance.liquidity_audit(p_actor,p_person,'liquidity.action-proposed',act::text,'RECORDED','CONFIRMED_BREACH');
 END IF;
 RETURN finance.forecast_document(target);
END $$;
-- Worker: due only when the active policy schedules forecasts.
CREATE FUNCTION finance.forecast_due() RETURNS boolean LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
 SELECT (p.document->'automation'->>'scheduleMinutes')::integer>0 AND coalesce((SELECT max(created_at) FROM finance.liquidity_forecasts),'-infinity')
  <clock_timestamp()-make_interval(mins=>(p.document->'automation'->>'scheduleMinutes')::integer)
 FROM finance.liquidity_policy_head h JOIN finance.liquidity_policies p ON p.version=h.active_version $$;
CREATE FUNCTION finance.reproduce_forecast(p_actor jsonb,p_person uuid,p_id uuid) RETURNS jsonb LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE f finance.liquidity_forecasts;pol jsonb;pol_digest text;hist jsonb;v_out jsonb;v_digest text;v_matched boolean;BEGIN
 PERFORM finance.require_liquidity(p_actor,p_person,ARRAY['finance_adjuster','finance_approver']);
 SELECT * INTO f FROM finance.liquidity_forecasts WHERE id=p_id;IF NOT FOUND THEN RAISE EXCEPTION 'Forecast unavailable' USING ERRCODE='P0002'; END IF;
 SELECT document,digest INTO STRICT pol,pol_digest FROM finance.liquidity_policies WHERE version=f.policy_version;
 hist:=analytics.liquidity_history(f.business_date,(f.history->>'days')::integer,ARRAY(SELECT jsonb_array_elements_text(f.history->'kinds')),(f.history->>'factCut')::bigint);
 v_out:=finance.forecast_compute(pol,f.inputs,hist);
 v_digest:=finance.digest(jsonb_build_object('policyVersion',f.policy_version,'policyDigest',pol_digest,'inputs',f.inputs,'history',hist,'output',v_out));
 v_matched:=v_digest=f.digest;
 INSERT INTO finance.liquidity_reproductions VALUES(gen_random_uuid(),p_id,v_matched,v_digest,p_actor,p_person,clock_timestamp());
 PERFORM finance.liquidity_audit(p_actor,p_person,'liquidity.forecast-reproduced',p_id::text,CASE WHEN v_matched THEN 'MATCHED' ELSE 'MISMATCH' END,'REPRODUCTION');
 RETURN jsonb_build_object('forecastId',p_id,'matched',v_matched,'digest',f.digest,'recomputedDigest',v_digest);
END $$;
CREATE FUNCTION finance.forecast(p_actor jsonb,p_person uuid,p_id uuid) RETURNS jsonb LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE d jsonb;BEGIN
 PERFORM finance.require_liquidity(p_actor,p_person,ARRAY['finance_adjuster','finance_approver']);
 d:=finance.forecast_document(p_id);IF d IS NULL THEN RAISE EXCEPTION 'Forecast unavailable' USING ERRCODE='P0002'; END IF;
 RETURN d;
END $$;

-- ===== Policy changes: Finance maker proposes, an independent Finance approver applies =====
CREATE FUNCTION finance.propose_liquidity_policy(p_actor jsonb,p_person uuid,p_document jsonb) RETURNS jsonb LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE v integer;BEGIN
 PERFORM finance.require_liquidity(p_actor,p_person,ARRAY['finance_adjuster']);
 IF NOT finance.valid_liquidity_policy(p_document) THEN RAISE EXCEPTION 'Invalid liquidity policy document' USING ERRCODE='22023'; END IF;
 PERFORM pg_advisory_xact_lock(hashtext('finance.liquidity'));
 v:=(SELECT max(version)+1 FROM finance.liquidity_policies);
 INSERT INTO finance.liquidity_policies VALUES(v,p_document,finance.digest(p_document),p_actor,p_person,clock_timestamp());
 PERFORM finance.liquidity_audit(p_actor,p_person,'liquidity.policy-proposed',v::text,'RECORDED','MAKER');
 RETURN jsonb_build_object('version',v,'digest',finance.digest(p_document));
END $$;
CREATE FUNCTION finance.decide_liquidity_policy(p_actor jsonb,p_person uuid,p_version integer,p_approve boolean) RETURNS jsonb LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE p finance.liquidity_policies;BEGIN
 PERFORM finance.require_liquidity(p_actor,p_person,ARRAY['finance_approver']);
 PERFORM pg_advisory_xact_lock(hashtext('finance.liquidity'));
 SELECT * INTO p FROM finance.liquidity_policies WHERE version=p_version;
 IF NOT FOUND OR p.proposed_person IS NULL THEN RAISE EXCEPTION 'Proposed policy unavailable' USING ERRCODE='P0002'; END IF;
 IF EXISTS(SELECT 1 FROM finance.liquidity_policy_decisions WHERE version=p_version) THEN RAISE EXCEPTION 'Policy already decided'; END IF;
 IF p.proposed_person=p_person THEN RAISE EXCEPTION 'Independent approver required'; END IF;
 INSERT INTO finance.liquidity_policy_decisions VALUES(p_version,CASE WHEN p_approve THEN 'APPLIED' ELSE 'REJECTED' END,p_actor,p_person,clock_timestamp());
 IF p_approve THEN UPDATE finance.liquidity_policy_head SET active_version=p_version,revision=revision+1,updated_at=clock_timestamp(); END IF;
 PERFORM finance.liquidity_audit(p_actor,p_person,'liquidity.policy-decided',p_version::text,CASE WHEN p_approve THEN 'APPLIED' ELSE 'REJECTED' END,'INDEPENDENT_APPROVER');
 RETURN jsonb_build_object('version',p_version,'outcome',CASE WHEN p_approve THEN 'APPLIED' ELSE 'REJECTED' END,
  'activeVersion',(SELECT active_version FROM finance.liquidity_policy_head));
END $$;

-- ===== Automated action: maker confirmation, independent approval, then one Core booking =====
CREATE FUNCTION finance.current_forecast_for(p_forecast uuid) RETURNS boolean LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
 SELECT f.id=p_forecast AND f.status='CONFIRMED' AND f.outcome='BREACH'
  AND f.created_at>=clock_timestamp()-make_interval(secs=>(p.document->'limits'->>'maxForecastAgeSeconds')::integer)
 FROM finance.liquidity_forecasts f JOIN finance.liquidity_policies p ON p.version=f.policy_version ORDER BY f.created_at DESC,f.id LIMIT 1 $$;
CREATE FUNCTION finance.confirm_liquidity_action(p_actor jsonb,p_person uuid,p_action uuid) RETURNS jsonb LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE a finance.liquidity_actions;BEGIN
 PERFORM finance.require_liquidity(p_actor,p_person,ARRAY['finance_adjuster']);
 PERFORM pg_advisory_xact_lock(hashtext('finance.liquidity'));
 SELECT * INTO a FROM finance.liquidity_actions WHERE id=p_action;IF NOT FOUND THEN RAISE EXCEPTION 'Action unavailable' USING ERRCODE='P0002'; END IF;
 IF EXISTS(SELECT 1 FROM finance.liquidity_action_confirmations WHERE action_id=p_action) THEN RAISE EXCEPTION 'Action already confirmed'; END IF;
 IF NOT coalesce(finance.current_forecast_for(a.forecast_id),false) THEN RAISE EXCEPTION 'Current confirmed breach forecast required'; END IF;
 INSERT INTO finance.liquidity_action_confirmations VALUES(p_action,p_actor,p_person,clock_timestamp());
 PERFORM finance.liquidity_audit(p_actor,p_person,'liquidity.action-confirmed',p_action::text,'RECORDED','MAKER');
 RETURN jsonb_build_object('actionId',p_action,'state',finance.action_state(p_action));
END $$;
CREATE FUNCTION finance.decide_liquidity_action(p_actor jsonb,p_person uuid,p_action uuid,p_approve boolean) RETURNS jsonb LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE a finance.liquidity_actions;c finance.liquidity_action_confirmations;BEGIN
 PERFORM finance.require_liquidity(p_actor,p_person,ARRAY['finance_approver']);
 PERFORM pg_advisory_xact_lock(hashtext('finance.liquidity'));
 SELECT * INTO a FROM finance.liquidity_actions WHERE id=p_action;IF NOT FOUND THEN RAISE EXCEPTION 'Action unavailable' USING ERRCODE='P0002'; END IF;
 SELECT * INTO c FROM finance.liquidity_action_confirmations WHERE action_id=p_action;IF NOT FOUND THEN RAISE EXCEPTION 'Maker confirmation required'; END IF;
 IF EXISTS(SELECT 1 FROM finance.liquidity_action_decisions WHERE action_id=p_action) THEN RAISE EXCEPTION 'Action already decided'; END IF;
 IF c.maker_person=p_person THEN RAISE EXCEPTION 'Independent approver required'; END IF;
 IF p_approve AND NOT coalesce(finance.current_forecast_for(a.forecast_id),false) THEN RAISE EXCEPTION 'Current confirmed breach forecast required'; END IF;
 INSERT INTO finance.liquidity_action_decisions VALUES(p_action,CASE WHEN p_approve THEN 'APPROVED' ELSE 'REJECTED' END,p_actor,p_person,clock_timestamp());
 PERFORM finance.liquidity_audit(p_actor,p_person,'liquidity.action-decided',p_action::text,CASE WHEN p_approve THEN 'APPROVED' ELSE 'REJECTED' END,'INDEPENDENT_APPROVER');
 RETURN jsonb_build_object('actionId',p_action,'state',finance.action_state(p_action));
END $$;
CREATE FUNCTION finance.liquidity_action_command(p_actor jsonb,p_person uuid,p_action uuid) RETURNS jsonb LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE r record;BEGIN
 PERFORM finance.require_liquidity(p_actor,p_person,ARRAY['finance_adjuster','finance_approver']);
 SELECT a.details,a.digest,c.maker_person,d.checker_person,d.outcome INTO r FROM finance.liquidity_actions a JOIN finance.liquidity_action_confirmations c ON c.action_id=a.id
  JOIN finance.liquidity_action_decisions d ON d.action_id=a.id WHERE a.id=p_action;
 IF NOT FOUND OR r.outcome<>'APPROVED' THEN RAISE EXCEPTION 'Approved liquidity action required'; END IF;
 RETURN jsonb_build_object('details',r.details,'digest',r.digest,'maker',r.maker_person,'checker',r.checker_person);
END $$;
CREATE FUNCTION finance.record_liquidity_execution(p_actor jsonb,p_person uuid,p_action uuid) RETURNS jsonb LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE b uuid;BEGIN
 PERFORM finance.require_liquidity(p_actor,p_person,ARRAY['finance_adjuster','finance_approver']);
 SELECT r.batch_id INTO b FROM core.liquidity_action_receipts r JOIN finance.liquidity_actions a ON a.id=r.execution_id AND a.digest=r.details_digest WHERE r.execution_id=p_action;
 IF b IS NULL THEN RAISE EXCEPTION 'Core receipt required'; END IF;
 INSERT INTO finance.liquidity_action_executions VALUES(p_action,b,p_actor,clock_timestamp()) ON CONFLICT(action_id) DO NOTHING;
 IF FOUND THEN PERFORM finance.liquidity_audit(p_actor,p_person,'liquidity.action-executed',p_action::text,'BOOKED','CORE_RECEIPT'); END IF;
 RETURN jsonb_build_object('actionId',p_action,'batchId',b,'state',finance.action_state(p_action));
END $$;

-- ===== Lifting a restriction: maker proposes, independent approver applies on a current within-limits forecast =====
CREATE FUNCTION finance.propose_restriction_lift(p_actor jsonb,p_person uuid,p_restriction uuid) RETURNS jsonb LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE target uuid:=gen_random_uuid();BEGIN
 PERFORM finance.require_liquidity(p_actor,p_person,ARRAY['finance_adjuster']);
 PERFORM pg_advisory_xact_lock(hashtext('finance.liquidity'));
 IF NOT EXISTS(SELECT 1 FROM finance.liquidity_restrictions WHERE id=p_restriction) THEN RAISE EXCEPTION 'Restriction unavailable' USING ERRCODE='P0002'; END IF;
 IF EXISTS(SELECT 1 FROM finance.liquidity_lift_decisions WHERE restriction_id=p_restriction AND outcome='APPLIED') THEN RAISE EXCEPTION 'Restriction already lifted'; END IF;
 IF EXISTS(SELECT 1 FROM finance.liquidity_lifts l WHERE l.restriction_id=p_restriction AND NOT EXISTS(SELECT 1 FROM finance.liquidity_lift_decisions d WHERE d.lift_id=l.id)) THEN
  RAISE EXCEPTION 'A lift is already pending'; END IF;
 INSERT INTO finance.liquidity_lifts VALUES(target,p_restriction,p_actor,p_person,clock_timestamp());
 PERFORM finance.liquidity_audit(p_actor,p_person,'liquidity.lift-proposed',target::text,'RECORDED','MAKER');
 RETURN jsonb_build_object('liftId',target,'restrictionId',p_restriction);
END $$;
CREATE FUNCTION finance.decide_restriction_lift(p_actor jsonb,p_person uuid,p_lift uuid,p_approve boolean) RETURNS jsonb LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE l finance.liquidity_lifts;r finance.liquidity_restrictions;f record;BEGIN
 PERFORM finance.require_liquidity(p_actor,p_person,ARRAY['finance_approver']);
 PERFORM pg_advisory_xact_lock(hashtext('finance.liquidity'));
 SELECT * INTO l FROM finance.liquidity_lifts WHERE id=p_lift;IF NOT FOUND THEN RAISE EXCEPTION 'Lift unavailable' USING ERRCODE='P0002'; END IF;
 IF EXISTS(SELECT 1 FROM finance.liquidity_lift_decisions WHERE lift_id=p_lift) THEN RAISE EXCEPTION 'Lift already decided'; END IF;
 IF l.proposed_person=p_person THEN RAISE EXCEPTION 'Independent approver required'; END IF;
 SELECT * INTO STRICT r FROM finance.liquidity_restrictions WHERE id=l.restriction_id;
 SELECT x.id,x.status,x.outcome,x.created_at,(p.document->'limits'->>'maxForecastAgeSeconds')::integer AS age INTO f
  FROM finance.liquidity_forecasts x JOIN finance.liquidity_policies p ON p.version=x.policy_version ORDER BY x.created_at DESC,x.id LIMIT 1;
 IF p_approve AND (f.id IS NULL OR f.status<>'CONFIRMED' OR f.outcome<>'WITHIN_LIMITS' OR f.created_at<=r.created_at OR f.created_at<clock_timestamp()-make_interval(secs=>f.age)) THEN
  RAISE EXCEPTION 'Current confirmed within-limits forecast required'; END IF;
 INSERT INTO finance.liquidity_lift_decisions VALUES(p_lift,r.id,CASE WHEN p_approve THEN 'APPLIED' ELSE 'REJECTED' END,CASE WHEN p_approve THEN f.id END,p_actor,p_person,clock_timestamp());
 PERFORM finance.liquidity_audit(p_actor,p_person,'liquidity.lift-decided',p_lift::text,CASE WHEN p_approve THEN 'APPLIED' ELSE 'REJECTED' END,'INDEPENDENT_APPROVER');
 RETURN jsonb_build_object('liftId',p_lift,'outcome',CASE WHEN p_approve THEN 'APPLIED' ELSE 'REJECTED' END,'restricted',finance.liquidity_restricted('EXTERNAL_SUBMISSION'));
END $$;

-- ===== Staff view: policies, forecasts, restrictions, actions, escalations and outcomes against actuals =====
CREATE FUNCTION finance.liquidity_view(p_actor jsonb,p_person uuid) RETURNS jsonb LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE h finance.liquidity_policy_head;BEGIN
 PERFORM finance.require_liquidity(p_actor,p_person,ARRAY['finance_adjuster','finance_approver']);
 SELECT * INTO STRICT h FROM finance.liquidity_policy_head;
 RETURN jsonb_build_object('activeVersion',h.active_version,'revision',h.revision,
  'restricted',finance.liquidity_restricted('EXTERNAL_SUBMISSION'),
  'policies',(SELECT jsonb_agg(jsonb_build_object('version',p.version,'document',p.document,'digest',p.digest,'proposedBy',p.proposed_by->>'subject',
    'outcome',d.outcome,'decidedBy',d.decided_by->>'subject') ORDER BY p.version DESC) FROM finance.liquidity_policies p LEFT JOIN finance.liquidity_policy_decisions d ON d.version=p.version),
  'forecasts',(SELECT coalesce(jsonb_agg(finance.forecast_document(x.id) ORDER BY x.created_at DESC),'[]'::jsonb)
    FROM (SELECT id,created_at FROM finance.liquidity_forecasts ORDER BY created_at DESC LIMIT 10) x),
  'restrictions',(SELECT coalesce(jsonb_agg(jsonb_build_object('id',r.id,'forecastId',r.forecast_id,'actions',to_jsonb(r.actions),'caseId',r.case_id,'createdAt',finance.utc(r.created_at),
    'active',NOT EXISTS(SELECT 1 FROM finance.liquidity_lift_decisions d WHERE d.restriction_id=r.id AND d.outcome='APPLIED'),
    'lifts',(SELECT coalesce(jsonb_agg(jsonb_build_object('id',l.id,'proposedBy',l.proposed_by->>'subject','outcome',d.outcome,'decidedBy',d.decided_by->>'subject','forecastId',d.forecast_id)
      ORDER BY l.created_at),'[]'::jsonb) FROM finance.liquidity_lifts l LEFT JOIN finance.liquidity_lift_decisions d ON d.lift_id=l.id WHERE l.restriction_id=r.id))
    ORDER BY r.created_at DESC),'[]'::jsonb) FROM finance.liquidity_restrictions r),
  'actions',(SELECT coalesce(jsonb_agg(jsonb_build_object('id',a.id,'forecastId',a.forecast_id,'kind',a.kind,'amountMinor',a.amount_minor::text,'state',finance.action_state(a.id),
    'confirmedBy',c.maker->>'subject','decidedBy',d.checker->>'subject','batchId',e.batch_id,'createdAt',finance.utc(a.created_at)) ORDER BY a.created_at DESC),'[]'::jsonb)
    FROM finance.liquidity_actions a LEFT JOIN finance.liquidity_action_confirmations c ON c.action_id=a.id LEFT JOIN finance.liquidity_action_decisions d ON d.action_id=a.id
    LEFT JOIN finance.liquidity_action_executions e ON e.action_id=a.id),
  'outcomes',(SELECT coalesce(jsonb_agg(jsonb_build_object('forecastId',f.id,'businessDate',f.business_date,'expectedOutflowMinor',f.output->>'expectedOutflowMinor',
    'highOutflowMinor',f.output->>'highOutflowMinor','actualOutflowMinor',act.v,'withinBand',act.v::numeric<=(f.output->>'highOutflowMinor')::numeric) ORDER BY f.created_at DESC),'[]'::jsonb)
    FROM (SELECT * FROM finance.liquidity_forecasts WHERE business_date<(clock_timestamp() AT TIME ZONE 'UTC')::date ORDER BY created_at DESC LIMIT 10) f
    CROSS JOIN LATERAL (SELECT analytics.liquidity_history(f.business_date+1,1,ARRAY(SELECT jsonb_array_elements_text(f.history->'kinds')),NULL)->'series'->0->>'outflowMinor' AS v) act),
  'evidence','LOCAL_SYNTHETIC_ONLY');
END $$;

REVOKE ALL ON FUNCTION analytics.liquidity_history(date,integer,text[],bigint) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.record_forecast(jsonb,uuid,jsonb),finance.forecast_due(),finance.reproduce_forecast(jsonb,uuid,uuid),finance.forecast(jsonb,uuid,uuid),
 finance.propose_liquidity_policy(jsonb,uuid,jsonb),finance.decide_liquidity_policy(jsonb,uuid,integer,boolean),finance.confirm_liquidity_action(jsonb,uuid,uuid),
 finance.decide_liquidity_action(jsonb,uuid,uuid,boolean),finance.liquidity_action_command(jsonb,uuid,uuid),finance.record_liquidity_execution(jsonb,uuid,uuid),
 finance.propose_restriction_lift(jsonb,uuid,uuid),finance.decide_restriction_lift(jsonb,uuid,uuid,boolean),finance.liquidity_view(jsonb,uuid),
 finance.liquidity_restricted(text) TO bank_finance;
GRANT USAGE ON SCHEMA finance TO bank_payments;
GRANT EXECUTE ON FUNCTION finance.liquidity_restricted(text) TO bank_payments;
