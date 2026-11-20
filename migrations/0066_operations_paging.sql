-- owner: controls. WP-29 on-call paging (NFR-10) and the operations metrics snapshot.
-- Pages are opened from owned alerts and escalated cases, delivered by synthetic capture, acknowledged by the
-- person paged and escalated to the secondary when unacknowledged. Nothing here changes a financial record.

CREATE TABLE controls.paging_policy(id boolean PRIMARY KEY DEFAULT true CHECK(id),ack_seconds integer NOT NULL CHECK(ack_seconds BETWEEN 0 AND 3600),label text NOT NULL CHECK(label='SYNTHETIC_FIXTURE'));
INSERT INTO controls.paging_policy VALUES(true,300,'SYNTHETIC_FIXTURE');
CREATE TABLE controls.oncall_roster(
 person_id uuid PRIMARY KEY,tier text NOT NULL CHECK(tier IN ('PRIMARY','SECONDARY')),active boolean NOT NULL DEFAULT true,
 actor_context jsonb NOT NULL,updated_at timestamptz NOT NULL DEFAULT clock_timestamp()
);
CREATE UNIQUE INDEX oncall_one_per_tier ON controls.oncall_roster(tier) WHERE active;
CREATE TABLE controls.oncall_history(id uuid PRIMARY KEY DEFAULT gen_random_uuid(),person_id uuid NOT NULL,tier text NOT NULL,active boolean NOT NULL,actor_context jsonb NOT NULL,recorded_at timestamptz NOT NULL DEFAULT clock_timestamp());
CREATE TRIGGER immutable_oncall_history BEFORE UPDATE OR DELETE OR TRUNCATE ON controls.oncall_history FOR EACH STATEMENT EXECUTE FUNCTION controls.immutable_check_result();
CREATE TABLE controls.pages(
 id uuid PRIMARY KEY,source_kind text NOT NULL CHECK(source_kind IN ('DATA_QUALITY','MONITORING','OVERDUE_CASE')),source_id uuid NOT NULL,case_id uuid,
 severity text NOT NULL CHECK(severity IN ('HIGH','CRITICAL')),summary text NOT NULL CHECK(length(summary) BETWEEN 3 AND 200),
 detected_at timestamptz NOT NULL,opened_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 primary_person uuid,delivered_at timestamptz,escalated_at timestamptz,secondary_person uuid,
 acknowledged_at timestamptz,acknowledged_by uuid,UNIQUE(source_kind,source_id),
 CHECK(acknowledged_by IS NULL OR acknowledged_by IN (primary_person,secondary_person))
);
CREATE TABLE controls.page_deliveries(page_id uuid NOT NULL REFERENCES controls.pages(id),tier text NOT NULL CHECK(tier IN ('PRIMARY','SECONDARY')),person_id uuid NOT NULL,
 channel text NOT NULL CHECK(channel='SYNTHETIC_PAGER'),delivered_at timestamptz NOT NULL DEFAULT clock_timestamp(),PRIMARY KEY(page_id,tier));
CREATE TRIGGER immutable_page_deliveries BEFORE UPDATE OR DELETE OR TRUNCATE ON controls.page_deliveries FOR EACH STATEMENT EXECUTE FUNCTION controls.immutable_check_result();
CREATE FUNCTION controls.guard_page() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN
 IF TG_OP<>'UPDATE' OR (to_jsonb(NEW)-ARRAY['primary_person','delivered_at','escalated_at','secondary_person','acknowledged_at','acknowledged_by']) IS DISTINCT FROM (to_jsonb(OLD)-ARRAY['primary_person','delivered_at','escalated_at','secondary_person','acknowledged_at','acknowledged_by'])
  OR (OLD.acknowledged_at IS NOT NULL AND to_jsonb(NEW) IS DISTINCT FROM to_jsonb(OLD)) OR (OLD.delivered_at IS NOT NULL AND NEW.delivered_at IS DISTINCT FROM OLD.delivered_at)
  OR (OLD.escalated_at IS NOT NULL AND NEW.escalated_at IS DISTINCT FROM OLD.escalated_at) THEN RAISE EXCEPTION 'Page history is append-only'; END IF;RETURN NEW;END $$;
CREATE TRIGGER page_guard BEFORE UPDATE OR DELETE ON controls.pages FOR EACH ROW EXECUTE FUNCTION controls.guard_page();
CREATE TRIGGER page_truncate BEFORE TRUNCATE ON controls.pages FOR EACH STATEMENT EXECUTE FUNCTION controls.immutable_check_result();
CREATE INDEX pages_open ON controls.pages(opened_at) WHERE acknowledged_at IS NULL;

