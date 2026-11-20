-- Editable recurring instructions retain their previous consent, financial history and family identity.
ALTER TABLE core.consumer_schedules ADD COLUMN family_id uuid;
ALTER TABLE core.consumer_schedules ADD COLUMN anchor_day integer;
UPDATE core.consumer_schedules s SET family_id=s.id,anchor_day=extract(day FROM (a.details->>'nextRun')::date)::integer
FROM core.consumer_authorizations a WHERE a.id=s.authorization_id;
ALTER TABLE core.consumer_schedules ALTER COLUMN family_id SET NOT NULL;
ALTER TABLE core.consumer_schedules ALTER COLUMN anchor_day SET NOT NULL;
ALTER TABLE core.consumer_schedules ADD CONSTRAINT schedule_family FOREIGN KEY(family_id) REFERENCES core.consumer_schedules(id);
ALTER TABLE core.consumer_schedules ADD CONSTRAINT schedule_anchor CHECK(anchor_day BETWEEN 1 AND 31);
CREATE TABLE core.consumer_schedule_replacements(
 original_id uuid PRIMARY KEY REFERENCES core.consumer_schedules(id),
 replacement_id uuid NOT NULL UNIQUE REFERENCES core.consumer_schedules(id),
 authorization_id uuid NOT NULL UNIQUE REFERENCES core.consumer_authorizations(id),
 basis jsonb NOT NULL,basis_digest text NOT NULL,created_at timestamptz NOT NULL DEFAULT clock_timestamp()
);
CREATE TRIGGER immutable_schedule_replacements BEFORE UPDATE OR DELETE OR TRUNCATE ON core.consumer_schedule_replacements
FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal();
-- Original run rows remain byte-for-byte unchanged. This separate index enforces one family/date effect.
CREATE TABLE core.consumer_schedule_family_runs(
 family_id uuid NOT NULL REFERENCES core.consumer_schedules(id),run_date date NOT NULL,
 schedule_id uuid NOT NULL,PRIMARY KEY(family_id,run_date),UNIQUE(schedule_id,run_date),
 FOREIGN KEY(schedule_id,run_date) REFERENCES core.consumer_schedule_runs(schedule_id,run_date)
);
INSERT INTO core.consumer_schedule_family_runs SELECT s.family_id,r.run_date,s.id
FROM core.consumer_schedule_runs r JOIN core.consumer_schedules s ON s.id=r.schedule_id;
CREATE TRIGGER immutable_schedule_family_runs BEFORE UPDATE OR DELETE OR TRUNCATE ON core.consumer_schedule_family_runs
FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal();
REVOKE ALL ON core.consumer_schedule_replacements,core.consumer_schedule_family_runs FROM bank_core;
GRANT SELECT ON core.consumer_schedule_replacements,core.consumer_schedule_family_runs TO bank_core;

CREATE FUNCTION core.consumer_schedule_config() RETURNS trigger LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,core AS $$
BEGIN
 IF TG_OP='INSERT' THEN
  NEW.family_id:=coalesce(NEW.family_id,NEW.id);
  NEW.anchor_day:=extract(day FROM (SELECT details->>'nextRun' FROM core.consumer_authorizations WHERE id=NEW.authorization_id)::date)::integer;
 ELSE
  IF ROW(NEW.person_id,NEW.actor_context,NEW.authorization_id,NEW.kind,NEW.source_account_id,NEW.vault_id,NEW.symbol,NEW.amount_minor,NEW.frequency,NEW.family_id,NEW.anchor_day,NEW.created_at)
   IS DISTINCT FROM ROW(OLD.person_id,OLD.actor_context,OLD.authorization_id,OLD.kind,OLD.source_account_id,OLD.vault_id,OLD.symbol,OLD.amount_minor,OLD.frequency,OLD.family_id,OLD.anchor_day,OLD.created_at) THEN
   RAISE EXCEPTION 'Recurring terms are immutable; replace them with a fresh approved mandate';
  END IF;
 END IF;RETURN NEW;
END $$;
CREATE TRIGGER consumer_schedule_config BEFORE INSERT OR UPDATE ON core.consumer_schedules FOR EACH ROW EXECUTE FUNCTION core.consumer_schedule_config();
CREATE FUNCTION core.consumer_schedule_family_run() RETURNS trigger LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,core AS $$
BEGIN
 INSERT INTO core.consumer_schedule_family_runs SELECT family_id,NEW.run_date,id FROM core.consumer_schedules WHERE id=NEW.schedule_id;
 RETURN NEW;
