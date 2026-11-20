CREATE TABLE core.consumer_group_withdrawals(id uuid PRIMARY KEY,vault_id uuid NOT NULL REFERENCES core.consumer_vaults(id),person_id uuid NOT NULL,account_id uuid NOT NULL REFERENCES core.account_registry(id),amount_minor bigint NOT NULL CHECK(amount_minor>0),state text NOT NULL DEFAULT 'PENDING',batch_id uuid,created_at timestamptz NOT NULL DEFAULT clock_timestamp());
CREATE TABLE core.consumer_group_approvals(withdrawal_id uuid NOT NULL REFERENCES core.consumer_group_withdrawals(id),person_id uuid NOT NULL,PRIMARY KEY(withdrawal_id,person_id));
CREATE TABLE core.consumer_roundups(person_id uuid PRIMARY KEY,card_id uuid NOT NULL REFERENCES core.consumer_cards(id),vault_id uuid NOT NULL REFERENCES core.consumer_vaults(id),multiplier integer NOT NULL CHECK(multiplier IN(1,2,3,5)),weekly_cap bigint NOT NULL CHECK(weekly_cap>0),enabled boolean NOT NULL DEFAULT true);
CREATE TABLE core.consumer_roundup_facts(operation_id uuid PRIMARY KEY,person_id uuid NOT NULL,amount_minor bigint NOT NULL,batch_id uuid NOT NULL UNIQUE,created_at timestamptz NOT NULL DEFAULT clock_timestamp());
CREATE TRIGGER immutable_consumer_roundup_facts BEFORE UPDATE OR DELETE OR TRUNCATE ON core.consumer_roundup_facts FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal();
CREATE TABLE core.consumer_statements(id uuid PRIMARY KEY,person_id uuid NOT NULL,document jsonb NOT NULL,digest text NOT NULL,created_at timestamptz NOT NULL DEFAULT clock_timestamp());
CREATE TRIGGER immutable_consumer_statements BEFORE UPDATE OR DELETE OR TRUNCATE ON core.consumer_statements FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal();
CREATE TABLE core.wealth_corporate_actions(id uuid PRIMARY KEY,symbol text NOT NULL REFERENCES core.wealth_assets(symbol),record_at timestamptz NOT NULL,pay_at timestamptz NOT NULL,per_share_minor bigint NOT NULL CHECK(per_share_minor>0),source text NOT NULL CHECK(source='SYNTHETIC_CORPORATE_ACTION'),CHECK(pay_at>=record_at));
CREATE TRIGGER immutable_wealth_corporate_actions BEFORE UPDATE OR DELETE OR TRUNCATE ON core.wealth_corporate_actions FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal();
ALTER TABLE core.wealth_dividends ADD COLUMN action_id uuid REFERENCES core.wealth_corporate_actions(id);
CREATE UNIQUE INDEX wealth_dividends_action ON core.wealth_dividends(person_id,action_id);
INSERT INTO core.wealth_corporate_actions SELECT gen_random_uuid(),symbol,(date_trunc('month',clock_timestamp())+interval '1 month'),(date_trunc('month',clock_timestamp())+interval '1 month 1 day'),greatest(1,trunc(price_minor::numeric*dividend_bps/40000)::bigint),'SYNTHETIC_CORPORATE_ACTION' FROM core.wealth_assets WHERE dividend_bps>0;
ALTER TABLE core.consumer_links ADD COLUMN funded_minor bigint NOT NULL DEFAULT 0 CHECK(funded_minor>=0 AND funded_minor<=100000);
DO $$ DECLARE t text;BEGIN FOREACH t IN ARRAY ARRAY['consumer_group_withdrawals','consumer_group_approvals','consumer_roundups','consumer_roundup_facts','consumer_statements','wealth_corporate_actions'] LOOP EXECUTE format('REVOKE ALL ON core.%I FROM bank_core',t);EXECUTE format('GRANT SELECT ON core.%I TO bank_core',t);END LOOP;END $$;
ALTER FUNCTION core.consumer_command(jsonb,uuid,uuid,text,jsonb) RENAME TO consumer_base_command;
REVOKE EXECUTE ON FUNCTION core.consumer_base_command(jsonb,uuid,uuid,text,jsonb) FROM bank_core;
ALTER FUNCTION core.consumer_snapshot(jsonb,uuid) RENAME TO consumer_base_snapshot;
REVOKE EXECUTE ON FUNCTION core.consumer_base_snapshot(jsonb,uuid) FROM bank_core;
ALTER FUNCTION core.consumer_review(jsonb,uuid,text,jsonb) RENAME TO consumer_base_review;
REVOKE EXECUTE ON FUNCTION core.consumer_base_review(jsonb,uuid,text,jsonb) FROM bank_core;
CREATE FUNCTION core.consumer_review(p_actor jsonb,p_person uuid,p_action text,p_data jsonb) RETURNS jsonb LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,core AS $$
DECLARE keys text[];BEGIN
 IF p_action NOT IN('CARD_PURCHASE','ROUNDUP_SET','DRIP_SET','LINK_DEPOSIT') THEN RETURN core.consumer_base_review(p_actor,p_person,p_action,p_data);END IF;
 PERFORM core.consumer_actor(p_actor,p_person);
 keys:=CASE p_action WHEN 'CARD_PURCHASE' THEN ARRAY['id','merchant','category','amountMinor'] WHEN 'ROUNDUP_SET' THEN ARRAY['cardId','vaultId','multiplier','weeklyCapMinor','enabled'] WHEN 'DRIP_SET' THEN ARRAY['symbol','enabled'] ELSE ARRAY['id','accountId','amountMinor'] END;
 IF jsonb_typeof(p_data)<>'object' OR EXISTS(SELECT 1 FROM jsonb_object_keys(p_data) k WHERE k<>ALL(keys)) THEN RAISE EXCEPTION 'Supported action fields required';END IF;
 RETURN core.consumer_authorize(p_actor,p_person,p_action,p_data);END $$;