-- Critical sources: high-severity data-quality alerts, open monitoring alerts, escalated overdue cases.
CREATE FUNCTION controls.page_tick() RETURNS jsonb LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,controls AS $$
DECLARE x record;target uuid;primary_on uuid;secondary_on uuid;opened integer:=0;escalated integer:=0;ack interval;BEGIN
 PERFORM pg_advisory_xact_lock(hashtextextended('controls.page-tick',0));
 SELECT make_interval(secs=>ack_seconds) INTO ack FROM controls.paging_policy;
 SELECT person_id INTO primary_on FROM controls.oncall_roster WHERE tier='PRIMARY' AND active;
 SELECT person_id INTO secondary_on FROM controls.oncall_roster WHERE tier='SECONDARY' AND active;
 FOR x IN
  SELECT 'DATA_QUALITY' AS kind,q.id AS source,q.case_id,'HIGH' AS severity,'Data quality: '||lower(replace(q.check_code,'_',' '))||' — '||lower(replace(q.next_action,'_',' ')) AS summary,q.first_detected_at AS detected
   FROM data.quality_alerts q JOIN controls.cases c ON c.id=q.case_id WHERE q.state='OPEN' AND c.severity IN ('HIGH','CRITICAL')
  UNION ALL
  SELECT 'MONITORING',a.id,a.case_id,'HIGH','Transaction monitoring alert needs investigation',a.created_at
   FROM controls.monitoring_alerts a JOIN controls.cases c ON c.id=a.case_id WHERE c.state<>'CLOSED'
  UNION ALL
  SELECT 'OVERDUE_CASE',c.id,c.id,'CRITICAL','Overdue '||lower(c.kind)||' case escalated in '||c.queue,c.escalated_at
   FROM controls.cases c WHERE c.state<>'CLOSED' AND c.escalated_at IS NOT NULL
 LOOP
  IF NOT EXISTS(SELECT 1 FROM controls.pages p WHERE p.source_kind=x.kind AND p.source_id=x.source) THEN
   target:=gen_random_uuid();
   INSERT INTO controls.pages(id,source_kind,source_id,case_id,severity,summary,detected_at,primary_person,delivered_at)
   VALUES(target,x.kind,x.source,x.case_id,x.severity,left(x.summary,200),x.detected,coalesce(primary_on,secondary_on),CASE WHEN coalesce(primary_on,secondary_on) IS NULL THEN NULL ELSE clock_timestamp() END);
   IF coalesce(primary_on,secondary_on) IS NOT NULL THEN
    INSERT INTO controls.page_deliveries(page_id,tier,person_id,channel) VALUES(target,CASE WHEN primary_on IS NULL THEN 'SECONDARY' ELSE 'PRIMARY' END,coalesce(primary_on,secondary_on),'SYNTHETIC_PAGER');
   END IF;
   opened:=opened+1;
  END IF;
 END LOOP;
 -- Unacknowledged after the policy window: escalate once to the secondary.
 FOR x IN SELECT * FROM controls.pages WHERE acknowledged_at IS NULL AND escalated_at IS NULL AND opened_at<=clock_timestamp()-ack ORDER BY opened_at FOR UPDATE LOOP
  IF secondary_on IS NOT NULL AND secondary_on IS DISTINCT FROM x.primary_person THEN
   UPDATE controls.pages SET escalated_at=clock_timestamp(),secondary_person=secondary_on,delivered_at=coalesce(delivered_at,clock_timestamp()) WHERE id=x.id;
   INSERT INTO controls.page_deliveries(page_id,tier,person_id,channel) VALUES(x.id,'SECONDARY',secondary_on,'SYNTHETIC_PAGER') ON CONFLICT DO NOTHING;
   escalated:=escalated+1;
  END IF;
 END LOOP;
 RETURN jsonb_build_object('opened',opened,'escalated',escalated);