END $$;
CREATE TRIGGER consumer_schedule_family_run AFTER INSERT ON core.consumer_schedule_runs FOR EACH ROW EXECUTE FUNCTION core.consumer_schedule_family_run();

CREATE FUNCTION core.consumer_next_run(p_date date,p_frequency text,p_anchor integer) RETURNS date LANGUAGE plpgsql IMMUTABLE SET search_path=pg_catalog AS $$
DECLARE first_day date;BEGIN
 IF p_frequency='DAILY' THEN RETURN p_date+1;END IF;
 IF p_frequency='WEEKLY' THEN RETURN p_date+7;END IF;
 IF p_frequency='BIWEEKLY' THEN RETURN p_date+14;END IF;
 IF p_frequency<>'MONTHLY' OR p_anchor NOT BETWEEN 1 AND 31 THEN RAISE EXCEPTION 'Supported cadence and anchor required';END IF;
 first_day:=(date_trunc('month',p_date)+interval '1 month')::date;
 RETURN first_day+least(p_anchor,extract(day FROM first_day+interval '1 month -1 day')::integer)-1;
END $$;
CREATE FUNCTION core.consumer_schedule_basis(p_schedule core.consumer_schedules) RETURNS jsonb LANGUAGE sql IMMUTABLE SET search_path=pg_catalog AS $$
 SELECT jsonb_build_object('id',p_schedule.id,'familyId',p_schedule.family_id,'authorizationId',p_schedule.authorization_id,
 'kind',p_schedule.kind,'accountId',p_schedule.source_account_id,'vaultId',p_schedule.vault_id,'symbol',p_schedule.symbol,
 'amountMinor',p_schedule.amount_minor::text,'frequency',p_schedule.frequency,'nextRun',p_schedule.next_run,'state',p_schedule.state)
$$;
CREATE FUNCTION core.consumer_schedule_terms(p_actor jsonb,p_person uuid,p_data jsonb,p_replace boolean) RETURNS void
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,core AS $$
DECLARE keys text[]:=ARRAY['kind','accountId','vaultId','symbol','amountMinor','frequency','nextRun'];required text[]:=ARRAY['kind','accountId','amountMinor','frequency','nextRun'];BEGIN
 PERFORM core.consumer_actor(p_actor,p_person);
 IF p_replace THEN keys:=keys||ARRAY['id'];required:=required||ARRAY['id'];END IF;
 IF jsonb_typeof(p_data) IS DISTINCT FROM 'object' OR EXISTS(SELECT 1 FROM jsonb_object_keys(p_data) k WHERE k<>ALL(keys))
 OR EXISTS(SELECT 1 FROM unnest(required) k WHERE NOT p_data?k OR jsonb_typeof(p_data->k)<>'string') THEN RAISE EXCEPTION 'Complete exact recurring terms required';END IF;
 IF p_data->>'kind' NOT IN('SAVE','INVEST') OR p_data->>'frequency' NOT IN('DAILY','WEEKLY','BIWEEKLY','MONTHLY')
 OR core.consumer_minor(p_data->>'amountMinor')<500 OR p_data->>'nextRun'!~'^[0-9]{4}-[0-9]{2}-[0-9]{2}$'
 OR (p_data->>'nextRun')::date<(clock_timestamp() AT TIME ZONE 'UTC')::date THEN RAISE EXCEPTION 'Positive exact amount, cadence and current or future UTC date required';END IF;
 PERFORM core.consumer_account((p_data->>'accountId')::uuid,p_person);
 IF p_data->>'kind'='SAVE' THEN
  IF p_data?'symbol' OR NOT EXISTS(SELECT 1 FROM core.consumer_vaults WHERE id=(p_data->>'vaultId')::uuid AND person_id=p_person) THEN RAISE EXCEPTION 'Owned destination vault required';END IF;
 ELSE
  IF p_data?'vaultId' OR NOT EXISTS(SELECT 1 FROM core.wealth_assets WHERE symbol=p_data->>'symbol') THEN RAISE EXCEPTION 'Supported investment destination required';END IF;
  PERFORM core.wealth_owner(p_person);
 END IF;