CREATE FUNCTION core.consumer_command(p_actor jsonb,p_person uuid,p_key uuid,p_action text,p_data jsonb) RETURNS jsonb LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,core AS $$
DECLARE old core.consumer_commands;target uuid:=gen_random_uuid();d jsonb;v core.consumer_vaults;w core.consumer_group_withdrawals;c core.consumer_cards;r core.consumer_roundups;amount bigint;spare bigint;remaining bigint;bank uuid;batch uuid;round_batch uuid;result jsonb;facts jsonb;fact_count bigint;stamp timestamptz;approved boolean;keys text[];BEGIN
 IF p_action NOT IN('GROUP_WITHDRAW','GROUP_APPROVE','GROUP_CANCEL','CARD_PURCHASE','ROUNDUP_SET','DRIP_SET','LINK_DEPOSIT','STATEMENT') THEN
  IF p_action='VAULT_UPDATE' AND (p_data->>'mutual')::boolean IS false AND (SELECT count(*) FROM core.consumer_members WHERE vault_id=(p_data->>'id')::uuid AND state='ACTIVE')>1 THEN RAISE EXCEPTION 'Shared withdrawal approval cannot be bypassed';END IF;
  result:=core.consumer_base_command(p_actor,p_person,p_key,p_action,p_data);
  IF p_action='VAULT_INVITE' THEN UPDATE core.consumer_vaults SET mutual=true WHERE id=(p_data->>'id')::uuid;END IF;RETURN result;
 END IF;
 PERFORM core.consumer_actor(p_actor,p_person);IF p_key IS NULL OR p_key='00000000-0000-0000-0000-000000000000' THEN RAISE EXCEPTION 'Request key required';END IF;
 PERFORM pg_advisory_xact_lock(hashtextextended(p_person::text,307));SELECT * INTO old FROM core.consumer_commands WHERE issuer=p_actor->>'issuer' AND subject=p_actor->>'subject' AND client_id=p_actor->>'clientId' AND request_key=p_key;
 IF FOUND THEN IF old.person_id<>p_person OR old.action<>p_action OR old.input<>p_data THEN RAISE EXCEPTION 'Command scope conflict';END IF;RETURN old.result;END IF;
 keys:=CASE WHEN p_action IN('CARD_PURCHASE','ROUNDUP_SET','DRIP_SET','LINK_DEPOSIT') THEN ARRAY['authorizationId'] WHEN p_action='GROUP_WITHDRAW' THEN ARRAY['id','accountId','amountMinor'] WHEN p_action='STATEMENT' THEN ARRAY['accountId','from','to'] ELSE ARRAY['id'] END;
 IF jsonb_typeof(p_data) IS DISTINCT FROM 'object' OR EXISTS(SELECT 1 FROM jsonb_object_keys(p_data) k WHERE k<>ALL(keys)) OR EXISTS(SELECT 1 FROM unnest(keys) k WHERE NOT p_data?k OR p_data->k='null'::jsonb) THEN RAISE EXCEPTION 'Complete supported command fields required';END IF;
 IF p_action='GROUP_WITHDRAW' THEN
  SELECT * INTO STRICT v FROM core.consumer_vaults WHERE id=(p_data->>'id')::uuid FOR UPDATE;
  IF v.locked OR NOT EXISTS(SELECT 1 FROM core.consumer_members WHERE vault_id=v.id AND person_id=p_person AND state='ACTIVE') THEN RAISE EXCEPTION 'Unlocked member vault required';END IF;
  bank:=(p_data->>'accountId')::uuid;PERFORM core.consumer_account(bank,p_person);amount:=core.consumer_minor(p_data->>'amountMinor');PERFORM core.reserve(target,v.id,amount);
  INSERT INTO core.consumer_group_withdrawals VALUES(target,v.id,p_person,bank,amount,'PENDING',NULL,clock_timestamp());INSERT INTO core.consumer_group_approvals VALUES(target,p_person);result:=jsonb_build_object('id',target,'state','PENDING');
 ELSIF p_action IN('GROUP_APPROVE','GROUP_CANCEL') THEN
  SELECT * INTO STRICT w FROM core.consumer_group_withdrawals WHERE id=(p_data->>'id')::uuid FOR UPDATE;SELECT * INTO STRICT v FROM core.consumer_vaults WHERE id=w.vault_id FOR UPDATE;
  IF NOT EXISTS(SELECT 1 FROM core.consumer_members WHERE vault_id=v.id AND person_id=p_person AND state='ACTIVE') THEN RAISE EXCEPTION 'Current member required';END IF;
  IF w.state='PENDING' THEN
   IF p_action='GROUP_CANCEL' THEN IF w.person_id<>p_person THEN RAISE EXCEPTION 'Requester required';END IF;PERFORM core.release_hold(w.id);UPDATE core.consumer_group_withdrawals SET state='CANCELLED' WHERE id=w.id;
   ELSE
    INSERT INTO core.consumer_group_approvals VALUES(w.id,p_person) ON CONFLICT DO NOTHING;
    approved:=NOT EXISTS(SELECT 1 FROM core.consumer_members m WHERE m.vault_id=v.id AND m.state='ACTIVE' AND NOT EXISTS(SELECT 1 FROM core.consumer_group_approvals a WHERE a.withdrawal_id=w.id AND a.person_id=m.person_id));
    IF approved THEN IF v.locked THEN RAISE EXCEPTION 'Vault is locked';END IF;PERFORM core.consumer_account(w.account_id,w.person_id);batch:=core.consumer_post(w.id,'SHARED_WITHDRAWAL',v.id,w.account_id,w.amount_minor,w.id);UPDATE core.consumer_group_withdrawals SET state='POSTED',batch_id=batch WHERE id=w.id;END IF;
   END IF;
  END IF;SELECT jsonb_build_object('id',id,'state',state,'batchId',batch_id) INTO result FROM core.consumer_group_withdrawals WHERE id=w.id;
 ELSIF p_action IN('CARD_PURCHASE','ROUNDUP_SET','DRIP_SET','LINK_DEPOSIT') THEN
  d:=core.consumer_approval(p_actor,p_person,(p_data->>'authorizationId')::uuid,p_action);
  IF p_action='ROUNDUP_SET' THEN
   SELECT * INTO STRICT c FROM core.consumer_cards WHERE id=(d->>'cardId')::uuid AND person_id=p_person;SELECT * INTO STRICT v FROM core.consumer_vaults WHERE id=(d->>'vaultId')::uuid AND person_id=p_person;
   INSERT INTO core.consumer_roundups VALUES(p_person,c.id,v.id,(d->>'multiplier')::integer,core.consumer_minor(d->>'weeklyCapMinor'),coalesce((d->>'enabled')::boolean,true)) ON CONFLICT(person_id) DO UPDATE SET card_id=EXCLUDED.card_id,vault_id=EXCLUDED.vault_id,multiplier=EXCLUDED.multiplier,weekly_cap=EXCLUDED.weekly_cap,enabled=EXCLUDED.enabled;result:=jsonb_build_object('saved',true);
  ELSIF p_action='DRIP_SET' THEN
   PERFORM core.wealth_owner(p_person);PERFORM core.consumer_approval(p_actor,p_person,(p_data->>'authorizationId')::uuid,p_action);INSERT INTO core.wealth_watchlist VALUES(p_person,d->>'symbol',NULL,coalesce((d->>'enabled')::boolean,false)) ON CONFLICT(person_id,symbol) DO UPDATE SET drip=EXCLUDED.drip;result:=jsonb_build_object('saved',true);
  ELSIF p_action='LINK_DEPOSIT' THEN
   bank:=(d->>'accountId')::uuid;PERFORM core.consumer_account(bank,p_person);PERFORM core.consumer_approval(p_actor,p_person,(p_data->>'authorizationId')::uuid,p_action);amount:=core.consumer_minor(d->>'amountMinor');
   -- A finite explicit evaluation allowance, shared across all synthetic links.
   IF coalesce((SELECT sum(funded_minor::numeric) FROM core.consumer_links WHERE person_id=p_person),0)+amount>100000 THEN RAISE EXCEPTION 'Synthetic funding allowance exhausted';END IF;
   UPDATE core.consumer_links SET funded_minor=funded_minor+amount WHERE id=(d->>'id')::uuid AND person_id=p_person AND state='VERIFIED_SYNTHETIC';IF NOT FOUND THEN RAISE EXCEPTION 'Verified synthetic source required';END IF;
   batch:=core.consumer_post(target,'SIMULATED_DEPOSIT','4f1a780b-3e31-48ef-a294-000000000021',bank,amount);result:=jsonb_build_object('id',target,'batchId',batch,'state','POSTED','dataSource','FINITE_LOCAL_SIMULATOR_ALLOWANCE');
  ELSE
   SELECT * INTO STRICT c FROM core.consumer_cards WHERE id=(d->>'id')::uuid AND person_id=p_person FOR UPDATE;
   IF c.frozen OR NOT c.online_enabled OR length(d->>'merchant') NOT BETWEEN 1 AND 64 OR length(d->>'category') NOT BETWEEN 1 AND 32 THEN RAISE EXCEPTION 'Active card, merchant and category required';END IF;
   PERFORM core.consumer_account(c.account_id,p_person);PERFORM core.consumer_approval(p_actor,p_person,(p_data->>'authorizationId')::uuid,p_action);amount:=core.consumer_minor(d->>'amountMinor');batch:=core.consumer_post(target,'SIMULATED_CARD_PURCHASE',c.account_id,'4f1a780b-3e31-48ef-a294-000000000021',amount);INSERT INTO core.consumer_metadata VALUES(c.account_id,batch,p_person,d->>'category',d->>'merchant',NULL);
   SELECT * INTO r FROM core.consumer_roundups WHERE person_id=p_person AND card_id=c.id AND enabled;
   IF FOUND THEN
    spare:=((100-amount%100)%100)*r.multiplier;SELECT greatest(0,r.weekly_cap-coalesce(sum(amount_minor),0))::bigint INTO remaining FROM core.consumer_roundup_facts WHERE person_id=p_person AND created_at>=date_trunc('week',clock_timestamp());spare:=least(spare,remaining);
    IF spare>0 THEN BEGIN round_batch:=core.consumer_post(gen_random_uuid(),'SPARE_CHANGE',c.account_id,r.vault_id,spare);INSERT INTO core.consumer_roundup_facts VALUES(target,p_person,spare,round_batch,clock_timestamp());EXCEPTION WHEN OTHERS THEN round_batch:=NULL;PERFORM core.consumer_notify(p_person,'Savings','Round-up needs attention','The purchase posted; the separate saving did not.','/save');END;END IF;
   END IF;result:=jsonb_build_object('id',target,'batchId',batch,'roundupBatchId',round_batch,'state','POSTED');
  END IF;
 ELSIF p_action='STATEMENT' THEN
  bank:=(p_data->>'accountId')::uuid;PERFORM 1 FROM core.account_relationships WHERE account_id=bank AND person_id=p_person AND kind='OWNER' AND revoked_at IS NULL FOR SHARE;IF NOT FOUND THEN RAISE EXCEPTION 'Account unavailable';END IF;
  IF (p_data->>'from')::date>=(p_data->>'to')::date OR (p_data->>'to')::date-(p_data->>'from')::date>366 THEN RAISE EXCEPTION 'Bounded statement period required';END IF;
  SELECT coalesce(jsonb_agg(jsonb_build_object('batchId',q.id,'date',q.business_date,'kind',q.posting_kind,'deltaMinor',q.delta::text,'originalBatchId',q.original_batch) ORDER BY q.business_date,q.id),'[]'::jsonb),count(*) INTO facts,fact_count FROM(SELECT j.*,e.delta FROM core.journal_entries e JOIN core.journal_batches j ON j.id=e.batch_id WHERE e.account_id=bank AND j.business_date>=(p_data->>'from')::date AND j.business_date<(p_data->>'to')::date ORDER BY j.business_date,j.id LIMIT 1001) q;
  IF fact_count>1000 THEN RAISE EXCEPTION 'Statement exceeds 1000 source facts; narrow the period';END IF;
  d:=jsonb_build_object('id',target,'accountId',bank,'from',p_data->>'from','to',p_data->>'to','source','CORE_AUTHORITATIVE','label','SYNTHETIC_TST_NOT_A_TAX_FORM','items',facts,'openingMinor',(SELECT coalesce(sum(delta::numeric),0)::text FROM core.journal_entries e JOIN core.journal_batches j ON j.id=e.batch_id WHERE e.account_id=bank AND j.business_date<(p_data->>'from')::date),'closingMinor',(SELECT coalesce(sum(delta::numeric),0)::text FROM core.journal_entries e JOIN core.journal_batches j ON j.id=e.batch_id WHERE e.account_id=bank AND j.business_date<(p_data->>'to')::date),'createdAt',clock_timestamp());INSERT INTO core.consumer_statements VALUES(target,p_person,d,encode(sha256(convert_to(core.periodic_policy_canonical(d),'UTF8')),'hex'),clock_timestamp());result:=jsonb_build_object('id',target,'state','READY','digest',encode(sha256(convert_to(core.periodic_policy_canonical(d),'UTF8')),'hex'));
 END IF;
 INSERT INTO core.consumer_commands VALUES(target,p_actor->>'issuer',p_actor->>'subject',p_actor->>'clientId',p_key,p_person,p_action,p_data,result,clock_timestamp());RETURN result;