END $$;
CREATE FUNCTION controls.acknowledge_page(p_actor jsonb,p_person uuid,p_page uuid) RETURNS jsonb LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,controls AS $$
DECLARE p controls.pages;BEGIN
 IF p_actor->>'trust' IS DISTINCT FROM 'WORKFORCE' THEN RAISE EXCEPTION 'Current trust context required'; END IF;
 PERFORM 1 FROM controls.person_links WHERE issuer=p_actor->>'issuer' AND subject=p_actor->>'subject' AND person_id=p_person;
 IF NOT FOUND THEN RAISE EXCEPTION 'Current canonical identity required'; END IF;
 SELECT * INTO p FROM controls.pages WHERE id=p_page FOR UPDATE;IF NOT FOUND THEN RAISE EXCEPTION 'Page unavailable' USING ERRCODE='P0002'; END IF;
 IF p.acknowledged_at IS NOT NULL THEN
  IF p.acknowledged_by=p_person THEN RETURN jsonb_build_object('id',p.id,'acknowledged',false); END IF;
  RAISE EXCEPTION 'Page already acknowledged';
 END IF;
 IF p_person IS DISTINCT FROM p.primary_person AND p_person IS DISTINCT FROM p.secondary_person THEN RAISE EXCEPTION 'Only the person paged can acknowledge'; END IF;
 UPDATE controls.pages SET acknowledged_at=clock_timestamp(),acknowledged_by=p_person WHERE id=p_page;
 PERFORM controls.append_audit(gen_random_uuid(),'paging',jsonb_build_object('actorIssuer',p_actor->>'issuer','actorSubject',p_actor->>'subject','actorPersonId',p_person,'representedParty',NULL,
  'action','page.acknowledged','resource',p_page::text,'outcome','RECORDED','policyVersion','synthetic-v1','reasonCode',p.source_kind,'correlationId',p_page,'occurredAt',controls.utc_text(clock_timestamp())));
 RETURN jsonb_build_object('id',p_page,'acknowledged',true);
END $$;
CREATE FUNCTION controls.set_oncall(p_actor jsonb,p_person uuid,p_target uuid,p_tier text,p_active boolean) RETURNS jsonb LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,controls AS $$ BEGIN
 PERFORM controls.require_staff(p_actor,p_person,ARRAY['operations_replay','security_admin']);
 IF p_tier NOT IN ('PRIMARY','SECONDARY') THEN RAISE EXCEPTION 'Supported tier required' USING ERRCODE='22023'; END IF;
 IF NOT EXISTS(SELECT 1 FROM controls.role_grants WHERE person_id=p_target AND revoked_at IS NULL AND valid_until>clock_timestamp()) THEN RAISE EXCEPTION 'On-call person needs a current workforce role' USING ERRCODE='22023'; END IF;
 IF p_active THEN UPDATE controls.oncall_roster SET active=false,actor_context=p_actor,updated_at=clock_timestamp() WHERE tier=p_tier AND active AND person_id<>p_target; END IF;
 INSERT INTO controls.oncall_roster VALUES(p_target,p_tier,p_active,p_actor,clock_timestamp()) ON CONFLICT(person_id) DO UPDATE SET tier=EXCLUDED.tier,active=EXCLUDED.active,actor_context=EXCLUDED.actor_context,updated_at=EXCLUDED.updated_at;
 INSERT INTO controls.oncall_history(person_id,tier,active,actor_context) VALUES(p_target,p_tier,p_active,p_actor);
 RETURN controls.pages_view(p_actor,p_person);
END $$;
CREATE FUNCTION controls.pages_view(p_actor jsonb,p_person uuid) RETURNS jsonb LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,controls AS $$ BEGIN
 PERFORM controls.require_staff(p_actor,p_person,ARRAY['case_reviewer','case_approver','operations_replay','data_steward','security_admin','security_approver','payment_inquiry']);
 RETURN jsonb_build_object('ackSeconds',(SELECT ack_seconds FROM controls.paging_policy),
  'roster',coalesce((SELECT jsonb_agg(jsonb_build_object('personId',person_id,'tier',tier,'active',active,'you',person_id=p_person) ORDER BY tier) FROM controls.oncall_roster WHERE active),'[]'::jsonb),
  'pages',coalesce((SELECT jsonb_agg(jsonb_build_object('id',p.id,'sourceKind',p.source_kind,'caseId',p.case_id,'severity',p.severity,'summary',p.summary,'detectedAt',controls.utc_text(p.detected_at),
    'openedAt',controls.utc_text(p.opened_at),'deliveredAt',controls.utc_text(p.delivered_at),'escalatedAt',controls.utc_text(p.escalated_at),'acknowledgedAt',controls.utc_text(p.acknowledged_at),
    'acknowledgedBy',p.acknowledged_by,'pagedYou',p_person IN (p.primary_person,p.secondary_person),
    'deliverySeconds',CASE WHEN p.delivered_at IS NULL THEN NULL ELSE round(extract(epoch FROM p.delivered_at-p.detected_at)::numeric,1) END) ORDER BY (p.acknowledged_at IS NULL) DESC,p.opened_at DESC)
    FROM (SELECT * FROM controls.pages ORDER BY (acknowledged_at IS NULL) DESC,opened_at DESC LIMIT 50) p),'[]'::jsonb));