END $$;
ALTER FUNCTION core.consumer_review(jsonb,uuid,text,jsonb) RENAME TO consumer_review_v59;
REVOKE EXECUTE ON FUNCTION core.consumer_review_v59(jsonb,uuid,text,jsonb) FROM bank_core;
CREATE FUNCTION core.consumer_review(p_actor jsonb,p_person uuid,p_action text,p_data jsonb) RETURNS jsonb
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,core AS $$
DECLARE s core.consumer_schedules;basis jsonb;latest date;BEGIN
 IF p_action NOT IN('SCHEDULE_CREATE','SCHEDULE_REPLACE') THEN RETURN core.consumer_review_v59(p_actor,p_person,p_action,p_data);END IF;
 PERFORM core.consumer_actor(p_actor,p_person);PERFORM pg_advisory_xact_lock(hashtextextended(p_person::text,307));
 PERFORM core.consumer_schedule_terms(p_actor,p_person,p_data,p_action='SCHEDULE_REPLACE');
 IF p_action='SCHEDULE_CREATE' THEN RETURN core.consumer_authorize(p_actor,p_person,p_action,p_data);END IF;
 SELECT * INTO STRICT s FROM core.consumer_schedules WHERE id=(p_data->>'id')::uuid AND person_id=p_person FOR SHARE;
 IF s.state NOT IN('ACTIVE','PAUSED') THEN RAISE EXCEPTION 'Only a current active or paused instruction can be edited';END IF;
 SELECT max(run_date) INTO latest FROM core.consumer_schedule_family_runs WHERE family_id=s.family_id;
 IF (p_data->>'nextRun')::date<=latest THEN RAISE EXCEPTION 'Next execution date must follow the last completed family run';END IF;
 basis:=core.consumer_schedule_basis(s);
 RETURN core.consumer_authorize(p_actor,p_person,p_action,p_data||jsonb_build_object('replaces',basis,'basisDigest',encode(sha256(convert_to(core.periodic_policy_canonical(basis),'UTF8')),'hex')));
END $$;
GRANT EXECUTE ON FUNCTION core.consumer_review(jsonb,uuid,text,jsonb) TO bank_core;

CREATE FUNCTION core.consumer_schedule_replace(p_actor jsonb,p_person uuid,p_key uuid,p_data jsonb) RETURNS jsonb
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,core AS $$
DECLARE old core.consumer_commands;s core.consumer_schedules;d jsonb;basis jsonb;target uuid:=gen_random_uuid();result jsonb;latest date;BEGIN
 PERFORM core.consumer_actor(p_actor,p_person);PERFORM pg_advisory_xact_lock(hashtextextended(p_person::text,307));
 IF p_key IS NULL OR p_key='00000000-0000-0000-0000-000000000000' OR jsonb_typeof(p_data) IS DISTINCT FROM 'object'
 OR NOT p_data?'authorizationId' OR EXISTS(SELECT 1 FROM jsonb_object_keys(p_data) k WHERE k<>'authorizationId') THEN RAISE EXCEPTION 'Original request key and exact authorization required';END IF;
 SELECT * INTO old FROM core.consumer_commands WHERE issuer=p_actor->>'issuer' AND subject=p_actor->>'subject' AND client_id=p_actor->>'clientId' AND request_key=p_key;
 IF FOUND THEN
  IF old.person_id IS DISTINCT FROM p_person OR old.action<>'SCHEDULE_REPLACE' OR old.input IS DISTINCT FROM p_data THEN RAISE EXCEPTION 'Command key scope conflict';END IF;RETURN old.result;
 END IF;
 d:=core.consumer_approval(p_actor,p_person,(p_data->>'authorizationId')::uuid,'SCHEDULE_REPLACE');
 SELECT * INTO STRICT s FROM core.consumer_schedules WHERE id=(d->>'id')::uuid AND person_id=p_person FOR UPDATE;
 basis:=core.consumer_schedule_basis(s);
 IF s.state NOT IN('ACTIVE','PAUSED') OR d->'replaces' IS DISTINCT FROM basis
 OR d->>'basisDigest' IS DISTINCT FROM encode(sha256(convert_to(core.periodic_policy_canonical(basis),'UTF8')),'hex') THEN RAISE EXCEPTION 'Instruction changed after review; review its current terms again';END IF;
 PERFORM core.consumer_schedule_terms(p_actor,p_person,d-'replaces'-'basisDigest',true);
 SELECT max(run_date) INTO latest FROM core.consumer_schedule_family_runs WHERE family_id=s.family_id;
 IF (d->>'nextRun')::date<=latest THEN RAISE EXCEPTION 'Next execution date must follow the last completed family run';END IF;
 PERFORM core.consumer_approval(p_actor,p_person,(p_data->>'authorizationId')::uuid,'SCHEDULE_REPLACE');
 INSERT INTO core.consumer_schedules(id,person_id,actor_context,authorization_id,kind,source_account_id,vault_id,symbol,amount_minor,frequency,next_run,state,family_id)
 VALUES(target,p_person,p_actor,(p_data->>'authorizationId')::uuid,d->>'kind',(d->>'accountId')::uuid,(d->>'vaultId')::uuid,d->>'symbol',core.consumer_minor(d->>'amountMinor'),d->>'frequency',(d->>'nextRun')::date,s.state,s.family_id);
 INSERT INTO core.consumer_schedule_replacements VALUES(s.id,target,(p_data->>'authorizationId')::uuid,basis,d->>'basisDigest',clock_timestamp());
 UPDATE core.consumer_schedules SET state='CANCELLED' WHERE id=s.id;
 result:=jsonb_build_object('id',target,'replacesId',s.id,'familyId',s.family_id,'state',s.state);
 INSERT INTO core.consumer_commands VALUES(target,p_actor->>'issuer',p_actor->>'subject',p_actor->>'clientId',p_key,p_person,'SCHEDULE_REPLACE',p_data,result,clock_timestamp());
 PERFORM core.consumer_notify(p_person,CASE WHEN s.kind='INVEST' THEN 'Investments' ELSE 'Savings' END,'Recurring instruction updated','Previous consent and completed runs remain retained.','/invest/automate');
 RETURN result;