END $$;
CREATE FUNCTION core.consumer_snapshot(p_actor jsonb,p_person uuid) RETURNS jsonb LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,core AS $$ DECLARE result jsonb;BEGIN
 result:=core.consumer_base_snapshot(p_actor,p_person);
 RETURN result||jsonb_build_object('roundups',(SELECT jsonb_build_object('cardId',card_id,'vaultId',vault_id,'multiplier',multiplier,'weeklyCapMinor',weekly_cap::text,'enabled',enabled,'savedMinor',(SELECT coalesce(sum(amount_minor),0)::text FROM core.consumer_roundup_facts WHERE person_id=p_person)) FROM core.consumer_roundups WHERE person_id=p_person),
 'groupWithdrawals',coalesce((SELECT jsonb_agg(jsonb_build_object('id',w.id,'vaultId',w.vault_id,'amountMinor',w.amount_minor::text,'state',w.state,'batchId',w.batch_id,'isRequester',w.person_id=p_person,'approved',EXISTS(SELECT 1 FROM core.consumer_group_approvals WHERE withdrawal_id=w.id AND person_id=p_person))) FROM core.consumer_group_withdrawals w JOIN core.consumer_members m ON m.vault_id=w.vault_id AND m.person_id=p_person AND m.state='ACTIVE'),'[]'::jsonb),
 'statements',coalesce((SELECT jsonb_agg(jsonb_build_object('id',id,'accountId',document->>'accountId','from',document->>'from','to',document->>'to','digest',digest,'createdAt',created_at)) FROM core.consumer_statements WHERE person_id=p_person),'[]'::jsonb));END $$;
