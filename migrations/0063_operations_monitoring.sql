-- owner: controls (transaction monitoring, operations snapshots) and data (data quality, notifications).
-- WP-26. Monitoring and data-quality functions read financial facts; none of them can write a balance or journal.

ALTER TABLE controls.cases DROP CONSTRAINT cases_kind_check;
ALTER TABLE controls.cases ADD CONSTRAINT cases_kind_check CHECK(kind IN ('CONTROL','SUPPORT','PAYMENT','DATA','FINANCE','MONITORING'));

-- Stage a case.changed fact and an audit record exactly as the Rust case service does.
CREATE FUNCTION controls.case_changed(p_case uuid,p_actor jsonb,p_action text) RETURNS void
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,controls AS $$
DECLARE c controls.cases;e jsonb;target uuid:=gen_random_uuid();stamp text:=to_char(clock_timestamp() AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"');BEGIN
 SELECT * INTO STRICT c FROM controls.cases WHERE id=p_case;
 e:=jsonb_build_object('eventId',target,'eventType','case.changed','schemaVersion',1,'producer','controls','aggregateId',c.id,'aggregateVersion',c.version,'occurredAt',stamp,
  'correlationId',c.id,'causationId',c.id::text,'data',jsonb_build_object('caseId',c.id,'state',c.state,'severity',c.severity,'queue',c.queue));
 INSERT INTO controls.outbox(event_id,aggregate_id,aggregate_version,document,fingerprint) VALUES(target,c.id,c.version,e,encode(sha256(convert_to(core.periodic_policy_canonical(e),'UTF8')),'hex'));
 PERFORM controls.append_audit(gen_random_uuid(),'cases',jsonb_build_object('actorIssuer',coalesce(p_actor->>'issuer','urn:piggybank:worker'),'actorSubject',coalesce(p_actor->>'subject','monitoring'),
  'actorPersonId',NULL,'representedParty',NULL,'action',p_action,'resource',c.id::text,'outcome','RECORDED','policyVersion','synthetic-v1','reasonCode','DOMAIN_EVENT','correlationId',c.id,'occurredAt',stamp));
END $$;
CREATE FUNCTION controls.open_case(p_kind text,p_origin uuid,p_resource uuid,p_queue text,p_next text,p_severity text,p_actor jsonb) RETURNS uuid
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,controls AS $$
DECLARE target uuid:=gen_random_uuid();made uuid;BEGIN
 INSERT INTO controls.cases(id,kind,origin_id,resource_id,queue,severity,next_action,deadline) VALUES(target,p_kind,p_origin,p_resource,p_queue,p_severity,p_next,clock_timestamp()+interval '4 hours')
 ON CONFLICT(kind,origin_id) DO NOTHING RETURNING id INTO made;
 IF made IS NULL THEN SELECT id INTO STRICT made FROM controls.cases WHERE kind=p_kind AND origin_id=p_origin; RETURN made; END IF;
 PERFORM controls.case_changed(made,p_actor,'case.created');
 RETURN made;
END $$;
CREATE FUNCTION controls.require_staff(p_actor jsonb,p_person uuid,p_roles text[]) RETURNS void
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,controls AS $$ BEGIN
 IF p_actor->>'trust' IS DISTINCT FROM 'WORKFORCE' THEN RAISE EXCEPTION 'Current trust context required'; END IF;
 PERFORM 1 FROM controls.person_links WHERE issuer=p_actor->>'issuer' AND subject=p_actor->>'subject' AND person_id=p_person FOR SHARE;
 IF NOT FOUND THEN RAISE EXCEPTION 'Current canonical identity required'; END IF;
 PERFORM 1 FROM controls.role_grants WHERE person_id=p_person AND role=ANY(p_roles) AND scope IS NULL AND revoked_at IS NULL AND valid_from<=clock_timestamp() AND valid_until>clock_timestamp() FOR SHARE;
 IF NOT FOUND THEN RAISE EXCEPTION 'Current operations role required'; END IF;
END $$;
CREATE FUNCTION controls.utc_text(p_at timestamptz) RETURNS text LANGUAGE sql IMMUTABLE STRICT SET search_path=pg_catalog AS
$$ SELECT to_char(p_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"') $$;

-- ===== Transaction monitoring (FR-CTRL-04) =====
CREATE TABLE controls.monitoring_rules(
 id uuid PRIMARY KEY,code text NOT NULL CHECK(code IN ('RAPID_OUTFLOW','STRUCTURED_CREDITS')),version integer NOT NULL CHECK(version>0),
 label text NOT NULL CHECK(label='SYNTHETIC_FIXTURE'),document jsonb NOT NULL,digest text NOT NULL CHECK(length(digest)=64),effective_from timestamptz NOT NULL,UNIQUE(code,version)
);
INSERT INTO controls.monitoring_rules
SELECT id,code,1,'SYNTHETIC_FIXTURE',doc,encode(sha256(convert_to(core.periodic_policy_canonical(doc),'UTF8')),'hex'),'2026-01-01T00:00:00Z' FROM (VALUES
 ('9b8f0c3e-5d2a-4f61-8a77-000000000001'::uuid,'RAPID_OUTFLOW',jsonb_build_object('code','RAPID_OUTFLOW','version',1,'direction','DEBIT','kinds',jsonb_build_array('INTERNAL_PAYMENT','EXTERNAL_PAYMENT','SIMULATED_CARD_PURCHASE'),
   'windowSeconds',900,'minCount',3,'minTotalMinor',30000,'minAmountMinor',1,'maxAmountMinor',NULL,'description','Three or more outgoing payments or purchases totaling at least 300.00 TST within 15 minutes.')),
 ('9b8f0c3e-5d2a-4f61-8a77-000000000002'::uuid,'STRUCTURED_CREDITS',jsonb_build_object('code','STRUCTURED_CREDITS','version',1,'direction','CREDIT','kinds',jsonb_build_array('SYNTHETIC_FUNDING','SIMULATED_DEPOSIT','INTERNAL_PAYMENT','CREDIT_ALLOCATION'),
   'windowSeconds',86400,'minCount',3,'minTotalMinor',0,'minAmountMinor',9000,'maxAmountMinor',9999,'description','Three or more incoming credits each between 90.00 and 99.99 TST within 24 hours.'))
) v(id,code,doc);
CREATE TABLE controls.monitoring_evaluations(rule_id uuid NOT NULL REFERENCES controls.monitoring_rules(id),batch_id uuid NOT NULL,account_id uuid NOT NULL,evaluated_at timestamptz NOT NULL DEFAULT clock_timestamp(),PRIMARY KEY(rule_id,batch_id,account_id));
CREATE TABLE controls.monitoring_alerts(
 id uuid PRIMARY KEY,rule_id uuid NOT NULL REFERENCES controls.monitoring_rules(id),account_id uuid NOT NULL,anchor_operation uuid NOT NULL,trigger_operation uuid NOT NULL,
 window_start timestamptz NOT NULL,window_end timestamptz NOT NULL,evidence jsonb NOT NULL,evidence_digest text NOT NULL CHECK(length(evidence_digest)=64),
 case_id uuid NOT NULL REFERENCES controls.cases(id),source text NOT NULL CHECK(source IN ('STREAM','RETROSPECTIVE')),created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 UNIQUE(rule_id,account_id,anchor_operation),CHECK(window_end>=window_start)
);
CREATE INDEX monitoring_alerts_episode ON controls.monitoring_alerts(rule_id,account_id,window_end DESC);
CREATE TABLE controls.monitoring_dispositions(
 alert_id uuid PRIMARY KEY REFERENCES controls.monitoring_alerts(id),disposition text NOT NULL CHECK(disposition IN ('NO_FURTHER_ACTION','REFER_FINANCIAL_CRIME')),
 rationale text NOT NULL CHECK(length(rationale) BETWEEN 3 AND 1000),maker_person uuid NOT NULL,maker_actor jsonb NOT NULL,proposed_at timestamptz NOT NULL,
 checker_person uuid,checker_actor jsonb,approved_at timestamptz,CHECK(checker_person IS NULL OR checker_person<>maker_person)
);
CREATE TABLE controls.monitoring_scans(id uuid PRIMARY KEY,actor_context jsonb NOT NULL,person_id uuid NOT NULL,from_at timestamptz NOT NULL,to_at timestamptz NOT NULL,evaluated integer NOT NULL,created integer NOT NULL,existing integer NOT NULL,recorded_at timestamptz NOT NULL DEFAULT clock_timestamp());
CREATE TRIGGER immutable_monitoring_rules BEFORE UPDATE OR DELETE OR TRUNCATE ON controls.monitoring_rules FOR EACH STATEMENT EXECUTE FUNCTION controls.immutable_check_result();
CREATE TRIGGER immutable_monitoring_alerts BEFORE UPDATE OR DELETE OR TRUNCATE ON controls.monitoring_alerts FOR EACH STATEMENT EXECUTE FUNCTION controls.immutable_check_result();
CREATE TRIGGER immutable_monitoring_evaluations BEFORE UPDATE OR DELETE OR TRUNCATE ON controls.monitoring_evaluations FOR EACH STATEMENT EXECUTE FUNCTION controls.immutable_check_result();
CREATE TRIGGER immutable_monitoring_scans BEFORE UPDATE OR DELETE OR TRUNCATE ON controls.monitoring_scans FOR EACH STATEMENT EXECUTE FUNCTION controls.immutable_check_result();
CREATE FUNCTION controls.guard_monitoring_disposition() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN
 IF TG_OP<>'UPDATE' OR OLD.checker_person IS NOT NULL OR NEW.checker_person IS NULL
  OR (to_jsonb(NEW)-ARRAY['checker_person','checker_actor','approved_at']) IS DISTINCT FROM (to_jsonb(OLD)-ARRAY['checker_person','checker_actor','approved_at']) THEN RAISE EXCEPTION 'Immutable monitoring disposition'; END IF;RETURN NEW;END $$;
CREATE TRIGGER monitoring_disposition_guard BEFORE UPDATE OR DELETE ON controls.monitoring_dispositions FOR EACH ROW EXECUTE FUNCTION controls.guard_monitoring_disposition();

-- Durable projected facts only: a row is a Core journal entry that arrived through a journal.batch-posted event.
CREATE FUNCTION controls.monitoring_facts(p_rule jsonb,p_account uuid,p_from timestamptz,p_to timestamptz)
RETURNS TABLE(batch_id uuid,operation_id uuid,posted_at timestamptz,amount_minor bigint,kind text,origin_event uuid)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
 SELECT t.batch_id,t.operation_id,j.created_at,abs(e.delta),j.posting_kind,t.origin_event
 FROM data.transaction_views t JOIN core.journal_batches j ON j.id=t.batch_id JOIN core.journal_entries e ON e.batch_id=t.batch_id AND e.account_id=t.account_id
 WHERE t.account_id=p_account AND j.created_at>=p_from AND j.created_at<=p_to
  AND j.posting_kind IN (SELECT jsonb_array_elements_text(p_rule->'kinds'))
  AND CASE WHEN p_rule->>'direction'='DEBIT' THEN e.delta<0 ELSE e.delta>0 END
  AND abs(e.delta)>=(p_rule->>'minAmountMinor')::bigint AND (p_rule->'maxAmountMinor'='null'::jsonb OR abs(e.delta)<=(p_rule->>'maxAmountMinor')::bigint)
 ORDER BY j.created_at,t.batch_id $$;
-- Evaluates one projected fact. Duplicate events, replays and retrospective scans converge on one alert per episode.
CREATE FUNCTION controls.evaluate_monitoring(p_rule uuid,p_batch uuid,p_account uuid,p_source text,p_actor jsonb) RETURNS jsonb
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,controls AS $$
DECLARE r controls.monitoring_rules;t record;n integer;total numeric;first_at timestamptz;anchor uuid;evidence jsonb;existing controls.monitoring_alerts;target uuid:=gen_random_uuid();made uuid;
BEGIN
 SELECT * INTO STRICT r FROM controls.monitoring_rules WHERE id=p_rule;
 SELECT * INTO t FROM controls.monitoring_facts(r.document,p_account,'-infinity','infinity') f WHERE f.batch_id=p_batch;
 IF NOT FOUND THEN RETURN jsonb_build_object('result','NOT_APPLICABLE'); END IF;
 -- Only Core customer accounts in the account registry are monitored.
 IF NOT EXISTS(SELECT 1 FROM core.account_registry WHERE id=p_account) THEN RETURN jsonb_build_object('result','NOT_APPLICABLE'); END IF;
 PERFORM pg_advisory_xact_lock(hashtextextended('controls.monitoring:'||p_rule||':'||p_account,0));
 SELECT count(*),coalesce(sum(f.amount_minor),0),min(f.posted_at) INTO n,total,first_at
 FROM controls.monitoring_facts(r.document,p_account,t.posted_at-make_interval(secs=>(r.document->>'windowSeconds')::int),t.posted_at) f;
 IF n<(r.document->>'minCount')::int OR total<(r.document->>'minTotalMinor')::numeric THEN RETURN jsonb_build_object('result','NO_MATCH'); END IF;
 SELECT f.operation_id INTO anchor FROM controls.monitoring_facts(r.document,p_account,first_at,t.posted_at) f ORDER BY f.posted_at,f.batch_id LIMIT 1;
 SELECT * INTO existing FROM controls.monitoring_alerts a WHERE a.rule_id=p_rule AND a.account_id=p_account AND (a.anchor_operation=anchor OR a.window_end>=first_at) ORDER BY a.window_end DESC LIMIT 1;
 IF FOUND THEN RETURN jsonb_build_object('result','EXISTING','alertId',existing.id,'caseId',existing.case_id); END IF;
 SELECT jsonb_build_object('ruleId',r.id,'ruleCode',r.code,'ruleVersion',r.version,'ruleDigest',r.digest,'accountId',p_account,'count',n,'totalMinor',total::text,
  'facts',jsonb_agg(jsonb_build_object('batchId',f.batch_id,'operationId',f.operation_id,'postedAt',controls.utc_text(f.posted_at),'amountMinor',f.amount_minor::text,'kind',f.kind,'originEvent',f.origin_event) ORDER BY f.posted_at,f.batch_id))
 INTO evidence FROM controls.monitoring_facts(r.document,p_account,first_at,t.posted_at) f;
 made:=controls.open_case('MONITORING',target,p_account,'controls-operations','INVESTIGATE_MONITORING_ALERT','HIGH',coalesce(p_actor,jsonb_build_object('issuer','urn:piggybank:worker','subject','transaction-monitoring')));
 INSERT INTO controls.monitoring_alerts VALUES(target,p_rule,p_account,anchor,t.operation_id,first_at,t.posted_at,evidence,encode(sha256(convert_to(core.periodic_policy_canonical(evidence),'UTF8')),'hex'),made,p_source,clock_timestamp());
 RETURN jsonb_build_object('result','CREATED','alertId',target,'caseId',made);
END $$;
CREATE FUNCTION controls.current_monitoring_rules() RETURNS SETOF controls.monitoring_rules LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS
$$ SELECT DISTINCT ON (code) * FROM controls.monitoring_rules WHERE effective_from<=clock_timestamp() ORDER BY code,version DESC $$;
-- Stream tick: every projected fact from the last seven days is evaluated exactly once per current rule.
CREATE FUNCTION controls.monitoring_tick(p_limit integer) RETURNS integer
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,controls AS $$
DECLARE x record;created integer:=0;result jsonb;BEGIN
 IF p_limit NOT BETWEEN 1 AND 2000 THEN RAISE EXCEPTION 'Bounded monitoring tick required'; END IF;
 FOR x IN SELECT r.id AS rule_id,t.batch_id,t.account_id FROM data.transaction_views t CROSS JOIN controls.current_monitoring_rules() r
  WHERE t.recorded_at>clock_timestamp()-interval '7 days' AND NOT EXISTS(SELECT 1 FROM controls.monitoring_evaluations v WHERE v.rule_id=r.id AND v.batch_id=t.batch_id AND v.account_id=t.account_id)
  ORDER BY t.recorded_at,t.batch_id,t.account_id,r.id LIMIT p_limit LOOP
  result:=controls.evaluate_monitoring(x.rule_id,x.batch_id,x.account_id,'STREAM',NULL);
  INSERT INTO controls.monitoring_evaluations(rule_id,batch_id,account_id) VALUES(x.rule_id,x.batch_id,x.account_id) ON CONFLICT DO NOTHING;
  IF result->>'result'='CREATED' THEN created:=created+1; END IF;
 END LOOP;
 RETURN created;
END $$;
-- Retrospective investigation over a bounded window re-evaluates every fact; it never duplicates an alert.
CREATE FUNCTION controls.monitoring_scan(p_actor jsonb,p_person uuid,p_from timestamptz,p_to timestamptz) RETURNS jsonb
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,controls AS $$
DECLARE x record;result jsonb;evaluated integer:=0;created integer:=0;existing integer:=0;target uuid:=gen_random_uuid();BEGIN
 PERFORM controls.require_staff(p_actor,p_person,ARRAY['case_reviewer']);
 IF p_from IS NULL OR p_to IS NULL OR p_to<=p_from OR p_to-p_from>interval '31 days' THEN RAISE EXCEPTION 'Bounded retrospective window required' USING ERRCODE='22023'; END IF;
 IF (SELECT count(*) FROM data.transaction_views t JOIN core.journal_batches j ON j.id=t.batch_id WHERE j.created_at>=p_from AND j.created_at<p_to)>5000 THEN RAISE EXCEPTION 'Retrospective window exceeds 5000 facts; narrow it'; END IF;
 FOR x IN SELECT r.id AS rule_id,t.batch_id,t.account_id FROM data.transaction_views t JOIN core.journal_batches j ON j.id=t.batch_id CROSS JOIN controls.current_monitoring_rules() r
  WHERE j.created_at>=p_from AND j.created_at<p_to ORDER BY j.created_at,t.batch_id,t.account_id,r.id LOOP
  result:=controls.evaluate_monitoring(x.rule_id,x.batch_id,x.account_id,'RETROSPECTIVE',p_actor);
  evaluated:=evaluated+1;
  IF result->>'result'='CREATED' THEN created:=created+1; ELSIF result->>'result'='EXISTING' THEN existing:=existing+1; END IF;
 END LOOP;
 INSERT INTO controls.monitoring_scans VALUES(target,p_actor,p_person,p_from,p_to,evaluated,created,existing,clock_timestamp());
 PERFORM controls.append_audit(gen_random_uuid(),'monitoring',jsonb_build_object('actorIssuer',p_actor->>'issuer','actorSubject',p_actor->>'subject','actorPersonId',p_person,'representedParty',NULL,
  'action','monitoring.retrospective-scan','resource',target::text,'outcome','RECORDED','policyVersion','synthetic-v1','reasonCode','DOMAIN_EVENT','correlationId',target,'occurredAt',controls.utc_text(clock_timestamp())));
 RETURN jsonb_build_object('scanId',target,'evaluated',evaluated,'created',created,'existing',existing);
END $$;
CREATE FUNCTION controls.monitoring_alert_json(a controls.monitoring_alerts) RETURNS jsonb LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
 SELECT jsonb_build_object('id',a.id,'ruleCode',r.code,'ruleVersion',r.version,'description',r.document->>'description','accountId',a.account_id,'anchorOperation',a.anchor_operation,
  'triggerOperation',a.trigger_operation,'windowStart',controls.utc_text(a.window_start),'windowEnd',controls.utc_text(a.window_end),'evidence',a.evidence,'evidenceDigest',a.evidence_digest,
  'source',a.source,'createdAt',controls.utc_text(a.created_at),'caseId',c.id,'caseState',c.state,'caseSeverity',c.severity,'caseNextAction',c.next_action,
  'disposition',CASE WHEN d.alert_id IS NULL THEN NULL ELSE jsonb_build_object('disposition',d.disposition,'rationale',d.rationale,'makerPerson',d.maker_person,'proposedAt',controls.utc_text(d.proposed_at),
   'checkerPerson',d.checker_person,'approvedAt',controls.utc_text(d.approved_at)) END)
 FROM controls.monitoring_rules r,controls.cases c LEFT JOIN controls.monitoring_dispositions d ON d.alert_id=a.id WHERE r.id=a.rule_id AND c.id=a.case_id $$;
CREATE FUNCTION controls.monitoring_alerts_view(p_actor jsonb,p_person uuid) RETURNS jsonb
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,controls AS $$ BEGIN
 PERFORM controls.require_staff(p_actor,p_person,ARRAY['case_reviewer','case_approver']);
 RETURN jsonb_build_object('rules',(SELECT jsonb_agg(document||jsonb_build_object('id',id,'digest',digest) ORDER BY code,version) FROM controls.monitoring_rules),
  'alerts',coalesce((SELECT jsonb_agg(controls.monitoring_alert_json(a) ORDER BY a.created_at DESC) FROM (SELECT * FROM controls.monitoring_alerts ORDER BY created_at DESC LIMIT 100) a),'[]'::jsonb),
  'scans',coalesce((SELECT jsonb_agg(jsonb_build_object('id',id,'from',controls.utc_text(from_at),'to',controls.utc_text(to_at),'evaluated',evaluated,'created',created,'existing',existing,'recordedAt',controls.utc_text(recorded_at)) ORDER BY recorded_at DESC)
    FROM (SELECT * FROM controls.monitoring_scans ORDER BY recorded_at DESC LIMIT 10) s),'[]'::jsonb));
END $$;
CREATE FUNCTION controls.propose_monitoring_disposition(p_actor jsonb,p_person uuid,p_alert uuid,p_disposition text,p_rationale text) RETURNS jsonb
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,controls AS $$
DECLARE a controls.monitoring_alerts;d controls.monitoring_dispositions;BEGIN
 PERFORM controls.require_staff(p_actor,p_person,ARRAY['case_reviewer']);
 PERFORM 1 FROM controls.case_queue_members WHERE queue='controls-operations' AND person_id=p_person AND capability='REVIEW' AND revoked_at IS NULL;
 IF NOT FOUND THEN RAISE EXCEPTION 'Current controls queue required'; END IF;
 SELECT * INTO a FROM controls.monitoring_alerts WHERE id=p_alert;IF NOT FOUND THEN RAISE EXCEPTION 'Alert unavailable' USING ERRCODE='P0002'; END IF;
 PERFORM 1 FROM controls.cases WHERE id=a.case_id FOR UPDATE;
 SELECT * INTO d FROM controls.monitoring_dispositions WHERE alert_id=p_alert;
 IF FOUND THEN
  IF d.maker_person=p_person AND d.disposition=p_disposition AND d.rationale=trim(p_rationale) THEN RETURN controls.monitoring_alert_json(a); END IF;
  RAISE EXCEPTION 'Disposition already proposed';
 END IF;
 IF p_disposition NOT IN ('NO_FURTHER_ACTION','REFER_FINANCIAL_CRIME') OR length(trim(coalesce(p_rationale,''))) NOT BETWEEN 3 AND 1000 THEN RAISE EXCEPTION 'Supported disposition and rationale required' USING ERRCODE='22023'; END IF;
 INSERT INTO controls.monitoring_dispositions VALUES(p_alert,p_disposition,trim(p_rationale),p_person,p_actor,clock_timestamp(),NULL,NULL,NULL);
 UPDATE controls.cases SET state='PROPOSED',owner_person=coalesce(owner_person,p_person),next_action='INDEPENDENT_APPROVAL',version=version+1,updated_at=clock_timestamp() WHERE id=a.case_id;
 INSERT INTO controls.case_actions VALUES(gen_random_uuid(),a.case_id,p_actor->>'issuer',p_actor->>'subject','monitoring.disposition-proposed',p_disposition,clock_timestamp());
 PERFORM controls.case_changed(a.case_id,p_actor,'case.disposition-proposed');
 RETURN controls.monitoring_alert_json(a);
END $$;
CREATE FUNCTION controls.approve_monitoring_disposition(p_actor jsonb,p_person uuid,p_alert uuid) RETURNS jsonb
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,controls AS $$
DECLARE a controls.monitoring_alerts;d controls.monitoring_dispositions;BEGIN
 PERFORM controls.require_staff(p_actor,p_person,ARRAY['case_approver']);
 PERFORM 1 FROM controls.case_queue_members WHERE queue='controls-operations' AND person_id=p_person AND capability='APPROVE' AND revoked_at IS NULL;
 IF NOT FOUND THEN RAISE EXCEPTION 'Current controls approval queue required'; END IF;
 SELECT * INTO a FROM controls.monitoring_alerts WHERE id=p_alert;IF NOT FOUND THEN RAISE EXCEPTION 'Alert unavailable' USING ERRCODE='P0002'; END IF;
 PERFORM 1 FROM controls.cases WHERE id=a.case_id FOR UPDATE;
 SELECT * INTO d FROM controls.monitoring_dispositions WHERE alert_id=p_alert FOR UPDATE;
 IF NOT FOUND THEN RAISE EXCEPTION 'No proposed disposition'; END IF;
 IF d.maker_person=p_person THEN RAISE EXCEPTION 'Independent approval required'; END IF;
 IF d.checker_person IS NOT NULL THEN
  IF d.checker_person=p_person THEN RETURN controls.monitoring_alert_json(a); END IF;
  RAISE EXCEPTION 'Disposition already approved';
 END IF;
 UPDATE controls.monitoring_dispositions SET checker_person=p_person,checker_actor=p_actor,approved_at=clock_timestamp() WHERE alert_id=p_alert;
 UPDATE controls.cases SET state='CLOSED',next_action=CASE WHEN d.disposition='REFER_FINANCIAL_CRIME' THEN 'REFERRED_FINANCIAL_CRIME' ELSE 'COMPLETE' END,version=version+1,updated_at=clock_timestamp() WHERE id=a.case_id;
 INSERT INTO controls.case_actions VALUES(gen_random_uuid(),a.case_id,p_actor->>'issuer',p_actor->>'subject','monitoring.disposition-approved',d.disposition,clock_timestamp());
 PERFORM controls.case_changed(a.case_id,p_actor,'case.disposition-approved');
 RETURN controls.monitoring_alert_json(a);
END $$;

-- ===== Operations reporting (FR-CTRL-05). Call inside one REPEATABLE READ transaction for a fixed snapshot. =====
CREATE TABLE controls.operations_snapshots(id uuid PRIMARY KEY,actor_context jsonb NOT NULL,person_id uuid NOT NULL,as_of timestamptz NOT NULL,counts jsonb NOT NULL,digest text NOT NULL CHECK(length(digest)=64),recorded_at timestamptz NOT NULL DEFAULT clock_timestamp());
CREATE TRIGGER immutable_operations_snapshots BEFORE UPDATE OR DELETE OR TRUNCATE ON controls.operations_snapshots FOR EACH STATEMENT EXECUTE FUNCTION controls.immutable_check_result();
CREATE FUNCTION controls.operations_counts() RETURNS jsonb LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,controls AS $$
DECLARE s text;outbox jsonb:='{}';dead jsonb:='{}';n bigint;oldest timestamptz;BEGIN
 FOREACH s IN ARRAY ARRAY['platform','identity','core','controls','journeys','payments','finance','documents','data','simulator'] LOOP
  EXECUTE format('SELECT count(*),min(created_at) FROM %I.outbox WHERE delivered_at IS NULL',s) INTO n,oldest;
  outbox:=outbox||jsonb_build_object(s,jsonb_build_object('undelivered',n,'oldestAgeSeconds',CASE WHEN oldest IS NULL THEN NULL ELSE floor(extract(epoch FROM clock_timestamp()-oldest))::bigint END));
  EXECUTE format('SELECT count(*) FROM %I.dead_letters WHERE state=''PENDING''',s) INTO n;
  dead:=dead||jsonb_build_object(s,n);
 END LOOP;
 RETURN jsonb_build_object(
  'payments',jsonb_build_object(
   'intentsByState',coalesce((SELECT jsonb_object_agg(state,c) FROM (SELECT state,count(*) c FROM payments.intents WHERE state IN ('ACCEPTED','CONTROLS_PENDING','CONTROL_REVIEW') GROUP BY state) x),'{}'::jsonb),
   'submissionsAwaitingOutcome',(SELECT count(*) FROM payments.rail_submissions s JOIN payments.intents i ON i.id=s.payment_id WHERE i.state NOT IN ('SETTLED','DECLINED','EXPIRED','BLOCKED')),
   'openRecoveryRequests',(SELECT count(*) FROM payments.recovery_requests WHERE state<>'COMPLETED'),
   'openReconciliationBreaks',(SELECT count(*) FROM payments.reconciliation_breaks WHERE state='OPEN')),
  'controls',jsonb_build_object(
   'pendingChecks',(SELECT count(*) FROM controls.check_requests WHERE state='PENDING'),
   'overdueChecks',(SELECT count(*) FROM controls.check_requests WHERE state='PENDING' AND deadline<clock_timestamp())),
  'cases',jsonb_build_object(
   'openByQueue',coalesce((SELECT jsonb_object_agg(queue,c) FROM (SELECT queue,count(*) c FROM controls.cases WHERE state<>'CLOSED' GROUP BY queue) x),'{}'::jsonb),
   'openByKind',coalesce((SELECT jsonb_object_agg(kind,c) FROM (SELECT kind,count(*) c FROM controls.cases WHERE state<>'CLOSED' GROUP BY kind) x),'{}'::jsonb),
   'overdue',(SELECT count(*) FROM controls.cases WHERE state<>'CLOSED' AND deadline<clock_timestamp()),
   'escalated',(SELECT count(*) FROM controls.cases WHERE state<>'CLOSED' AND escalated_at IS NOT NULL)),
  'finance',jsonb_build_object(
   'reconciliationIssues',(SELECT count(*) FROM finance.reconciliation_issues),
   'periodicIssues',(SELECT count(*) FROM finance.periodic_issues)),
  'integrations',jsonb_build_object('outbox',outbox,'deadLetters',dead),
  'monitoring',jsonb_build_object('openAlerts',(SELECT count(*) FROM controls.monitoring_alerts a JOIN controls.cases c ON c.id=a.case_id WHERE c.state<>'CLOSED')));
END $$;

-- ===== Data quality (FR-DATA-05) =====
CREATE TABLE data.quality_alerts(
 id uuid PRIMARY KEY,check_code text NOT NULL CHECK(check_code IN ('MISSING_FINANCIAL_EVENT','QUARANTINED_EVENT','BROKEN_REFERENCE','STALE_PROJECTION','UNDELIVERED_EVENTS','PROVIDER_BACKLOG','RECONCILIATION_BREAK_AGED','NOTIFICATION_FAILED')),
 subject text NOT NULL CHECK(length(subject) BETWEEN 1 AND 200),state text NOT NULL CHECK(state IN ('OPEN','CLEARED')),next_action text NOT NULL,observation jsonb NOT NULL,
 case_id uuid NOT NULL,first_detected_at timestamptz NOT NULL,last_detected_at timestamptz NOT NULL,cleared_at timestamptz,
 acknowledged_by uuid,acknowledged_at timestamptz,acknowledgement text,CHECK((state='CLEARED')=(cleared_at IS NOT NULL))
);
CREATE UNIQUE INDEX quality_alert_open ON data.quality_alerts(check_code,subject) WHERE state='OPEN';
CREATE TABLE data.quality_alert_history(alert_id uuid NOT NULL REFERENCES data.quality_alerts(id),version integer NOT NULL,state text NOT NULL,observation jsonb NOT NULL,recorded_at timestamptz NOT NULL DEFAULT clock_timestamp(),PRIMARY KEY(alert_id,version));
CREATE TRIGGER immutable_quality_history BEFORE UPDATE OR DELETE OR TRUNCATE ON data.quality_alert_history FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal();
CREATE FUNCTION data.guard_quality_alert() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN
 IF TG_OP<>'UPDATE' OR NEW.id<>OLD.id OR NEW.check_code<>OLD.check_code OR NEW.subject<>OLD.subject OR NEW.case_id<>OLD.case_id OR NEW.first_detected_at<>OLD.first_detected_at
  OR (OLD.state='CLEARED' AND NEW.state<>'CLEARED') OR (OLD.acknowledged_at IS NOT NULL AND (NEW.acknowledged_at IS DISTINCT FROM OLD.acknowledged_at OR NEW.acknowledged_by IS DISTINCT FROM OLD.acknowledged_by)) THEN
  RAISE EXCEPTION 'Quality alert identity and closed history are immutable'; END IF;RETURN NEW;END $$;
CREATE TRIGGER quality_alert_guard BEFORE UPDATE OR DELETE ON data.quality_alerts FOR EACH ROW EXECUTE FUNCTION data.guard_quality_alert();
CREATE TRIGGER quality_alert_truncate BEFORE TRUNCATE ON data.quality_alerts FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal();
CREATE TABLE data.projection_repairs(id uuid PRIMARY KEY,batch_id uuid NOT NULL,actor_context jsonb NOT NULL,person_id uuid NOT NULL,rows_inserted integer NOT NULL,origin_event uuid NOT NULL,recorded_at timestamptz NOT NULL DEFAULT clock_timestamp());
CREATE TRIGGER immutable_projection_repairs BEFORE UPDATE OR DELETE OR TRUNCATE ON data.projection_repairs FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal();
CREATE FUNCTION data.utc_text(p_at timestamptz) RETURNS text LANGUAGE sql IMMUTABLE STRICT SET search_path=pg_catalog AS
$$ SELECT to_char(p_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"') $$;

-- Synthetic fixture thresholds (seconds). Only the migration owner can change them; disposable test databases lower them.
CREATE TABLE data.quality_thresholds(check_code text PRIMARY KEY,seconds integer NOT NULL CHECK(seconds>=0),label text NOT NULL CHECK(label='SYNTHETIC_FIXTURE'));
INSERT INTO data.quality_thresholds VALUES('MISSING_FINANCIAL_EVENT',60,'SYNTHETIC_FIXTURE'),('BROKEN_REFERENCE',60,'SYNTHETIC_FIXTURE'),('STALE_PROJECTION',30,'SYNTHETIC_FIXTURE'),
 ('UNDELIVERED_EVENTS',60,'SYNTHETIC_FIXTURE'),('PROVIDER_BACKLOG',600,'SYNTHETIC_FIXTURE'),('RECONCILIATION_BREAK_AGED',3600,'SYNTHETIC_FIXTURE');
CREATE FUNCTION data.threshold(p_code text) RETURNS interval LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS
$$ SELECT make_interval(secs=>seconds) FROM data.quality_thresholds WHERE check_code=p_code $$;
-- Current findings: (check, subject, observation). Each threshold is a synthetic fixture; NFR-08 lag is 30 s at p99.
CREATE FUNCTION data.quality_findings() RETURNS TABLE(check_code text,subject text,observation jsonb)
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,data AS $$
#variable_conflict use_column
DECLARE s text;n bigint;oldest timestamptz;BEGIN
 RETURN QUERY SELECT 'MISSING_FINANCIAL_EVENT',j.id::text,jsonb_build_object('batchId',j.id,'operationId',j.operation_id,'postingKind',j.posting_kind,'postedAt',data.utc_text(j.created_at),
   'eventPublished',EXISTS(SELECT 1 FROM core.outbox o WHERE o.aggregate_id=j.id AND o.delivered_at IS NOT NULL))
  FROM core.journal_batches j WHERE j.created_at<=clock_timestamp()-data.threshold('MISSING_FINANCIAL_EVENT') AND j.created_at>clock_timestamp()-interval '7 days'
   AND EXISTS(SELECT 1 FROM core.journal_entries e JOIN core.account_registry a ON a.id=e.account_id WHERE e.batch_id=j.id)
   AND NOT EXISTS(SELECT 1 FROM data.transaction_views t WHERE t.batch_id=j.id) ORDER BY j.created_at LIMIT 100;
 RETURN QUERY SELECT 'QUARANTINED_EVENT',d.consumer||':'||d.event_id,jsonb_build_object('consumer',d.consumer,'eventId',d.event_id,'reason',d.reason,'eventType',d.document->>'eventType','aggregateId',d.document->>'aggregateId','quarantinedAt',data.utc_text(d.created_at))
  FROM data.dead_letters d WHERE d.state='PENDING' ORDER BY d.created_at LIMIT 100;
 RETURN QUERY SELECT 'BROKEN_REFERENCE',t.account_id::text,jsonb_build_object('accountId',t.account_id,'reason','TRANSACTION_WITHOUT_ACCOUNT_VIEW','exampleBatchId',min(t.batch_id::text))
  FROM data.transaction_views t WHERE t.recorded_at<=clock_timestamp()-data.threshold('BROKEN_REFERENCE') AND NOT EXISTS(SELECT 1 FROM data.account_views v WHERE v.account_id=t.account_id)
   AND EXISTS(SELECT 1 FROM core.account_registry a WHERE a.id=t.account_id) GROUP BY t.account_id LIMIT 100;
 RETURN QUERY SELECT 'BROKEN_REFERENCE',v.account_id::text,jsonb_build_object('accountId',v.account_id,'reason','ACCOUNT_VIEW_WITHOUT_PARTY_REFERENCE','partyId',v.party_id)
  FROM data.account_views v WHERE v.recorded_at<=clock_timestamp()-data.threshold('BROKEN_REFERENCE') AND NOT EXISTS(SELECT 1 FROM data.master_refs m WHERE m.kind='PARTY' AND m.source_ref=v.party_id) LIMIT 100;
 SELECT min(j.created_at) INTO oldest FROM core.journal_batches j WHERE j.created_at>clock_timestamp()-interval '7 days'
  AND EXISTS(SELECT 1 FROM core.journal_entries e JOIN core.account_registry a ON a.id=e.account_id WHERE e.batch_id=j.id) AND NOT EXISTS(SELECT 1 FROM data.transaction_views t WHERE t.batch_id=j.id);
 IF oldest IS NOT NULL AND oldest<=clock_timestamp()-data.threshold('STALE_PROJECTION') THEN
  RETURN QUERY SELECT 'STALE_PROJECTION','data.transaction_views',jsonb_build_object('projection','data.transaction_views','oldestUnprojectedAt',data.utc_text(oldest),'lagSeconds',floor(extract(epoch FROM clock_timestamp()-oldest))::bigint,'objectiveSeconds',(SELECT q.seconds FROM data.quality_thresholds q WHERE q.check_code='STALE_PROJECTION'));
 END IF;
 FOREACH s IN ARRAY ARRAY['platform','identity','core','controls','journeys','payments','finance','documents','data','simulator'] LOOP
  EXECUTE format('SELECT count(*),min(created_at) FROM %I.outbox WHERE delivered_at IS NULL',s) INTO n,oldest;
  IF oldest IS NOT NULL AND oldest<=clock_timestamp()-data.threshold('UNDELIVERED_EVENTS') THEN
   RETURN QUERY SELECT 'UNDELIVERED_EVENTS',s,jsonb_build_object('module',s,'undelivered',n,'oldestCreatedAt',data.utc_text(oldest),'lagSeconds',floor(extract(epoch FROM clock_timestamp()-oldest))::bigint);
  END IF;
 END LOOP;
 RETURN QUERY SELECT 'PROVIDER_BACKLOG',s2.payment_id::text,jsonb_build_object('paymentId',s2.payment_id,'instructionId',s2.instruction_id,'lastStatus',s2.last_status,'attempts',s2.attempts,'inquiries',s2.inquiries,'createdAt',data.utc_text(s2.created_at))
  FROM payments.rail_submissions s2 JOIN payments.intents i ON i.id=s2.payment_id
  WHERE i.state NOT IN ('SETTLED','DECLINED','EXPIRED','BLOCKED') AND s2.created_at<=clock_timestamp()-data.threshold('PROVIDER_BACKLOG') ORDER BY s2.created_at LIMIT 100;
 RETURN QUERY SELECT 'RECONCILIATION_BREAK_AGED',b.id::text,jsonb_build_object('breakId',b.id,'paymentId',b.payment_id,'code',b.code,'firstSeenAt',data.utc_text(b.first_seen_at),'caseId',b.case_id)
  FROM payments.reconciliation_breaks b WHERE b.state='OPEN' AND b.first_seen_at<=clock_timestamp()-data.threshold('RECONCILIATION_BREAK_AGED') ORDER BY b.first_seen_at LIMIT 100;
 RETURN QUERY SELECT 'NOTIFICATION_FAILED',x.id::text,jsonb_build_object('notificationId',x.id,'template',x.template,'channel',x.channel,'attempts',x.attempts,'lastError',x.last_error)
  FROM data.notifications x WHERE x.state='FAILED' AND NOT x.failure_acknowledged LIMIT 100;
END $$;
CREATE FUNCTION data.quality_next_action(p_code text) RETURNS text LANGUAGE sql IMMUTABLE SET search_path=pg_catalog AS $$
 SELECT CASE p_code WHEN 'MISSING_FINANCIAL_EVENT' THEN 'CHECK_RELAY_THEN_REPAIR_PROJECTION' WHEN 'QUARANTINED_EVENT' THEN 'INSPECT_AND_REPLAY_OR_REJECT_EVENT'
  WHEN 'BROKEN_REFERENCE' THEN 'REPLAY_REFERENCE_EVENTS' WHEN 'STALE_PROJECTION' THEN 'RESTORE_CONSUMER_AND_WATCH_WATERMARK' WHEN 'UNDELIVERED_EVENTS' THEN 'RESTORE_EVENT_RELAY'
  WHEN 'PROVIDER_BACKLOG' THEN 'INQUIRE_PROVIDER_WITH_ORIGINAL_INSTRUCTION' WHEN 'RECONCILIATION_BREAK_AGED' THEN 'RESOLVE_RECONCILIATION_BREAK' ELSE 'RESTORE_CHANNEL_AND_REVIEW_MESSAGE' END $$;

-- Notifications must exist before the scan references their columns.
CREATE TABLE data.notification_purposes(code text PRIMARY KEY CHECK(code IN ('SECURITY','SERVICE','ACTIVITY','MARKETING')),mandatory boolean NOT NULL,default_enabled boolean NOT NULL,description text NOT NULL);
INSERT INTO data.notification_purposes VALUES
 ('SECURITY',true,true,'Sign-in and access changes. Always sent.'),
 ('SERVICE',true,true,'Statements and request outcomes. Always sent.'),
 ('ACTIVITY',false,true,'Payments leaving your account.'),
 ('MARKETING',false,false,'Product announcements.');
CREATE TABLE data.notification_preferences(person_id uuid NOT NULL,purpose text NOT NULL REFERENCES data.notification_purposes(code),enabled boolean NOT NULL,actor_context jsonb NOT NULL,updated_at timestamptz NOT NULL,PRIMARY KEY(person_id,purpose));
CREATE TABLE data.notification_preference_history(id uuid PRIMARY KEY DEFAULT gen_random_uuid(),person_id uuid NOT NULL,purpose text NOT NULL,enabled boolean NOT NULL,actor_context jsonb NOT NULL,recorded_at timestamptz NOT NULL DEFAULT clock_timestamp());
CREATE TRIGGER immutable_preference_history BEFORE UPDATE OR DELETE OR TRUNCATE ON data.notification_preference_history FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal();
CREATE TABLE data.notification_channels(channel text PRIMARY KEY CHECK(channel IN ('SYNTHETIC_EMAIL','IN_APP')),available boolean NOT NULL,actor_context jsonb,changed_at timestamptz NOT NULL DEFAULT clock_timestamp());
INSERT INTO data.notification_channels(channel,available) VALUES('SYNTHETIC_EMAIL',true),('IN_APP',true);
CREATE TABLE data.notification_channel_history(id uuid PRIMARY KEY DEFAULT gen_random_uuid(),channel text NOT NULL,available boolean NOT NULL,reason text NOT NULL,actor_context jsonb NOT NULL,recorded_at timestamptz NOT NULL DEFAULT clock_timestamp());
CREATE TRIGGER immutable_channel_history BEFORE UPDATE OR DELETE OR TRUNCATE ON data.notification_channel_history FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal();
CREATE TABLE data.notifications(
 id uuid PRIMARY KEY,dedupe_key text NOT NULL UNIQUE CHECK(length(dedupe_key) BETWEEN 1 AND 300),person_id uuid NOT NULL,purpose text NOT NULL REFERENCES data.notification_purposes(code),
 template text NOT NULL CHECK(template IN ('STATEMENT_READY','DATA_REQUEST_COMPLETED','PAYMENT_SENT','DEVELOPER_TOKEN_CREATED')),params jsonb NOT NULL,source_ref uuid NOT NULL,
 channel text NOT NULL REFERENCES data.notification_channels(channel),recipient text NOT NULL,state text NOT NULL CHECK(state IN ('PENDING','DELIVERED','FAILED','SUPPRESSED')),
 suppression_reason text,attempts integer NOT NULL DEFAULT 0 CHECK(attempts BETWEEN 0 AND 6),next_attempt_at timestamptz,last_error text,created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 delivered_at timestamptz,failure_acknowledged boolean NOT NULL DEFAULT false
);
CREATE INDEX notifications_due ON data.notifications(next_attempt_at) WHERE state='PENDING';
CREATE INDEX notifications_person ON data.notifications(person_id,created_at DESC);
CREATE TABLE data.notification_attempts(notification_id uuid NOT NULL REFERENCES data.notifications(id),attempt integer NOT NULL,channel text NOT NULL,outcome text NOT NULL CHECK(outcome IN ('DELIVERED','FAILED')),
 reason text NOT NULL,subject text NOT NULL,body text NOT NULL,attempted_at timestamptz NOT NULL DEFAULT clock_timestamp(),PRIMARY KEY(notification_id,attempt));
CREATE TRIGGER immutable_notification_attempts BEFORE UPDATE OR DELETE OR TRUNCATE ON data.notification_attempts FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal();
CREATE FUNCTION data.guard_notification() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN
 IF TG_OP<>'UPDATE' OR (to_jsonb(NEW)-ARRAY['state','attempts','next_attempt_at','last_error','delivered_at','failure_acknowledged']) IS DISTINCT FROM (to_jsonb(OLD)-ARRAY['state','attempts','next_attempt_at','last_error','delivered_at','failure_acknowledged'])
  OR OLD.state IN ('DELIVERED','SUPPRESSED') OR NEW.attempts<OLD.attempts OR (OLD.failure_acknowledged AND NOT NEW.failure_acknowledged) THEN RAISE EXCEPTION 'Notification identity and outcome are immutable'; END IF;RETURN NEW;END $$;
CREATE TRIGGER notification_guard BEFORE UPDATE OR DELETE ON data.notifications FOR EACH ROW EXECUTE FUNCTION data.guard_notification();
CREATE TRIGGER notification_truncate BEFORE TRUNCATE ON data.notifications FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal();

-- Opens owned alerts and cases for new findings; clears alerts whose condition no longer holds.
CREATE FUNCTION data.quality_scan() RETURNS jsonb
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,data AS $$
DECLARE f record;a data.quality_alerts;opened integer:=0;cleared integer:=0;target uuid;made uuid;seen text[]:='{}';
 actor jsonb:=jsonb_build_object('issuer','urn:piggybank:worker','subject','data-quality');BEGIN
 PERFORM pg_advisory_xact_lock(hashtextextended('data.quality-scan',0));
 FOR f IN SELECT * FROM data.quality_findings() LOOP
  seen:=seen||(f.check_code||'|'||f.subject);
  SELECT * INTO a FROM data.quality_alerts WHERE check_code=f.check_code AND subject=f.subject AND state='OPEN';
  IF FOUND THEN
   UPDATE data.quality_alerts SET last_detected_at=clock_timestamp(),observation=f.observation WHERE id=a.id AND observation IS DISTINCT FROM f.observation;
  ELSE
   target:=gen_random_uuid();
   made:=controls.open_case('DATA',target,coalesce((f.observation->>'batchId')::uuid,(f.observation->>'accountId')::uuid,(f.observation->>'paymentId')::uuid,(f.observation->>'notificationId')::uuid,target),
     'data-operations',data.quality_next_action(f.check_code),CASE WHEN f.check_code IN ('MISSING_FINANCIAL_EVENT','STALE_PROJECTION','UNDELIVERED_EVENTS') THEN 'HIGH' ELSE 'MEDIUM' END,actor);
   INSERT INTO data.quality_alerts VALUES(target,f.check_code,f.subject,'OPEN',data.quality_next_action(f.check_code),f.observation,made,clock_timestamp(),clock_timestamp(),NULL,NULL,NULL,NULL);
   INSERT INTO data.quality_alert_history VALUES(target,1,'OPEN',f.observation,clock_timestamp());
   opened:=opened+1;
  END IF;
 END LOOP;
 FOR a IN SELECT * FROM data.quality_alerts WHERE state='OPEN' AND NOT ((check_code||'|'||subject)=ANY(seen)) LOOP
  UPDATE data.quality_alerts SET state='CLEARED',cleared_at=clock_timestamp() WHERE id=a.id;
  INSERT INTO data.quality_alert_history VALUES(a.id,(SELECT max(version)+1 FROM data.quality_alert_history WHERE alert_id=a.id),'CLEARED',a.observation,clock_timestamp());
  cleared:=cleared+1;
 END LOOP;
 RETURN jsonb_build_object('opened',opened,'cleared',cleared,'open',(SELECT count(*) FROM data.quality_alerts WHERE state='OPEN'));
END $$;
CREATE FUNCTION data.require_staff(p_actor jsonb,p_person uuid,p_roles text[]) RETURNS void LANGUAGE sql VOLATILE SECURITY DEFINER SET search_path=pg_catalog AS
$$ SELECT controls.require_staff(p_actor,p_person,p_roles) $$;
-- Repair re-projects one missing journal batch from Core's immutable journal into the Data read model only.
CREATE FUNCTION data.repair_projection(p_actor jsonb,p_person uuid,p_batch uuid) RETURNS jsonb
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,data AS $$
DECLARE j core.journal_batches;origin uuid;n integer;target uuid:=gen_random_uuid();BEGIN
 PERFORM data.require_staff(p_actor,p_person,ARRAY['data_steward']);
 SELECT * INTO j FROM core.journal_batches WHERE id=p_batch;IF NOT FOUND THEN RAISE EXCEPTION 'Journal batch unavailable' USING ERRCODE='P0002'; END IF;
 SELECT event_id INTO origin FROM core.outbox WHERE aggregate_id=p_batch AND document->>'eventType'='journal.batch-posted';
 IF origin IS NULL THEN RAISE EXCEPTION 'No durable journal event exists for this batch; investigate Core before repair'; END IF;
 INSERT INTO data.transaction_views(batch_id,account_id,operation_id,document,origin_event)
 SELECT j.id,e.account_id,j.operation_id,jsonb_build_object('batchId',j.id,'accountId',e.account_id,'amount',jsonb_build_object('minorUnits',e.delta::text,'currency',jsonb_build_object('code','TST','scale',2)),'businessDate',j.business_date,
   'postingKind',j.posting_kind,'originalBatchId',j.original_batch,'status','POSTED','source','CORE_JOURNAL_REPAIR'),origin
 FROM core.journal_entries e WHERE e.batch_id=j.id ON CONFLICT DO NOTHING;
 GET DIAGNOSTICS n=ROW_COUNT;
 INSERT INTO data.projection_repairs VALUES(target,p_batch,p_actor,p_person,n,origin,clock_timestamp());
 RETURN jsonb_build_object('repairId',target,'batchId',p_batch,'rowsInserted',n,'originEvent',origin);
END $$;
CREATE FUNCTION data.acknowledge_quality_alert(p_actor jsonb,p_person uuid,p_alert uuid,p_note text) RETURNS jsonb
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,data AS $$
DECLARE a data.quality_alerts;BEGIN
 PERFORM data.require_staff(p_actor,p_person,ARRAY['data_steward']);
 SELECT * INTO a FROM data.quality_alerts WHERE id=p_alert FOR UPDATE;IF NOT FOUND THEN RAISE EXCEPTION 'Alert unavailable' USING ERRCODE='P0002'; END IF;
 IF a.acknowledged_at IS NOT NULL THEN RETURN jsonb_build_object('id',a.id,'acknowledged',false); END IF;
 IF length(trim(coalesce(p_note,''))) NOT BETWEEN 3 AND 500 THEN RAISE EXCEPTION 'Acknowledgement note required' USING ERRCODE='22023'; END IF;
 IF a.check_code='NOTIFICATION_FAILED' THEN UPDATE data.notifications SET failure_acknowledged=true WHERE id=a.subject::uuid; PERFORM data.quality_scan(); SELECT * INTO a FROM data.quality_alerts WHERE id=p_alert; END IF;
 IF a.state='OPEN' THEN RAISE EXCEPTION 'Condition persists; repair the source first'; END IF;
 UPDATE data.quality_alerts SET acknowledged_by=p_person,acknowledged_at=clock_timestamp(),acknowledgement=trim(p_note) WHERE id=a.id;
 UPDATE controls.cases SET state='CLOSED',next_action='COMPLETE',owner_person=coalesce(owner_person,p_person),version=version+1,updated_at=clock_timestamp() WHERE id=a.case_id AND state<>'CLOSED';
 IF FOUND THEN
  INSERT INTO controls.case_actions VALUES(gen_random_uuid(),a.case_id,p_actor->>'issuer',p_actor->>'subject','data-quality.acknowledged','CONDITION_CLEARED',clock_timestamp());
  PERFORM controls.case_changed(a.case_id,p_actor,'case.closed');
 END IF;
 RETURN jsonb_build_object('id',a.id,'acknowledged',true);
END $$;
CREATE FUNCTION data.quality_view(p_actor jsonb,p_person uuid) RETURNS jsonb
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,data AS $$
DECLARE newest_journal timestamptz;newest_projection timestamptz;oldest timestamptz;BEGIN
 PERFORM data.require_staff(p_actor,p_person,ARRAY['data_steward','case_reviewer','case_approver','operations_replay']);
 SELECT max(created_at) INTO newest_journal FROM core.journal_batches;
 SELECT max(recorded_at) INTO newest_projection FROM data.transaction_views;
 SELECT min(j.created_at) INTO oldest FROM core.journal_batches j WHERE j.created_at>clock_timestamp()-interval '7 days'
  AND EXISTS(SELECT 1 FROM core.journal_entries e JOIN core.account_registry a ON a.id=e.account_id WHERE e.batch_id=j.id) AND NOT EXISTS(SELECT 1 FROM data.transaction_views t WHERE t.batch_id=j.id);
 RETURN jsonb_build_object('asOf',data.utc_text(clock_timestamp()),
  'freshness',jsonb_build_object('projection','data.transaction_views','newestJournalAt',data.utc_text(newest_journal),'newestProjectedAt',data.utc_text(newest_projection),
   'oldestUnprojectedAt',data.utc_text(oldest),'lagSeconds',CASE WHEN oldest IS NULL THEN 0 ELSE floor(extract(epoch FROM clock_timestamp()-oldest))::bigint END,'objectiveSeconds',(SELECT seconds FROM data.quality_thresholds WHERE check_code='STALE_PROJECTION'),
   'status',CASE WHEN oldest IS NULL THEN 'CURRENT' WHEN oldest>clock_timestamp()-data.threshold('STALE_PROJECTION') THEN 'WITHIN_OBJECTIVE' ELSE 'STALE' END),
  'alerts',coalesce((SELECT jsonb_agg(jsonb_build_object('id',q.id,'checkCode',q.check_code,'subject',q.subject,'state',q.state,'nextAction',q.next_action,'observation',q.observation,'caseId',q.case_id,
    'caseState',c.state,'firstDetectedAt',data.utc_text(q.first_detected_at),'lastDetectedAt',data.utc_text(q.last_detected_at),'clearedAt',data.utc_text(q.cleared_at),'acknowledgedAt',data.utc_text(q.acknowledged_at),'acknowledgement',q.acknowledgement)
    ORDER BY (q.state='OPEN') DESC,q.first_detected_at DESC) FROM (SELECT * FROM data.quality_alerts ORDER BY (state='OPEN') DESC,first_detected_at DESC LIMIT 100) q JOIN controls.cases c ON c.id=q.case_id),'[]'::jsonb),
  'repairs',coalesce((SELECT jsonb_agg(jsonb_build_object('id',id,'batchId',batch_id,'rowsInserted',rows_inserted,'recordedAt',data.utc_text(recorded_at)) ORDER BY recorded_at DESC) FROM (SELECT * FROM data.projection_repairs ORDER BY recorded_at DESC LIMIT 20) r),'[]'::jsonb));
END $$;

-- ===== Notifications =====
CREATE FUNCTION data.person_party(p_person uuid) RETURNS uuid LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS
$$ SELECT p.id FROM controls.person_links l JOIN core.parties p ON p.issuer=l.issuer AND p.subject=l.subject WHERE l.person_id=p_person ORDER BY p.id LIMIT 1 $$;
CREATE FUNCTION data.masked_email(p_email text) RETURNS text LANGUAGE sql IMMUTABLE SET search_path=pg_catalog AS
$$ SELECT left(p_email,1)||'***@'||split_part(p_email,'@',2) $$;
CREATE FUNCTION data.purpose_enabled(p_person uuid,p_purpose text) RETURNS boolean LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS
$$ SELECT CASE WHEN p.mandatory THEN true ELSE coalesce((SELECT enabled FROM data.notification_preferences WHERE person_id=p_person AND purpose=p_purpose),p.default_enabled) END FROM data.notification_purposes p WHERE p.code=p_purpose $$;
-- Templates carry no amounts, balances, full identifiers, tokens or links with secrets.
CREATE FUNCTION data.render_notification(p_template text,p_params jsonb) RETURNS TABLE(subject text,body text) LANGUAGE sql IMMUTABLE SET search_path=pg_catalog AS $$
 SELECT CASE p_template WHEN 'STATEMENT_READY' THEN 'Your statement is ready' WHEN 'DATA_REQUEST_COMPLETED' THEN 'Your data request was processed'
   WHEN 'PAYMENT_SENT' THEN 'A payment left your account' ELSE 'A developer access token was created' END,
  CASE p_template WHEN 'STATEMENT_READY' THEN 'Your '||(p_params->>'month')||' statement for the account ending '||(p_params->>'accountEnding')||' is available. Sign in to Piggybank and open Statements & documents.'
   WHEN 'DATA_REQUEST_COMPLETED' THEN 'We finished processing your '||lower(p_params->>'kind')||' request. Sign in to Piggybank to see the outcome.'
   WHEN 'PAYMENT_SENT' THEN 'A payment from the account ending '||(p_params->>'accountEnding')||' was posted. Sign in to Piggybank to see the details. If you did not make it, contact support.'
   ELSE 'A read-only developer access token was created for your profile. If this was not you, sign in and revoke it.' END $$;
CREATE FUNCTION data.queue_notification(p_key text,p_person uuid,p_purpose text,p_template text,p_params jsonb,p_source uuid) RETURNS boolean
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,data AS $$
DECLARE contact text;enabled boolean:=data.purpose_enabled(p_person,p_purpose);made uuid;BEGIN
 SELECT email INTO contact FROM core.contacts WHERE party_id=data.person_party(p_person);
 INSERT INTO data.notifications(id,dedupe_key,person_id,purpose,template,params,source_ref,channel,recipient,state,suppression_reason,next_attempt_at)
 VALUES(gen_random_uuid(),p_key,p_person,p_purpose,p_template,p_params,p_source,CASE WHEN contact IS NULL THEN 'IN_APP' ELSE 'SYNTHETIC_EMAIL' END,
  CASE WHEN contact IS NULL THEN 'in-app inbox' ELSE data.masked_email(contact) END,CASE WHEN enabled THEN 'PENDING' ELSE 'SUPPRESSED' END,
  CASE WHEN enabled THEN NULL ELSE 'PURPOSE_PREFERENCE_DISABLED' END,CASE WHEN enabled THEN clock_timestamp() ELSE NULL END)
 ON CONFLICT(dedupe_key) DO NOTHING RETURNING id INTO made;
 RETURN made IS NOT NULL;
END $$;
-- Derivation is keyed by the economic or document identity, so duplicate events and replays send nothing twice.
CREATE FUNCTION data.derive_notifications(p_limit integer) RETURNS integer
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,data AS $$
DECLARE x record;made integer:=0;BEGIN
 IF p_limit NOT BETWEEN 1 AND 1000 THEN RAISE EXCEPTION 'Bounded notification derivation required'; END IF;
 FOR x IN SELECT v.id,v.account_id,v.period_start,o.person FROM documents.statement_versions v CROSS JOIN LATERAL unnest(v.owners) o(person)
  WHERE v.created_at>clock_timestamp()-interval '7 days' AND NOT EXISTS(SELECT 1 FROM data.notifications n WHERE n.dedupe_key='STATEMENT_READY:'||v.id||':'||o.person) ORDER BY v.created_at LIMIT p_limit LOOP
  IF data.queue_notification('STATEMENT_READY:'||x.id||':'||x.person,x.person,'SERVICE','STATEMENT_READY',
   jsonb_build_object('month',to_char(x.period_start,'FMMonth YYYY'),'accountEnding',right(x.account_id::text,4)),x.id) THEN made:=made+1; END IF;
 END LOOP;
 FOR x IN SELECT r.id,r.person_id,r.kind FROM documents.data_request_outcomes o JOIN documents.data_requests r ON r.id=o.request_id
  WHERE o.processed_at>clock_timestamp()-interval '7 days' AND NOT EXISTS(SELECT 1 FROM data.notifications n WHERE n.dedupe_key='DATA_REQUEST:'||r.id) LIMIT p_limit LOOP
  IF data.queue_notification('DATA_REQUEST:'||x.id,x.person_id,'SERVICE','DATA_REQUEST_COMPLETED',jsonb_build_object('kind',x.kind),x.id) THEN made:=made+1; END IF;
 END LOOP;
 FOR x IN SELECT t.batch_id,t.account_id,rel.person_id FROM data.transaction_views t JOIN core.journal_batches j ON j.id=t.batch_id JOIN core.journal_entries e ON e.batch_id=t.batch_id AND e.account_id=t.account_id
  JOIN core.account_relationships rel ON rel.account_id=t.account_id AND rel.kind='OWNER' AND rel.revoked_at IS NULL
  WHERE t.recorded_at>clock_timestamp()-interval '7 days' AND j.posting_kind IN ('INTERNAL_PAYMENT','EXTERNAL_PAYMENT') AND e.delta<0
   AND NOT EXISTS(SELECT 1 FROM data.notifications n WHERE n.dedupe_key='PAYMENT_SENT:'||t.batch_id||':'||t.account_id||':'||rel.person_id) ORDER BY t.recorded_at LIMIT p_limit LOOP
  IF data.queue_notification('PAYMENT_SENT:'||x.batch_id||':'||x.account_id||':'||x.person_id,x.person_id,'ACTIVITY','PAYMENT_SENT',jsonb_build_object('accountEnding',right(x.account_id::text,4)),x.batch_id) THEN made:=made+1; END IF;
 END LOOP;
 FOR x IN SELECT k.id,k.person_id FROM core.consumer_tokens k WHERE k.created_at>clock_timestamp()-interval '7 days' AND NOT EXISTS(SELECT 1 FROM data.notifications n WHERE n.dedupe_key='TOKEN:'||k.id) LIMIT p_limit LOOP
  IF data.queue_notification('TOKEN:'||x.id,x.person_id,'SECURITY','DEVELOPER_TOKEN_CREATED','{}'::jsonb,x.id) THEN made:=made+1; END IF;
 END LOOP;
 RETURN made;
END $$;
-- Synthetic capture: delivery is recorded, never sent. An unavailable channel fails the attempt and backs off (30 s × 2^n, six attempts).
CREATE FUNCTION data.deliver_notifications(p_limit integer) RETURNS jsonb
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,data AS $$
DECLARE x data.notifications;up boolean;r record;delivered integer:=0;failed integer:=0;BEGIN
 IF p_limit NOT BETWEEN 1 AND 1000 THEN RAISE EXCEPTION 'Bounded delivery required'; END IF;
 FOR x IN SELECT * FROM data.notifications WHERE state='PENDING' AND next_attempt_at<=clock_timestamp() ORDER BY next_attempt_at,id LIMIT p_limit FOR UPDATE SKIP LOCKED LOOP
  SELECT available INTO up FROM data.notification_channels WHERE channel=x.channel;
  SELECT * INTO r FROM data.render_notification(x.template,x.params);
  IF up THEN
   INSERT INTO data.notification_attempts VALUES(x.id,x.attempts+1,x.channel,'DELIVERED','SYNTHETIC_CAPTURE',r.subject,r.body,clock_timestamp());
   UPDATE data.notifications SET state='DELIVERED',attempts=attempts+1,delivered_at=clock_timestamp(),next_attempt_at=NULL,last_error=NULL WHERE id=x.id;
   delivered:=delivered+1;
  ELSE
   INSERT INTO data.notification_attempts VALUES(x.id,x.attempts+1,x.channel,'FAILED','CHANNEL_UNAVAILABLE',r.subject,r.body,clock_timestamp());
   UPDATE data.notifications SET attempts=attempts+1,last_error='CHANNEL_UNAVAILABLE',state=CASE WHEN attempts+1>=6 THEN 'FAILED' ELSE 'PENDING' END,
    next_attempt_at=CASE WHEN attempts+1>=6 THEN NULL ELSE clock_timestamp()+make_interval(secs=>30*power(2,attempts)) END WHERE id=x.id;
   failed:=failed+1;
  END IF;
 END LOOP;
 RETURN jsonb_build_object('delivered',delivered,'failed',failed);
END $$;
CREATE FUNCTION data.customer_messages(p_actor jsonb,p_person uuid) RETURNS jsonb
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,data AS $$ BEGIN
 PERFORM documents.require_periodic_actor(p_actor,p_person,false);
 RETURN jsonb_build_object(
  'purposes',(SELECT jsonb_agg(jsonb_build_object('code',p.code,'mandatory',p.mandatory,'description',p.description,'enabled',data.purpose_enabled(p_person,p.code)) ORDER BY p.code) FROM data.notification_purposes p),
  'messages',coalesce((SELECT jsonb_agg(jsonb_build_object('id',n.id,'purpose',n.purpose,'channel',n.channel,'recipient',n.recipient,'state',n.state,'createdAt',data.utc_text(n.created_at),
    'deliveredAt',data.utc_text(n.delivered_at),'subject',r.subject,'body',r.body) ORDER BY n.created_at DESC)
    FROM (SELECT * FROM data.notifications WHERE person_id=p_person AND state IN ('PENDING','DELIVERED','FAILED') ORDER BY created_at DESC LIMIT 50) n CROSS JOIN LATERAL data.render_notification(n.template,n.params) r),'[]'::jsonb));
END $$;
CREATE FUNCTION data.set_notification_preference(p_actor jsonb,p_person uuid,p_purpose text,p_enabled boolean) RETURNS jsonb
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,data AS $$
DECLARE p data.notification_purposes;BEGIN
 PERFORM documents.require_periodic_actor(p_actor,p_person,false);
 SELECT * INTO p FROM data.notification_purposes WHERE code=p_purpose;IF NOT FOUND THEN RAISE EXCEPTION 'Unknown purpose' USING ERRCODE='22023'; END IF;
 IF p.mandatory AND NOT p_enabled THEN RAISE EXCEPTION 'Mandatory notices cannot be disabled'; END IF;
 INSERT INTO data.notification_preferences VALUES(p_person,p_purpose,p_enabled,p_actor,clock_timestamp()) ON CONFLICT(person_id,purpose) DO UPDATE SET enabled=EXCLUDED.enabled,actor_context=EXCLUDED.actor_context,updated_at=EXCLUDED.updated_at;
 INSERT INTO data.notification_preference_history(person_id,purpose,enabled,actor_context) VALUES(p_person,p_purpose,p_enabled,p_actor);
 RETURN data.customer_messages(p_actor,p_person);
END $$;
CREATE FUNCTION data.set_notification_channel(p_actor jsonb,p_person uuid,p_channel text,p_available boolean,p_reason text) RETURNS jsonb
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,data AS $$ BEGIN
 PERFORM data.require_staff(p_actor,p_person,ARRAY['operations_replay']);
 IF length(trim(coalesce(p_reason,''))) NOT BETWEEN 3 AND 500 THEN RAISE EXCEPTION 'Reason required' USING ERRCODE='22023'; END IF;
 UPDATE data.notification_channels SET available=p_available,actor_context=p_actor,changed_at=clock_timestamp() WHERE channel=p_channel;
 IF NOT FOUND THEN RAISE EXCEPTION 'Unknown channel' USING ERRCODE='22023'; END IF;
 INSERT INTO data.notification_channel_history(channel,available,reason,actor_context) VALUES(p_channel,p_available,trim(p_reason),p_actor);
 RETURN data.staff_notifications(p_actor,p_person);
END $$;
CREATE FUNCTION data.staff_notifications(p_actor jsonb,p_person uuid) RETURNS jsonb
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,data AS $$ BEGIN
 PERFORM data.require_staff(p_actor,p_person,ARRAY['operations_replay','data_steward','case_reviewer','case_approver']);
 RETURN jsonb_build_object(
  'channels',(SELECT jsonb_agg(jsonb_build_object('channel',channel,'available',available,'changedAt',data.utc_text(changed_at)) ORDER BY channel) FROM data.notification_channels),
  'counts',coalesce((SELECT jsonb_object_agg(state,c) FROM (SELECT state,count(*) c FROM data.notifications GROUP BY state) x),'{}'::jsonb),
  'recent',coalesce((SELECT jsonb_agg(jsonb_build_object('id',n.id,'template',n.template,'purpose',n.purpose,'channel',n.channel,'recipient',n.recipient,'state',n.state,'attempts',n.attempts,
    'suppressionReason',n.suppression_reason,'lastError',n.last_error,'createdAt',data.utc_text(n.created_at),'deliveredAt',data.utc_text(n.delivered_at)) ORDER BY n.created_at DESC)
    FROM (SELECT * FROM data.notifications ORDER BY created_at DESC LIMIT 50) n),'[]'::jsonb));
END $$;

-- ===== Operations snapshot and end-to-end correlation (FR-OPS-02) =====
CREATE FUNCTION controls.operations_snapshot(p_actor jsonb,p_person uuid) RETURNS jsonb
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,controls AS $$
DECLARE counts jsonb;target uuid:=gen_random_uuid();stamp timestamptz:=clock_timestamp();BEGIN
 PERFORM controls.require_staff(p_actor,p_person,ARRAY['case_reviewer','case_approver','operations_replay','payment_inquiry','finance_adjuster','finance_approver','data_steward']);
 counts:=controls.operations_counts()||jsonb_build_object(
  'dataQuality',jsonb_build_object('openAlerts',(SELECT count(*) FROM data.quality_alerts WHERE state='OPEN'),
   'openByCheck',coalesce((SELECT jsonb_object_agg(check_code,c) FROM (SELECT check_code,count(*) c FROM data.quality_alerts WHERE state='OPEN' GROUP BY check_code) x),'{}'::jsonb)),
  'notifications',coalesce((SELECT jsonb_object_agg(state,c) FROM (SELECT state,count(*) c FROM data.notifications GROUP BY state) x),'{}'::jsonb));
 INSERT INTO controls.operations_snapshots VALUES(target,p_actor,p_person,stamp,counts,encode(sha256(convert_to(core.periodic_policy_canonical(counts),'UTF8')),'hex'),clock_timestamp());
 RETURN jsonb_build_object('id',target,'asOf',controls.utc_text(stamp),'counts',counts,'digest',encode(sha256(convert_to(core.periodic_policy_canonical(counts),'UTF8')),'hex'),
  'overdueCases',coalesce((SELECT jsonb_agg(jsonb_build_object('id',id,'kind',kind,'queue',queue,'severity',severity,'nextAction',next_action,'deadline',controls.utc_text(deadline)) ORDER BY deadline)
    FROM (SELECT * FROM controls.cases WHERE state<>'CLOSED' AND deadline<clock_timestamp() ORDER BY deadline LIMIT 20) c),'[]'::jsonb));
END $$;
-- Durable identifiers only; no names, emails or identity evidence.
CREATE FUNCTION controls.operation_trace(p_actor jsonb,p_person uuid,p_id uuid) RETURNS jsonb
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,controls AS $$
DECLARE s text;events jsonb:='[]';part jsonb;BEGIN
 PERFORM controls.require_staff(p_actor,p_person,ARRAY['case_reviewer','case_approver','operations_replay','payment_inquiry','data_steward']);
 FOREACH s IN ARRAY ARRAY['platform','core','controls','journeys','payments','finance','documents','data'] LOOP
  EXECUTE format('SELECT coalesce(jsonb_agg(jsonb_build_object(''module'',%L,''eventId'',event_id,''eventType'',document->>''eventType'',''aggregateId'',aggregate_id,''aggregateVersion'',aggregate_version,''correlationId'',document->>''correlationId'',''createdAt'',created_at,''deliveredAt'',delivered_at) ORDER BY created_at),''[]''::jsonb) FROM (SELECT * FROM %I.outbox WHERE aggregate_id=$1 OR document->>''correlationId''=$1::text OR document->''data''->>''operationId''=$1::text ORDER BY created_at LIMIT 50) o',s,s)
   INTO part USING p_id;
  events:=events||part;
 END LOOP;
 RETURN jsonb_build_object('id',p_id,'asOf',controls.utc_text(clock_timestamp()),
  'journal',coalesce((SELECT jsonb_agg(jsonb_build_object('batchId',j.id,'operationId',j.operation_id,'postingKind',j.posting_kind,'businessDate',j.business_date,'originalBatchId',j.original_batch,'createdAt',controls.utc_text(j.created_at),
    'legs',(SELECT jsonb_agg(jsonb_build_object('accountId',e.account_id,'deltaMinor',e.delta::text) ORDER BY e.account_id) FROM core.journal_entries e WHERE e.batch_id=j.id),
    'projected',EXISTS(SELECT 1 FROM data.transaction_views t WHERE t.batch_id=j.id)) ORDER BY j.created_at) FROM core.journal_batches j WHERE j.id=p_id OR j.operation_id=p_id OR j.original_batch=p_id),'[]'::jsonb),
  'payment',(SELECT jsonb_build_object('paymentId',i.id,'state',i.state,'operationId',i.operation_id,'batchId',i.batch_id,'reasonCode',i.reason_code,'createdAt',controls.utc_text(i.created_at),
    'instructionId',(SELECT instruction_id FROM payments.rail_submissions WHERE payment_id=i.id),'providerStatus',(SELECT last_status FROM payments.rail_submissions WHERE payment_id=i.id),
    'receipts',(SELECT count(*) FROM payments.rail_receipts WHERE payment_id=i.id)) FROM payments.intents i WHERE i.id=p_id OR i.operation_id=p_id OR i.batch_id=p_id LIMIT 1),
  'events',events,
  'cases',coalesce((SELECT jsonb_agg(jsonb_build_object('caseId',c.id,'kind',c.kind,'state',c.state,'queue',c.queue,'nextAction',c.next_action) ORDER BY c.created_at) FROM controls.cases c WHERE c.resource_id=p_id OR c.origin_id=p_id OR c.id=p_id),'[]'::jsonb),
  'monitoring',coalesce((SELECT jsonb_agg(jsonb_build_object('alertId',a.id,'caseId',a.case_id,'anchorOperation',a.anchor_operation)) FROM controls.monitoring_alerts a WHERE a.anchor_operation=p_id OR a.trigger_operation=p_id OR a.account_id=p_id OR a.id=p_id),'[]'::jsonb),
  'notifications',coalesce((SELECT jsonb_agg(jsonb_build_object('id',n.id,'template',n.template,'state',n.state,'channel',n.channel)) FROM data.notifications n WHERE n.source_ref=p_id),'[]'::jsonb));
END $$;

DO $$ DECLARE t text;BEGIN
 FOREACH t IN ARRAY ARRAY['monitoring_rules','monitoring_evaluations','monitoring_alerts','monitoring_dispositions','monitoring_scans','operations_snapshots'] LOOP EXECUTE format('REVOKE ALL ON controls.%I FROM bank_controls',t); END LOOP;
 FOREACH t IN ARRAY ARRAY['quality_thresholds','quality_alerts','quality_alert_history','projection_repairs','notification_purposes','notification_preferences','notification_preference_history','notification_channels','notification_channel_history','notifications','notification_attempts'] LOOP EXECUTE format('REVOKE ALL ON data.%I FROM bank_data',t); END LOOP;
END $$;
GRANT EXECUTE ON FUNCTION controls.monitoring_tick(integer),controls.monitoring_scan(jsonb,uuid,timestamptz,timestamptz),controls.monitoring_alerts_view(jsonb,uuid),
 controls.propose_monitoring_disposition(jsonb,uuid,uuid,text,text),controls.approve_monitoring_disposition(jsonb,uuid,uuid),controls.operations_snapshot(jsonb,uuid),
 controls.operation_trace(jsonb,uuid,uuid),controls.operations_counts() TO bank_controls;
GRANT EXECUTE ON FUNCTION data.quality_scan(),data.repair_projection(jsonb,uuid,uuid),data.acknowledge_quality_alert(jsonb,uuid,uuid,text),data.quality_view(jsonb,uuid),
 data.derive_notifications(integer),data.deliver_notifications(integer),data.customer_messages(jsonb,uuid),data.set_notification_preference(jsonb,uuid,text,boolean),
 data.set_notification_channel(jsonb,uuid,text,boolean,text),data.staff_notifications(jsonb,uuid) TO bank_data;