END $$;
ALTER FUNCTION core.consumer_command(jsonb,uuid,uuid,text,jsonb) RENAME TO consumer_command_v59;
REVOKE EXECUTE ON FUNCTION core.consumer_command_v59(jsonb,uuid,uuid,text,jsonb) FROM bank_core;
CREATE FUNCTION core.consumer_command(p_actor jsonb,p_person uuid,p_key uuid,p_action text,p_data jsonb) RETURNS jsonb
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,core AS $$ BEGIN
 IF p_action='SCHEDULE_REPLACE' THEN RETURN core.consumer_schedule_replace(p_actor,p_person,p_key,p_data);END IF;
 RETURN core.consumer_command_v59(p_actor,p_person,p_key,p_action,p_data);
END $$;
GRANT EXECUTE ON FUNCTION core.consumer_command(jsonb,uuid,uuid,text,jsonb) TO bank_core;

-- Preserve existing recovery behavior; accept retained replacement consent and anchored monthly cadence.
CREATE OR REPLACE FUNCTION core.consumer_tick() RETURNS integer LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,core AS $$
DECLARE s core.consumer_schedules;original core.consumer_authorizations;candidate record;cash uuid;operation uuid;batch uuid;funding uuid;asset core.wealth_assets;quantity bigint;cost bigint;executed integer:=0;order_row core.wealth_orders;corporate core.wealth_corporate_actions;w core.wealth_accounts;owned_units bigint;dividend bigint;flow uuid;BEGIN
 FOR candidate IN SELECT id,person_id FROM core.consumer_schedules WHERE state='ACTIVE' AND next_run<=(clock_timestamp() AT TIME ZONE 'UTC')::date AND retry_after<=clock_timestamp() ORDER BY next_run,id LIMIT 25 LOOP
  IF NOT pg_try_advisory_xact_lock(hashtextextended(candidate.person_id::text,307)) THEN CONTINUE;END IF;
  SELECT * INTO s FROM core.consumer_schedules WHERE id=candidate.id AND state='ACTIVE' FOR UPDATE SKIP LOCKED;IF NOT FOUND THEN CONTINUE;END IF;
  BEGIN
   PERFORM core.consumer_actor(s.actor_context,s.person_id);PERFORM core.consumer_account(s.source_account_id,s.person_id);
   SELECT * INTO STRICT original FROM core.consumer_authorizations WHERE id=s.authorization_id AND person_id=s.person_id AND action IN('SCHEDULE_CREATE','SCHEDULE_REPLACE');
   IF original.details->>'accountId' IS DISTINCT FROM s.source_account_id::text OR original.details->>'amountMinor' IS DISTINCT FROM s.amount_minor::text OR original.details->>'frequency' IS DISTINCT FROM s.frequency OR original.details->>'kind' IS DISTINCT FROM s.kind OR original.details->>'symbol' IS DISTINCT FROM s.symbol OR (original.details->>'vaultId')::uuid IS DISTINCT FROM s.vault_id THEN RAISE EXCEPTION 'Retained recurring consent changed';END IF;
   operation:=gen_random_uuid();
   IF NOT EXISTS(SELECT 1 FROM core.consumer_schedule_runs WHERE schedule_id=s.id AND run_date=s.next_run) THEN
    IF s.kind='SAVE' THEN
     IF NOT EXISTS(SELECT 1 FROM core.consumer_vaults WHERE id=s.vault_id AND person_id=s.person_id) THEN RAISE EXCEPTION 'Current vault ownership required';END IF;
     batch:=core.consumer_post(operation,'AUTO_SAVE',s.source_account_id,s.vault_id,s.amount_minor);
    ELSE
     cash:=core.wealth_owner(s.person_id);SELECT * INTO STRICT asset FROM core.wealth_assets WHERE symbol=s.symbol;quantity:=trunc(s.amount_minor::numeric*1000000/asset.price_minor)::bigint;cost:=ceil(quantity::numeric*asset.price_minor/1000000)::bigint;IF quantity<1 OR cost<1 THEN RAISE EXCEPTION 'Representable recurring trade required';END IF;
     flow:=gen_random_uuid();funding:=core.consumer_post(flow,'BROKER_DEPOSIT',s.source_account_id,cash,s.amount_minor);INSERT INTO core.wealth_cash_flows VALUES(flow,s.person_id,s.amount_minor,funding,clock_timestamp());
     batch:=core.consumer_post(operation,'RECURRING_SECURITY_BUY',cash,'4f1a780b-3e31-48ef-a294-000000000020',cost);
     INSERT INTO core.wealth_positions(person_id,symbol,units,cost_minor) VALUES(s.person_id,s.symbol,quantity,cost) ON CONFLICT(person_id,symbol) DO UPDATE SET units=core.wealth_positions.units+EXCLUDED.units,cost_minor=core.wealth_positions.cost_minor+EXCLUDED.cost_minor;
     INSERT INTO core.wealth_position_events VALUES(operation,s.person_id,s.symbol,quantity,cost,clock_timestamp());PERFORM core.wealth_observe(s.person_id);
    END IF;
    INSERT INTO core.consumer_schedule_runs VALUES(s.id,s.next_run,operation,batch,clock_timestamp());executed:=executed+1;
   END IF;
   UPDATE core.consumer_schedules SET next_run=core.consumer_next_run(s.next_run,s.frequency,s.anchor_day),last_error='',retry_after=clock_timestamp() WHERE id=s.id;
   PERFORM core.consumer_notify(s.person_id,CASE WHEN s.kind='INVEST' THEN 'Investments' ELSE 'Savings' END,'Scheduled instruction completed','Your instruction has a native financial receipt.',CASE WHEN s.kind='INVEST' THEN '/invest/automate' ELSE '/save' END);
  EXCEPTION WHEN OTHERS THEN UPDATE core.consumer_schedules SET last_error='AUTOMATION_REQUIRES_REVIEW',retry_after=clock_timestamp()+interval '1 minute' WHERE id=s.id;END;
 END LOOP;
 FOR candidate IN SELECT id,person_id FROM core.wealth_orders WHERE state='OPEN' ORDER BY last_check,created_at,id LIMIT 50 LOOP
  IF NOT pg_try_advisory_xact_lock(hashtextextended(candidate.person_id::text,307)) THEN CONTINUE;END IF;
  SELECT * INTO order_row FROM core.wealth_orders WHERE id=candidate.id AND state='OPEN' FOR UPDATE SKIP LOCKED;IF NOT FOUND THEN CONTINUE;END IF;
  UPDATE core.wealth_orders SET last_check=clock_timestamp() WHERE id=order_row.id;
  BEGIN
   IF order_row.expires_at<=clock_timestamp() THEN
    IF order_row.side='BUY' THEN PERFORM core.release_hold(order_row.id);ELSE UPDATE core.wealth_positions SET reserved_units=reserved_units-order_row.units WHERE person_id=order_row.person_id AND symbol=order_row.symbol;END IF;UPDATE core.wealth_orders SET state='EXPIRED' WHERE id=order_row.id;
   ELSE PERFORM core.wealth_fill(order_row.id);END IF;
  EXCEPTION WHEN OTHERS THEN UPDATE core.wealth_orders SET last_error='ORDER_REQUIRES_REVIEW' WHERE id=order_row.id;END;
 END LOOP;
 FOR corporate IN SELECT * FROM core.wealth_corporate_actions WHERE pay_at<=clock_timestamp() ORDER BY pay_at,id LIMIT 50 LOOP
  FOR w IN SELECT * FROM core.wealth_accounts WHERE NOT EXISTS(SELECT 1 FROM core.wealth_action_observations WHERE person_id=core.wealth_accounts.person_id AND action_id=corporate.id) AND NOT EXISTS(SELECT 1 FROM core.wealth_action_failures WHERE person_id=core.wealth_accounts.person_id AND action_id=corporate.id AND retry_at>clock_timestamp()) ORDER BY person_id LIMIT 100 LOOP
   IF NOT pg_try_advisory_xact_lock(hashtextextended(w.person_id::text,307)) THEN CONTINUE;END IF;
   BEGIN
    PERFORM core.consumer_actor(w.actor_context,w.person_id);cash:=core.wealth_owner(w.person_id);
    SELECT coalesce(sum(units_delta),0)::bigint INTO owned_units FROM core.wealth_position_events WHERE person_id=w.person_id AND symbol=corporate.symbol AND created_at<=corporate.record_at;
    dividend:=trunc(owned_units::numeric*corporate.per_share_minor/1000000)::bigint;
    batch:=NULL;
    IF dividend>0 THEN
     operation:=gen_random_uuid();batch:=core.consumer_post(operation,'SYNTHETIC_DIVIDEND','4f1a780b-3e31-48ef-a294-000000000020',cash,dividend);
     INSERT INTO core.wealth_dividends(id,person_id,symbol,record_date,units,amount_minor,batch_id,action_id) VALUES(operation,w.person_id,corporate.symbol,corporate.record_at::date,owned_units,dividend,batch,corporate.id);
     IF EXISTS(SELECT 1 FROM core.wealth_watchlist WHERE person_id=w.person_id AND symbol=corporate.symbol AND drip) THEN
      SELECT * INTO STRICT asset FROM core.wealth_assets WHERE symbol=corporate.symbol;quantity:=trunc(dividend::numeric*1000000/asset.price_minor)::bigint;cost:=ceil(quantity::numeric*asset.price_minor/1000000)::bigint;
      IF quantity>0 AND cost>0 THEN operation:=gen_random_uuid();PERFORM core.consumer_post(operation,'DIVIDEND_REINVESTMENT',cash,'4f1a780b-3e31-48ef-a294-000000000020',cost);UPDATE core.wealth_positions SET units=units+quantity,cost_minor=cost_minor+cost WHERE person_id=w.person_id AND symbol=corporate.symbol;INSERT INTO core.wealth_position_events VALUES(operation,w.person_id,corporate.symbol,quantity,cost,clock_timestamp());END IF;
     END IF;PERFORM core.wealth_observe(w.person_id);executed:=executed+1;
    END IF;
    INSERT INTO core.wealth_action_observations VALUES(corporate.id,w.person_id,owned_units,dividend,batch);
   EXCEPTION WHEN OTHERS THEN INSERT INTO core.wealth_action_failures VALUES(corporate.id,w.person_id,clock_timestamp()+interval '1 minute','CORPORATE_ACTION_REQUIRES_REVIEW') ON CONFLICT(action_id,person_id) DO UPDATE SET retry_at=EXCLUDED.retry_at,error_code=EXCLUDED.error_code;END;
  END LOOP;
 END LOOP;PERFORM core.consumer_signals_tick();RETURN executed;