CREATE FUNCTION core.consumer_statement(p_actor jsonb,p_person uuid,p_id uuid) RETURNS jsonb LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,core AS $$ DECLARE r core.consumer_statements;BEGIN PERFORM core.consumer_actor(p_actor,p_person);SELECT * INTO STRICT r FROM core.consumer_statements WHERE id=p_id AND person_id=p_person;IF r.digest IS DISTINCT FROM encode(sha256(convert_to(core.periodic_policy_canonical(r.document),'UTF8')),'hex') THEN RAISE EXCEPTION 'Statement integrity conflict';END IF;RETURN r.document||jsonb_build_object('digest',r.digest);END $$;
GRANT EXECUTE ON FUNCTION core.consumer_command(jsonb,uuid,uuid,text,jsonb),core.consumer_snapshot(jsonb,uuid),core.consumer_review(jsonb,uuid,text,jsonb),core.consumer_statement(jsonb,uuid,uuid) TO bank_core;
ALTER TABLE core.wealth_orders ADD COLUMN last_check timestamptz NOT NULL DEFAULT '-infinity';
ALTER TABLE core.wealth_orders ADD COLUMN last_error text NOT NULL DEFAULT '';
CREATE TABLE core.wealth_action_failures(action_id uuid NOT NULL REFERENCES core.wealth_corporate_actions(id),person_id uuid NOT NULL,retry_at timestamptz NOT NULL,error_code text NOT NULL,PRIMARY KEY(action_id,person_id));
REVOKE ALL ON core.wealth_action_failures FROM bank_core;GRANT SELECT ON core.wealth_action_failures TO bank_core;
CREATE TABLE core.wealth_action_observations(action_id uuid NOT NULL REFERENCES core.wealth_corporate_actions(id),person_id uuid NOT NULL,units bigint NOT NULL,amount_minor bigint NOT NULL,batch_id uuid,PRIMARY KEY(action_id,person_id));
CREATE TRIGGER immutable_wealth_action_observations BEFORE UPDATE OR DELETE OR TRUNCATE ON core.wealth_action_observations FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal();
REVOKE ALL ON core.wealth_action_observations FROM bank_core;GRANT SELECT ON core.wealth_action_observations TO bank_core;
CREATE FUNCTION core.consumer_tick() RETURNS integer LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,core AS $$
DECLARE s core.consumer_schedules;original core.consumer_authorizations;candidate record;cash uuid;operation uuid;batch uuid;funding uuid;asset core.wealth_assets;quantity bigint;cost bigint;executed integer:=0;order_row core.wealth_orders;corporate core.wealth_corporate_actions;w core.wealth_accounts;owned_units bigint;dividend bigint;flow uuid;BEGIN
 FOR candidate IN SELECT id,person_id FROM core.consumer_schedules WHERE state='ACTIVE' AND next_run<=(clock_timestamp() AT TIME ZONE 'UTC')::date AND retry_after<=clock_timestamp() ORDER BY next_run,id LIMIT 25 LOOP
  IF NOT pg_try_advisory_xact_lock(hashtextextended(candidate.person_id::text,307)) THEN CONTINUE;END IF;
  SELECT * INTO s FROM core.consumer_schedules WHERE id=candidate.id AND state='ACTIVE' FOR UPDATE SKIP LOCKED;IF NOT FOUND THEN CONTINUE;END IF;
  BEGIN
   PERFORM core.consumer_actor(s.actor_context,s.person_id);PERFORM core.consumer_account(s.source_account_id,s.person_id);
   SELECT * INTO STRICT original FROM core.consumer_authorizations WHERE id=s.authorization_id AND person_id=s.person_id AND action='SCHEDULE_CREATE';
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
   UPDATE core.consumer_schedules SET next_run=CASE s.frequency WHEN 'DAILY' THEN s.next_run+1 WHEN 'WEEKLY' THEN s.next_run+7 WHEN 'BIWEEKLY' THEN s.next_run+14 ELSE (s.next_run+interval '1 month')::date END,last_error='',retry_after=clock_timestamp() WHERE id=s.id;
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
GRANT EXECUTE ON FUNCTION core.consumer_tick() TO bank_core;
CREATE FUNCTION core.consumer_analytics(p_person uuid) RETURNS jsonb LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,core AS $$
DECLARE start_day date:=(date_trunc('month',clock_timestamp() AT TIME ZONE 'UTC'))::date;BEGIN
 RETURN (WITH movements AS(
 SELECT j.id,j.business_date,j.posting_kind,sum(e.delta::numeric) delta,max(coalesce(m.category,'Other')) category,max(coalesce(m.note,'')) merchant
 FROM core.journal_entries e JOIN core.journal_batches j ON j.id=e.batch_id JOIN core.account_relationships r ON r.account_id=e.account_id AND r.person_id=p_person AND r.kind='OWNER' AND r.revoked_at IS NULL LEFT JOIN core.consumer_metadata m ON m.batch_id=e.batch_id AND m.account_id=e.account_id
 WHERE j.business_date>=start_day-interval '3 months' AND j.business_date<start_day+interval '1 month' AND j.posting_kind NOT IN('VAULT_DEPOSIT','VAULT_WITHDRAWAL','AUTO_SAVE','SPARE_CHANGE','SHARED_WITHDRAWAL','BROKER_DEPOSIT','BROKER_WITHDRAWAL') GROUP BY j.id
 ),cats AS(SELECT category,coalesce(sum(-delta) FILTER(WHERE delta<0 AND business_date>=start_day),0) spent,coalesce(sum(-delta) FILTER(WHERE delta<0 AND business_date>=start_day-interval '1 month' AND business_date<start_day),0) previous,coalesce(sum(-delta) FILTER(WHERE delta<0 AND business_date<start_day),0)/3 average FROM movements GROUP BY category)
 SELECT jsonb_build_object('from',start_day,'to',(start_day+interval '1 month')::date,'incomeMinor',coalesce((SELECT sum(delta) FROM movements WHERE delta>0 AND business_date>=start_day),0)::text,'spentMinor',coalesce((SELECT sum(-delta) FROM movements WHERE delta<0 AND business_date>=start_day),0)::text,
 'categories',coalesce((SELECT jsonb_agg(jsonb_build_object('category',c.category,'spentMinor',trunc(c.spent)::text,'averageMinor',trunc(c.average)::text,'budgetMinor',(b.limit_minor+CASE WHEN b.rollover THEN greatest(0,b.limit_minor-c.previous) ELSE 0 END)::text)) FROM cats c LEFT JOIN core.consumer_budgets b ON b.person_id=p_person AND b.category=c.category),'[]'::jsonb),
 'merchants',coalesce((SELECT jsonb_agg(x) FROM(SELECT jsonb_build_object('name',coalesce(nullif(merchant,''),'Unspecified merchant'),'spentMinor',sum(-delta)::text,'visits',count(*)) x FROM movements WHERE delta<0 AND business_date>=start_day GROUP BY merchant ORDER BY sum(-delta) DESC LIMIT 10) m),'[]'::jsonb)));