END $$;

-- Gauges for the protected /metrics endpoint. Counts only; no identifiers or personal data.
CREATE FUNCTION controls.metrics_snapshot() RETURNS jsonb LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,controls AS $$
DECLARE s text;n bigint;oldest timestamptz;outbox jsonb:='{}';dead jsonb:='{}';lag bigint;BEGIN
 FOREACH s IN ARRAY ARRAY['platform','identity','core','controls','journeys','payments','finance','documents','data','simulator'] LOOP
  EXECUTE format('SELECT count(*),min(created_at) FROM %I.outbox WHERE delivered_at IS NULL',s) INTO n,oldest;
  outbox:=outbox||jsonb_build_object(s,jsonb_build_object('undelivered',n,'oldestAgeSeconds',coalesce(floor(extract(epoch FROM clock_timestamp()-oldest)),0)));
  EXECUTE format('SELECT count(*) FROM %I.dead_letters WHERE state=''PENDING''',s) INTO n;
  dead:=dead||jsonb_build_object(s,n);
 END LOOP;
 SELECT coalesce(floor(extract(epoch FROM clock_timestamp()-min(j.created_at))),0) INTO lag FROM core.journal_batches j WHERE j.created_at>clock_timestamp()-interval '7 days'
  AND EXISTS(SELECT 1 FROM core.journal_entries e JOIN core.account_registry a ON a.id=e.account_id WHERE e.batch_id=j.id) AND NOT EXISTS(SELECT 1 FROM data.transaction_views t WHERE t.batch_id=j.id);
 RETURN jsonb_build_object('outbox',outbox,'deadLetters',dead,
  'paymentsAwaiting',(SELECT count(*) FROM payments.intents WHERE state IN ('ACCEPTED','CONTROLS_PENDING','CONTROL_REVIEW')),
  'casesOpen',(SELECT count(*) FROM controls.cases WHERE state<>'CLOSED'),
  'casesOverdue',(SELECT count(*) FROM controls.cases WHERE state<>'CLOSED' AND deadline<clock_timestamp()),
  'qualityAlertsOpen',(SELECT count(*) FROM data.quality_alerts WHERE state='OPEN'),
  'monitoringAlertsOpen',(SELECT count(*) FROM controls.monitoring_alerts a JOIN controls.cases c ON c.id=a.case_id WHERE c.state<>'CLOSED'),
  'notificationsPending',(SELECT count(*) FROM data.notifications WHERE state='PENDING'),
  'notificationsFailed',(SELECT count(*) FROM data.notifications WHERE state='FAILED'),
  'projectionLagSeconds',lag,
  'pagesUnacknowledged',(SELECT count(*) FROM controls.pages WHERE acknowledged_at IS NULL),
  'pagesUndelivered',(SELECT count(*) FROM controls.pages WHERE delivered_at IS NULL),
  'oldestUnacknowledgedPageSeconds',(SELECT coalesce(floor(extract(epoch FROM clock_timestamp()-min(opened_at))),0) FROM controls.pages WHERE acknowledged_at IS NULL),
  'journalBatches',(SELECT count(*) FROM core.journal_batches),
  'unbalancedBatches',(SELECT count(*) FROM (SELECT batch_id FROM core.journal_entries GROUP BY batch_id HAVING sum(delta::numeric)<>0) u));
END $$;

DO $$ DECLARE t text;BEGIN FOREACH t IN ARRAY ARRAY['paging_policy','oncall_roster','oncall_history','pages','page_deliveries'] LOOP EXECUTE format('REVOKE ALL ON controls.%I FROM bank_controls',t); END LOOP; END $$;
GRANT EXECUTE ON FUNCTION controls.page_tick(),controls.acknowledge_page(jsonb,uuid,uuid),controls.set_oncall(jsonb,uuid,uuid,text,boolean),controls.pages_view(jsonb,uuid),controls.metrics_snapshot() TO bank_controls;