END $$;

CREATE FUNCTION core.consumer_schedule_report(p_person uuid) RETURNS jsonb LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,core AS $$
DECLARE result jsonb;BEGIN
 IF EXISTS(
  SELECT 1 FROM core.consumer_schedule_runs r JOIN core.consumer_schedules s ON s.id=r.schedule_id
  LEFT JOIN core.wealth_accounts w ON w.person_id=s.person_id
  LEFT JOIN core.journal_batches b ON b.id=r.batch_id AND b.operation_id=r.operation_id
  LEFT JOIN core.journal_entries e ON e.batch_id=b.id AND e.account_id=CASE WHEN s.kind='SAVE' THEN s.source_account_id ELSE w.cash_account_id END
  WHERE s.person_id=p_person AND (e.delta IS NULL OR e.delta>=0 OR b.posting_kind IS DISTINCT FROM CASE WHEN s.kind='SAVE' THEN 'AUTO_SAVE' ELSE 'RECURRING_SECURITY_BUY' END)
 ) THEN RAISE EXCEPTION 'Recurring execution source proof unavailable';END IF;
 WITH facts AS(
  SELECT r.*,s.family_id,s.person_id,s.kind,-e.delta::numeric executed_minor
  FROM core.consumer_schedule_runs r JOIN core.consumer_schedules s ON s.id=r.schedule_id
  LEFT JOIN core.wealth_accounts w ON w.person_id=s.person_id
  JOIN core.journal_entries e ON e.batch_id=r.batch_id AND e.account_id=CASE WHEN s.kind='SAVE' THEN s.source_account_id ELSE w.cash_account_id END
  WHERE s.person_id=p_person
 ) SELECT jsonb_build_object(
  'schedules',coalesce((SELECT jsonb_agg(jsonb_build_object('id',s.id,'familyId',s.family_id,'kind',s.kind,'accountId',s.source_account_id,'vaultId',s.vault_id,'symbol',s.symbol,'amountMinor',s.amount_minor::text,'frequency',s.frequency,'anchorDay',s.anchor_day,'nextRun',s.next_run,'state',s.state,'lastError',s.last_error,
   'replacesId',(SELECT original_id FROM core.consumer_schedule_replacements WHERE replacement_id=s.id),
   'replacedBy',(SELECT replacement_id FROM core.consumer_schedule_replacements WHERE original_id=s.id),
   'completedRuns',(SELECT count(*) FROM facts WHERE schedule_id=s.id),
   'executedMinor',(SELECT coalesce(sum(executed_minor),0)::text FROM facts WHERE schedule_id=s.id),
   'familyCompletedRuns',(SELECT count(*) FROM facts WHERE family_id=s.family_id),
   'familyExecutedMinor',(SELECT coalesce(sum(executed_minor),0)::text FROM facts WHERE family_id=s.family_id),
   'lastCompletedDate',(SELECT max(run_date) FROM facts WHERE family_id=s.family_id),
   'recentRuns',coalesce((SELECT jsonb_agg(x ORDER BY x->>'dueDate' DESC) FROM(SELECT jsonb_build_object('dueDate',run_date,'batchId',batch_id,'executedMinor',executed_minor::text,'postedAt',created_at) x FROM facts WHERE schedule_id=s.id ORDER BY run_date DESC LIMIT 5) q),'[]'::jsonb)
  ) ORDER BY s.created_at DESC,s.id) FROM core.consumer_schedules s WHERE s.person_id=p_person),'[]'::jsonb),
  'automationSummary',jsonb_build_object(
   'activeCount',(SELECT count(*) FROM core.consumer_schedules WHERE person_id=p_person AND state='ACTIVE'),
   'nextDueDate',(SELECT min(next_run) FROM core.consumer_schedules WHERE person_id=p_person AND state='ACTIVE'),
   'saveMonthlyMinor',(SELECT coalesce(ceil(sum(amount_minor::numeric*CASE frequency WHEN 'DAILY' THEN 365 WHEN 'WEEKLY' THEN 52 WHEN 'BIWEEKLY' THEN 26 ELSE 12 END)/12),0)::text FROM core.consumer_schedules WHERE person_id=p_person AND state='ACTIVE' AND kind='SAVE'),
   'investMonthlyMinor',(SELECT coalesce(ceil(sum(amount_minor::numeric*CASE frequency WHEN 'DAILY' THEN 365 WHEN 'WEEKLY' THEN 52 WHEN 'BIWEEKLY' THEN 26 ELSE 12 END)/12),0)::text FROM core.consumer_schedules WHERE person_id=p_person AND state='ACTIVE' AND kind='INVEST'),
   'totalSavedMinor',(SELECT coalesce(sum(executed_minor) FILTER(WHERE kind='SAVE'),0)::text FROM facts),
   'totalInvestedMinor',(SELECT coalesce(sum(executed_minor) FILTER(WHERE kind='INVEST'),0)::text FROM facts)
  )
 ) INTO result;RETURN result;