END $$;
CREATE FUNCTION core.consumer_token_read(p_secret text,p_scope text) RETURNS jsonb LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,core AS $$
DECLARE t core.consumer_tokens;d jsonb;BEGIN
 IF length(p_secret)<>72 THEN RAISE EXCEPTION 'Token unavailable';END IF;SELECT * INTO STRICT t FROM core.consumer_tokens WHERE secret_hash=encode(sha256(convert_to(p_secret,'UTF8')),'hex') AND scope=p_scope AND revoked_at IS NULL AND expires_at>clock_timestamp();PERFORM core.consumer_actor(t.actor_context,t.person_id);
 IF p_scope='portfolio:read' THEN d:=core.wealth_snapshot(t.actor_context,t.person_id);RETURN jsonb_build_object('account',d->'account','positions',d->'positions','dataSource',d->'dataSource');END IF;
 d:=core.consumer_snapshot(t.actor_context,t.person_id);RETURN jsonb_build_object('scope',p_scope,'items',CASE WHEN p_scope='accounts:read' THEN d->'accounts' ELSE d->'activity' END);END $$;
GRANT EXECUTE ON FUNCTION core.consumer_token_read(text,text) TO bank_core;
-- The native analytical report is independent of the bounded recent-activity list.
ALTER FUNCTION core.consumer_snapshot(jsonb,uuid) RENAME TO consumer_snapshot_v2;
REVOKE EXECUTE ON FUNCTION core.consumer_snapshot_v2(jsonb,uuid) FROM bank_core;
CREATE FUNCTION core.consumer_snapshot(p_actor jsonb,p_person uuid) RETURNS jsonb LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,core AS $$ BEGIN RETURN core.consumer_snapshot_v2(p_actor,p_person)||jsonb_build_object('analytics',core.consumer_analytics(p_person));END $$;
GRANT EXECUTE ON FUNCTION core.consumer_snapshot(jsonb,uuid) TO bank_core;

CREATE TABLE core.consumer_signals(person_id uuid NOT NULL,signal_key text NOT NULL,created_at timestamptz NOT NULL DEFAULT clock_timestamp(),PRIMARY KEY(person_id,signal_key));
CREATE TABLE core.wealth_price_observations(symbol text NOT NULL REFERENCES core.wealth_assets(symbol),observed_at timestamptz NOT NULL,price_minor bigint NOT NULL CHECK(price_minor>0),source text NOT NULL CHECK(source='SYNTHETIC_PRICE_FIXTURE'),PRIMARY KEY(symbol,observed_at));
CREATE TRIGGER immutable_wealth_price_observations BEFORE UPDATE OR DELETE OR TRUNCATE ON core.wealth_price_observations FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal();
REVOKE ALL ON core.consumer_signals,core.wealth_price_observations FROM bank_core;GRANT SELECT ON core.consumer_signals,core.wealth_price_observations TO bank_core;
CREATE FUNCTION core.consumer_signals_tick() RETURNS void LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,core AS $$
DECLARE b core.consumer_budgets;w record;a jsonb;s jsonb;bound numeric;spent numeric;k text;BEGIN
 INSERT INTO core.wealth_price_observations SELECT symbol,date_trunc('minute',clock_timestamp()),price_minor,'SYNTHETIC_PRICE_FIXTURE' FROM core.wealth_assets ON CONFLICT DO NOTHING;
 FOR b IN SELECT x.* FROM core.consumer_budgets x WHERE NOT EXISTS(SELECT 1 FROM core.consumer_signals WHERE person_id=x.person_id AND signal_key='budget:'||x.category||':'||to_char(clock_timestamp() AT TIME ZONE 'UTC','YYYY-MM')) ORDER BY person_id,category LIMIT 100 LOOP
  a:=core.consumer_analytics(b.person_id);SELECT value INTO s FROM jsonb_array_elements(a->'categories') WHERE value->>'category'=b.category;
  spent:=coalesce((s->>'spentMinor')::numeric,0);bound:=coalesce((s->>'budgetMinor')::numeric,b.limit_minor);k:='budget:'||b.category||':'||to_char(clock_timestamp() AT TIME ZONE 'UTC','YYYY-MM');
  IF spent*100>=bound*b.threshold THEN INSERT INTO core.consumer_signals(person_id,signal_key) VALUES(b.person_id,k) ON CONFLICT DO NOTHING;IF FOUND THEN PERFORM core.consumer_notify(b.person_id,'Transactions','Budget threshold reached',b.category||' spending reached your configured threshold.','/spend');END IF;END IF;
 END LOOP;
 FOR w IN SELECT x.person_id,x.symbol,x.alert_minor,a.price_minor FROM core.wealth_watchlist x JOIN core.wealth_assets a USING(symbol) WHERE x.alert_minor IS NOT NULL AND a.price_minor>=x.alert_minor AND NOT EXISTS(SELECT 1 FROM core.consumer_signals WHERE person_id=x.person_id AND signal_key='price:'||x.symbol||':'||x.alert_minor) LIMIT 100 LOOP
  INSERT INTO core.consumer_signals(person_id,signal_key) VALUES(w.person_id,'price:'||w.symbol||':'||w.alert_minor) ON CONFLICT DO NOTHING;IF FOUND THEN PERFORM core.consumer_notify(w.person_id,'Investments','Synthetic price alert',w.symbol||' fixture price reached the saved threshold.','/invest/asset/'||w.symbol);END IF;
 END LOOP;
 FOR w IN SELECT x.* FROM core.consumer_bills x WHERE state='ACTIVE' AND next_date BETWEEN (clock_timestamp() AT TIME ZONE 'UTC')::date AND (clock_timestamp() AT TIME ZONE 'UTC')::date+7 AND NOT EXISTS(SELECT 1 FROM core.consumer_signals WHERE person_id=x.person_id AND signal_key='bill:'||x.id||':'||x.next_date) LIMIT 100 LOOP
  INSERT INTO core.consumer_signals(person_id,signal_key) VALUES(w.person_id,'bill:'||w.id||':'||w.next_date) ON CONFLICT DO NOTHING;IF FOUND THEN PERFORM core.consumer_notify(w.person_id,'Transactions','Bill reminder',w.merchant||' is due '||w.next_date||'; this reminder does not debit money.','/spend');END IF;
 END LOOP;