END $$;
ALTER FUNCTION core.wealth_snapshot(jsonb,uuid) RENAME TO wealth_snapshot_v59;
REVOKE EXECUTE ON FUNCTION core.wealth_snapshot_v59(jsonb,uuid) FROM bank_core;
CREATE FUNCTION core.wealth_snapshot(p_actor jsonb,p_person uuid) RETURNS jsonb LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,core AS $$ BEGIN
 RETURN core.wealth_snapshot_v59(p_actor,p_person)||core.consumer_schedule_report(p_person);
END $$;
GRANT EXECUTE ON FUNCTION core.wealth_snapshot(jsonb,uuid) TO bank_core;

CREATE FUNCTION core.consumer_vault_contributors(p_person uuid) RETURNS jsonb LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog,core AS $$
 WITH posted AS(
  SELECT e.account_id vault_id,coalesce(c.person_id,s.person_id,f.person_id) person_id,e.delta::numeric amount
  FROM core.journal_entries e JOIN core.journal_batches b ON b.id=e.batch_id
  LEFT JOIN core.consumer_commands c ON c.id=b.operation_id AND c.action='VAULT_DEPOSIT'
  LEFT JOIN core.consumer_schedule_runs r ON r.batch_id=b.id
  LEFT JOIN core.consumer_schedules s ON s.id=r.schedule_id AND s.kind='SAVE'
  LEFT JOIN core.consumer_roundup_facts f ON f.batch_id=b.id
  WHERE e.delta>0 AND EXISTS(SELECT 1 FROM core.consumer_members WHERE vault_id=e.account_id AND person_id=p_person AND state='ACTIVE')
 ),members AS(
  SELECT m.vault_id,m.person_id,row_number() OVER(PARTITION BY m.vault_id ORDER BY m.person_id) ordinal,
  coalesce((SELECT sum(amount) FROM posted p WHERE p.vault_id=m.vault_id AND p.person_id=m.person_id),0) contribution
  FROM core.consumer_members m WHERE m.state='ACTIVE'
  AND EXISTS(SELECT 1 FROM core.consumer_members x WHERE x.vault_id=m.vault_id AND x.person_id=p_person AND x.state='ACTIVE')
 ) SELECT coalesce(jsonb_agg(jsonb_build_object('vaultId',vault_id,'label',CASE WHEN person_id=p_person THEN 'You' ELSE 'Contributor '||ordinal END,'isYou',person_id=p_person,'contributedMinor',contribution::text) ORDER BY vault_id,ordinal),'[]'::jsonb) FROM members
$$;
ALTER FUNCTION core.consumer_snapshot(jsonb,uuid) RENAME TO consumer_snapshot_v59;
REVOKE EXECUTE ON FUNCTION core.consumer_snapshot_v59(jsonb,uuid) FROM bank_core;
CREATE FUNCTION core.consumer_snapshot(p_actor jsonb,p_person uuid) RETURNS jsonb LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,core AS $$
DECLARE d jsonb;r jsonb;BEGIN
 d:=core.consumer_snapshot_v59(p_actor,p_person);r:=d->'roundups';
 IF r IS NOT NULL AND r<>'null'::jsonb THEN
  r:=r||jsonb_build_object('saved30DaysMinor',(SELECT coalesce(sum(amount_minor::numeric),0)::text FROM core.consumer_roundup_facts WHERE person_id=p_person AND created_at>=clock_timestamp()-interval '720 hours'),
  'roundupCount',(SELECT count(*) FROM core.consumer_roundup_facts WHERE person_id=p_person));
 END IF;
 RETURN d||jsonb_build_object('roundups',r,'vaultContributors',core.consumer_vault_contributors(p_person));
END $$;
GRANT EXECUTE ON FUNCTION core.consumer_snapshot(jsonb,uuid) TO bank_core;