END $$;
ALTER FUNCTION core.consumer_snapshot(jsonb,uuid) RENAME TO consumer_snapshot_v3;
REVOKE EXECUTE ON FUNCTION core.consumer_snapshot_v3(jsonb,uuid) FROM bank_core;
CREATE FUNCTION core.consumer_snapshot(p_actor jsonb,p_person uuid) RETURNS jsonb LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,core AS $$ BEGIN
 RETURN core.consumer_snapshot_v3(p_actor,p_person)||jsonb_build_object('vaultActivity',coalesce((SELECT jsonb_agg(q ORDER BY q->>'at' DESC) FROM(SELECT jsonb_build_object('vaultId',e.account_id,'batchId',j.id,'accountId',e.account_id,'date',j.business_date,'at',j.created_at,'kind',j.posting_kind,'deltaMinor',e.delta::text,'category','Savings','note','') q FROM core.journal_entries e JOIN core.journal_batches j ON j.id=e.batch_id JOIN core.consumer_members m ON m.vault_id=e.account_id AND m.person_id=p_person AND m.state='ACTIVE' ORDER BY j.created_at DESC,j.id LIMIT 100) t),'[]'::jsonb));END $$;
GRANT EXECUTE ON FUNCTION core.consumer_snapshot(jsonb,uuid) TO bank_core;
ALTER FUNCTION core.wealth_snapshot(jsonb,uuid) RENAME TO wealth_snapshot_v1;
REVOKE EXECUTE ON FUNCTION core.wealth_snapshot_v1(jsonb,uuid) FROM bank_core;
CREATE FUNCTION core.wealth_snapshot(p_actor jsonb,p_person uuid) RETURNS jsonb LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,core AS $$ BEGIN
 RETURN core.wealth_snapshot_v1(p_actor,p_person)||jsonb_build_object('priceHistory',coalesce((SELECT jsonb_agg(jsonb_build_object('symbol',symbol,'at',observed_at,'priceMinor',price_minor::text) ORDER BY observed_at) FROM(SELECT * FROM core.wealth_price_observations WHERE observed_at>=clock_timestamp()-interval '7 days' ORDER BY observed_at DESC,symbol LIMIT 700) x),'[]'::jsonb),'realizedMinor',(SELECT coalesce(sum(realized_minor::numeric),0)::text FROM core.wealth_positions WHERE person_id=p_person),'upcomingDividends',coalesce((SELECT jsonb_agg(jsonb_build_object('symbol',symbol,'recordAt',record_at,'payAt',pay_at,'perShareMinor',per_share_minor::text)) FROM core.wealth_corporate_actions WHERE pay_at>clock_timestamp()),'[]'::jsonb));END $$;
GRANT EXECUTE ON FUNCTION core.wealth_snapshot(jsonb,uuid) TO bank_core;
