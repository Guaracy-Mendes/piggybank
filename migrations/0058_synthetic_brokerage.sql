-- Synthetic securities: integer microshares, protected cash and immutable native fills.
CREATE TABLE core.wealth_accounts(person_id uuid PRIMARY KEY,cash_account_id uuid NOT NULL UNIQUE REFERENCES core.ledger_accounts(id),actor_context jsonb NOT NULL,accepted_at timestamptz NOT NULL DEFAULT clock_timestamp());
CREATE TABLE core.wealth_assets(symbol text PRIMARY KEY,name text NOT NULL,class text NOT NULL,sector text NOT NULL,price_minor bigint NOT NULL CHECK(price_minor>0),dividend_bps integer NOT NULL DEFAULT 0,description text NOT NULL);
INSERT INTO core.wealth_assets VALUES
('AAPL','Apple Inc.','Stocks','Technology',23480,50,'Synthetic technology equity fixture.'),('NVDA','NVIDIA Corp.','Stocks','Semiconductors',14860,10,'Synthetic semiconductor equity fixture.'),('MSFT','Microsoft Corp.','Stocks','Technology',44152,70,'Synthetic software equity fixture.'),('VOO','S&P 500 ETF fixture','ETFs','Broad market',51955,130,'Synthetic diversified fund; no live fund exposure.'),('VTI','Total market ETF fixture','ETFs','Broad market',28420,140,'Synthetic broad market fund.'),('US10Y','Treasury note fixture','Bonds','Government',10000,435,'Synthetic fixed income instrument; not a treasury obligation.'),('BTC','Bitcoin fixture','Crypto','Digital assets',9124500,0,'Synthetic digital asset; no blockchain custody.');
CREATE TABLE core.wealth_positions(person_id uuid NOT NULL REFERENCES core.wealth_accounts(person_id),symbol text NOT NULL REFERENCES core.wealth_assets(symbol),units bigint NOT NULL DEFAULT 0 CHECK(units>=0),reserved_units bigint NOT NULL DEFAULT 0 CHECK(reserved_units>=0 AND reserved_units<=units),cost_minor bigint NOT NULL DEFAULT 0 CHECK(cost_minor>=0),realized_minor bigint NOT NULL DEFAULT 0,PRIMARY KEY(person_id,symbol));
CREATE TABLE core.wealth_position_events(operation_id uuid PRIMARY KEY,person_id uuid NOT NULL,symbol text NOT NULL,units_delta bigint NOT NULL,cost_delta bigint NOT NULL,created_at timestamptz NOT NULL DEFAULT clock_timestamp());
CREATE TRIGGER immutable_wealth_position_events BEFORE UPDATE OR DELETE OR TRUNCATE ON core.wealth_position_events FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal();
REVOKE ALL ON core.wealth_position_events FROM bank_core;GRANT SELECT ON core.wealth_position_events TO bank_core;
CREATE TABLE core.wealth_orders(id uuid PRIMARY KEY,person_id uuid NOT NULL REFERENCES core.wealth_accounts(person_id),authorization_id uuid NOT NULL UNIQUE REFERENCES core.consumer_authorizations(id),symbol text NOT NULL REFERENCES core.wealth_assets(symbol),side text NOT NULL CHECK(side IN('BUY','SELL')),order_type text NOT NULL CHECK(order_type IN('MARKET','LIMIT')),units bigint NOT NULL CHECK(units>0),limit_minor bigint NOT NULL CHECK(limit_minor>0),reserved_minor bigint NOT NULL DEFAULT 0,state text NOT NULL CHECK(state IN('OPEN','FILLED','CANCELLED','EXPIRED')),expires_at timestamptz NOT NULL,created_at timestamptz NOT NULL DEFAULT clock_timestamp());
CREATE TABLE core.wealth_fills(order_id uuid PRIMARY KEY REFERENCES core.wealth_orders(id),symbol text NOT NULL,units bigint NOT NULL,price_minor bigint NOT NULL,amount_minor bigint NOT NULL,batch_id uuid NOT NULL UNIQUE REFERENCES core.journal_batches(id),filled_at timestamptz NOT NULL DEFAULT clock_timestamp());
CREATE TRIGGER immutable_wealth_fills BEFORE UPDATE OR DELETE OR TRUNCATE ON core.wealth_fills FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal();
CREATE TABLE core.wealth_watchlist(person_id uuid NOT NULL,symbol text NOT NULL REFERENCES core.wealth_assets(symbol),alert_minor bigint,drip boolean NOT NULL DEFAULT false,PRIMARY KEY(person_id,symbol));
CREATE TABLE core.wealth_observations(id uuid PRIMARY KEY DEFAULT gen_random_uuid(),person_id uuid NOT NULL,cash_minor bigint NOT NULL,equity_minor bigint NOT NULL,net_flows_minor bigint NOT NULL,observed_at timestamptz NOT NULL DEFAULT clock_timestamp());
CREATE TABLE core.wealth_cash_flows(id uuid PRIMARY KEY,person_id uuid NOT NULL,amount_minor bigint NOT NULL,batch_id uuid NOT NULL UNIQUE REFERENCES core.journal_batches(id),created_at timestamptz NOT NULL DEFAULT clock_timestamp());
CREATE TRIGGER immutable_wealth_cash_flows BEFORE UPDATE OR DELETE OR TRUNCATE ON core.wealth_cash_flows FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal();
CREATE TABLE core.wealth_dividends(id uuid PRIMARY KEY DEFAULT gen_random_uuid(),person_id uuid NOT NULL,symbol text NOT NULL,record_date date NOT NULL,units bigint NOT NULL,amount_minor bigint NOT NULL,batch_id uuid NOT NULL REFERENCES core.journal_batches(id),created_at timestamptz NOT NULL DEFAULT clock_timestamp(),UNIQUE(person_id,symbol,record_date));
CREATE TRIGGER immutable_wealth_dividends BEFORE UPDATE OR DELETE OR TRUNCATE ON core.wealth_dividends FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal();
CREATE TABLE core.consumer_schedules(id uuid PRIMARY KEY,person_id uuid NOT NULL,actor_context jsonb NOT NULL,authorization_id uuid NOT NULL UNIQUE REFERENCES core.consumer_authorizations(id),kind text NOT NULL CHECK(kind IN('SAVE','INVEST')),source_account_id uuid NOT NULL REFERENCES core.account_registry(id),vault_id uuid REFERENCES core.consumer_vaults(id),symbol text REFERENCES core.wealth_assets(symbol),amount_minor bigint NOT NULL CHECK(amount_minor>=500),frequency text NOT NULL CHECK(frequency IN('DAILY','WEEKLY','BIWEEKLY','MONTHLY')),next_run date NOT NULL,state text NOT NULL DEFAULT 'ACTIVE' CHECK(state IN('ACTIVE','PAUSED','CANCELLED')),last_error text NOT NULL DEFAULT '',retry_after timestamptz NOT NULL DEFAULT clock_timestamp(),created_at timestamptz NOT NULL DEFAULT clock_timestamp());
CREATE TABLE core.consumer_schedule_runs(schedule_id uuid NOT NULL REFERENCES core.consumer_schedules(id),run_date date NOT NULL,operation_id uuid NOT NULL UNIQUE,batch_id uuid NOT NULL REFERENCES core.journal_batches(id),created_at timestamptz NOT NULL DEFAULT clock_timestamp(),PRIMARY KEY(schedule_id,run_date));
CREATE TRIGGER immutable_consumer_schedule_runs BEFORE UPDATE OR DELETE OR TRUNCATE ON core.consumer_schedule_runs FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal();
DO $$ DECLARE t text;BEGIN FOREACH t IN ARRAY ARRAY['wealth_accounts','wealth_assets','wealth_positions','wealth_orders','wealth_fills','wealth_watchlist','wealth_observations','wealth_cash_flows','wealth_dividends','consumer_schedules','consumer_schedule_runs'] LOOP EXECUTE format('REVOKE ALL ON core.%I FROM bank_core',t);EXECUTE format('GRANT SELECT ON core.%I TO bank_core',t);END LOOP;END $$;
CREATE FUNCTION core.wealth_observe(p_person uuid) RETURNS void LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,core AS $$ BEGIN
 INSERT INTO core.wealth_observations(person_id,cash_minor,equity_minor,net_flows_minor) SELECT p_person,c.booked,coalesce((SELECT sum(trunc(p.units::numeric*a.price_minor/1000000)) FROM core.wealth_positions p JOIN core.wealth_assets a ON a.symbol=p.symbol WHERE p.person_id=p_person),0)::bigint,coalesce((SELECT sum(amount_minor::numeric) FROM core.wealth_cash_flows WHERE person_id=p_person),0)::bigint FROM core.wealth_accounts w JOIN core.ledger_accounts c ON c.id=w.cash_account_id WHERE w.person_id=p_person;
END $$;
CREATE FUNCTION core.wealth_owner(p_person uuid) RETURNS uuid LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,core AS $$ DECLARE a uuid;BEGIN SELECT cash_account_id INTO STRICT a FROM core.wealth_accounts WHERE person_id=p_person;PERFORM 1 FROM core.ledger_accounts WHERE id=a AND state='ACTIVE' AND NOT debit_blocked AND NOT credit_blocked FOR UPDATE;IF NOT FOUND THEN RAISE EXCEPTION 'Active brokerage cash required';END IF;RETURN a;END $$;
CREATE FUNCTION core.wealth_quote(p_actor jsonb,p_person uuid,p_data jsonb) RETURNS jsonb LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,core AS $$
DECLARE asset core.wealth_assets;cash uuid;q_units numeric;price bigint;amount numeric;details jsonb;BEGIN
 PERFORM core.consumer_actor(p_actor,p_person);PERFORM pg_advisory_xact_lock(hashtextextended(p_person::text,307));cash:=core.wealth_owner(p_person);
 IF EXISTS(SELECT 1 FROM jsonb_object_keys(p_data) k WHERE k<>ALL(ARRAY['symbol','side','orderType','mode','amountMinor','quantityUnits','limitMinor','timeInForce'])) OR p_data->>'side' NOT IN('BUY','SELL') OR p_data->>'orderType' NOT IN('MARKET','LIMIT') OR p_data->>'mode' NOT IN('AMOUNT','UNITS') OR p_data->>'timeInForce' NOT IN('DAY','GTC') THEN RAISE EXCEPTION 'Supported exact trade fields required';END IF;
 SELECT * INTO STRICT asset FROM core.wealth_assets WHERE symbol=p_data->>'symbol';price:=CASE WHEN p_data->>'orderType'='LIMIT' THEN core.consumer_minor(p_data->>'limitMinor') ELSE asset.price_minor END;
 IF p_data->>'mode'='AMOUNT' THEN q_units:=trunc(core.consumer_minor(p_data->>'amountMinor')::numeric*1000000/price);ELSE q_units:=core.consumer_minor(p_data->>'quantityUnits');END IF;
 amount:=CASE WHEN p_data->>'side'='BUY' THEN ceil(q_units*price/1000000) ELSE trunc(q_units*price/1000000) END;IF q_units<1 OR q_units>9223372036854775807 OR amount<1 OR amount>9223372036854775807 THEN RAISE EXCEPTION 'Representable positive trade required';END IF;
 IF p_data->>'side'='BUY' AND (SELECT booked::numeric-held FROM core.ledger_accounts WHERE id=cash)<amount THEN RAISE EXCEPTION 'Insufficient buying power';END IF;
 IF p_data->>'side'='SELL' AND coalesce((SELECT p.units-p.reserved_units FROM core.wealth_positions p WHERE person_id=p_person AND symbol=asset.symbol),0)<q_units THEN RAISE EXCEPTION 'Insufficient unreserved shares';END IF;
 details:=jsonb_build_object('symbol',asset.symbol,'name',asset.name,'side',p_data->>'side','orderType',p_data->>'orderType','quantityUnits',q_units::text,'priceMinor',price::text,'amountMinor',amount::text,'feeMinor','0','timeInForce',p_data->>'timeInForce','cashAccountId',cash,'dataSource','SYNTHETIC_PRICE_FIXTURE','notice','Simulated trading only. TST has no monetary value. No FINRA/SIPC membership or insurance is provided. No margin, short selling or real execution.');
 RETURN core.consumer_authorize(p_actor,p_person,'TRADE',details);
END $$;
CREATE FUNCTION core.wealth_fill(p_order uuid) RETURNS jsonb LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,core AS $$
DECLARE o core.wealth_orders;a core.wealth_assets;cash uuid;cost bigint;basis bigint;batch uuid;hold uuid;BEGIN
 SELECT * INTO STRICT o FROM core.wealth_orders WHERE id=p_order;
 PERFORM pg_advisory_xact_lock(hashtextextended(o.person_id::text,307));
 SELECT * INTO STRICT o FROM core.wealth_orders WHERE id=p_order FOR UPDATE;
 IF o.state<>'OPEN' THEN RETURN jsonb_build_object('id',o.id,'state',o.state,'fill',(SELECT jsonb_build_object('batchId',batch_id,'amountMinor',amount_minor::text,'priceMinor',price_minor::text,'quantityUnits',units::text) FROM core.wealth_fills WHERE order_id=o.id));END IF;
 PERFORM core.consumer_actor((SELECT actor_context FROM core.consumer_authorizations WHERE id=o.authorization_id),o.person_id);cash:=core.wealth_owner(o.person_id);
 SELECT * INTO STRICT a FROM core.wealth_assets WHERE symbol=o.symbol;
 IF (o.side='BUY' AND a.price_minor>o.limit_minor) OR (o.side='SELL' AND a.price_minor<o.limit_minor) THEN RETURN jsonb_build_object('id',o.id,'state','OPEN');END IF;
 cost:=CASE WHEN o.side='BUY' THEN ceil(o.units::numeric*a.price_minor/1000000)::bigint ELSE trunc(o.units::numeric*a.price_minor/1000000)::bigint END;IF cost<1 THEN RAISE EXCEPTION 'Fill below native minor unit';END IF;
 INSERT INTO core.wealth_positions(person_id,symbol) VALUES(o.person_id,o.symbol) ON CONFLICT DO NOTHING;
 PERFORM 1 FROM core.wealth_positions WHERE person_id=o.person_id AND symbol=o.symbol FOR UPDATE;
 IF o.side='BUY' THEN
  -- A favorable limit fill may be below the reservation. Release only this local
  -- unsubmitted reservation, then let Core recheck all availability/restrictions.
  PERFORM core.release_hold(o.id);batch:=core.consumer_post(o.id,'SECURITY_BUY',cash,'4f1a780b-3e31-48ef-a294-000000000020',cost);
  UPDATE core.wealth_positions SET units=units+o.units,cost_minor=cost_minor+cost WHERE person_id=o.person_id AND symbol=o.symbol;
 ELSE
  SELECT trunc(cost_minor::numeric*o.units/units)::bigint INTO basis FROM core.wealth_positions WHERE person_id=o.person_id AND symbol=o.symbol AND units>=o.units AND reserved_units>=o.units;
  IF basis IS NULL THEN RAISE EXCEPTION 'Native share reservation required';END IF;
  batch:=core.consumer_post(o.id,'SECURITY_SELL','4f1a780b-3e31-48ef-a294-000000000020',cash,cost);
  UPDATE core.wealth_positions SET units=units-o.units,reserved_units=reserved_units-o.units,cost_minor=cost_minor-basis,realized_minor=realized_minor+cost-basis WHERE person_id=o.person_id AND symbol=o.symbol;
 END IF;
 INSERT INTO core.wealth_position_events VALUES(o.id,o.person_id,o.symbol,CASE WHEN o.side='BUY' THEN o.units ELSE -o.units END,CASE WHEN o.side='BUY' THEN cost ELSE -basis END,clock_timestamp());
 INSERT INTO core.wealth_fills VALUES(o.id,o.symbol,o.units,a.price_minor,cost,batch,clock_timestamp());UPDATE core.wealth_orders SET state='FILLED' WHERE id=o.id;
 PERFORM core.consumer_notify(o.person_id,'Investments',o.side||' '||o.symbol||' filled','Your simulated trade has a native financial receipt.','/invest');PERFORM core.wealth_observe(o.person_id);
 RETURN jsonb_build_object('id',o.id,'state','FILLED','batchId',batch,'quantityUnits',o.units::text,'priceMinor',a.price_minor::text,'amountMinor',cost::text);
END $$;
CREATE FUNCTION core.wealth_command(p_actor jsonb,p_person uuid,p_key uuid,p_action text,p_data jsonb) RETURNS jsonb LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,core AS $$
DECLARE old core.consumer_commands;target uuid:=gen_random_uuid();cash uuid;bank uuid;amount bigint;batch uuid;d jsonb;asset_row core.wealth_assets;o core.wealth_orders;result jsonb;allowed text[];BEGIN
 PERFORM core.consumer_actor(p_actor,p_person);IF p_key IS NULL OR p_key='00000000-0000-0000-0000-000000000000' THEN RAISE EXCEPTION 'Request key required';END IF;
 PERFORM pg_advisory_xact_lock(hashtextextended(p_person::text,307));SELECT * INTO old FROM core.consumer_commands WHERE issuer=p_actor->>'issuer' AND subject=p_actor->>'subject' AND client_id=p_actor->>'clientId' AND request_key=p_key;
 IF FOUND THEN IF old.person_id<>p_person OR old.action<>p_action OR old.input<>p_data THEN RAISE EXCEPTION 'Command scope conflict';END IF;RETURN old.result;END IF;
 allowed:=CASE p_action WHEN 'BROKER_OPEN' THEN ARRAY['accepted'] WHEN 'BROKER_DEPOSIT' THEN ARRAY['accountId','amountMinor'] WHEN 'BROKER_WITHDRAW' THEN ARRAY['authorizationId'] WHEN 'TRADE_EXECUTE' THEN ARRAY['authorizationId'] WHEN 'ORDER_CANCEL' THEN ARRAY['id'] WHEN 'WATCHLIST' THEN ARRAY['symbol','watch','alertMinor','drip'] WHEN 'SCHEDULE_CREATE' THEN ARRAY['authorizationId'] WHEN 'SCHEDULE_UPDATE' THEN ARRAY['id','state'] ELSE NULL END;
 IF allowed IS NULL OR jsonb_typeof(p_data)<>'object' OR EXISTS(SELECT 1 FROM jsonb_object_keys(p_data) k WHERE k<>ALL(allowed)) THEN RAISE EXCEPTION 'Unsupported command fields';END IF;
 IF p_action='BROKER_OPEN' THEN
  IF (p_data->>'accepted')::boolean IS DISTINCT FROM true OR NOT EXISTS(SELECT 1 FROM core.account_relationships r JOIN core.ledger_accounts a ON a.id=r.account_id WHERE r.person_id=p_person AND r.kind='OWNER' AND r.revoked_at IS NULL AND a.state='ACTIVE') THEN RAISE EXCEPTION 'Active owned bank account and explicit synthetic brokerage consent required';END IF;
  SELECT cash_account_id INTO cash FROM core.wealth_accounts WHERE person_id=p_person;
  IF cash IS NULL THEN INSERT INTO core.ledger_accounts(id,kind,state,debit_blocked) VALUES(target,'CUSTOMER','ACTIVE',false);INSERT INTO core.wealth_accounts VALUES(p_person,target,p_actor,clock_timestamp());cash:=target;PERFORM core.wealth_observe(p_person);END IF;result:=jsonb_build_object('id',cash,'state','ACTIVE_SYNTHETIC');
 ELSIF p_action IN('BROKER_DEPOSIT','BROKER_WITHDRAW') THEN
  cash:=core.wealth_owner(p_person);
  IF p_action='BROKER_WITHDRAW' THEN d:=core.consumer_approval(p_actor,p_person,(p_data->>'authorizationId')::uuid,'BROKER_WITHDRAW');ELSE d:=p_data;END IF;
  bank:=(d->>'accountId')::uuid;PERFORM core.consumer_account(bank,p_person);amount:=core.consumer_minor(d->>'amountMinor');
  IF p_action='BROKER_WITHDRAW' THEN PERFORM core.consumer_approval(p_actor,p_person,(p_data->>'authorizationId')::uuid,'BROKER_WITHDRAW');END IF;
  IF p_action='BROKER_DEPOSIT' THEN batch:=core.consumer_post(target,'BROKER_DEPOSIT',bank,cash,amount);ELSE batch:=core.consumer_post(target,'BROKER_WITHDRAWAL',cash,bank,amount);amount:=-amount;END IF;
  INSERT INTO core.wealth_cash_flows VALUES(target,p_person,amount,batch,clock_timestamp());PERFORM core.wealth_observe(p_person);result:=jsonb_build_object('id',target,'batchId',batch,'state','POSTED');
 ELSIF p_action='TRADE_EXECUTE' THEN
  d:=core.consumer_approval(p_actor,p_person,(p_data->>'authorizationId')::uuid,'TRADE');cash:=core.wealth_owner(p_person);PERFORM core.consumer_approval(p_actor,p_person,(p_data->>'authorizationId')::uuid,'TRADE');
  IF d->>'cashAccountId' IS DISTINCT FROM cash::text THEN RAISE EXCEPTION 'Quote cash scope conflict';END IF;
  IF EXISTS(SELECT 1 FROM core.wealth_orders WHERE authorization_id=(p_data->>'authorizationId')::uuid) THEN SELECT id INTO target FROM core.wealth_orders WHERE authorization_id=(p_data->>'authorizationId')::uuid;SELECT core.wealth_fill(target) INTO result;
  ELSE
   SELECT * INTO STRICT asset_row FROM core.wealth_assets WHERE symbol=d->>'symbol';
   INSERT INTO core.wealth_orders VALUES(target,p_person,(p_data->>'authorizationId')::uuid,asset_row.symbol,d->>'side',d->>'orderType',(d->>'quantityUnits')::bigint,(d->>'priceMinor')::bigint,(d->>'amountMinor')::bigint,'OPEN',CASE WHEN d->>'timeInForce'='GTC' THEN clock_timestamp()+interval '30 days' ELSE (date_trunc('day',clock_timestamp() AT TIME ZONE 'UTC')+interval '1 day') AT TIME ZONE 'UTC' END,clock_timestamp());
   IF d->>'side'='BUY' THEN PERFORM core.reserve(target,cash,(d->>'amountMinor')::bigint);
   ELSE UPDATE core.wealth_positions SET reserved_units=reserved_units+(d->>'quantityUnits')::bigint WHERE person_id=p_person AND symbol=asset_row.symbol AND units-reserved_units>=(d->>'quantityUnits')::bigint;IF NOT FOUND THEN RAISE EXCEPTION 'Insufficient unreserved shares';END IF;END IF;
   result:=core.wealth_fill(target);
  END IF;
 ELSIF p_action='ORDER_CANCEL' THEN
  SELECT * INTO STRICT o FROM core.wealth_orders WHERE id=(p_data->>'id')::uuid AND person_id=p_person FOR UPDATE;
  IF o.state NOT IN('OPEN','CANCELLED') THEN RAISE EXCEPTION 'Only open orders can be cancelled';END IF;
  IF o.state='OPEN' THEN IF o.side='BUY' THEN PERFORM core.release_hold(o.id);ELSE UPDATE core.wealth_positions SET reserved_units=reserved_units-o.units WHERE person_id=p_person AND symbol=o.symbol;END IF;UPDATE core.wealth_orders SET state='CANCELLED' WHERE id=o.id;END IF;result:=jsonb_build_object('id',o.id,'state','CANCELLED');
 ELSIF p_action='WATCHLIST' THEN
  IF p_data?'drip' THEN RAISE EXCEPTION 'Use separately approved DRIP consent';END IF;
  SELECT * INTO STRICT asset_row FROM core.wealth_assets WHERE symbol=p_data->>'symbol';
  IF (p_data->>'watch')::boolean IS DISTINCT FROM false THEN INSERT INTO core.wealth_watchlist VALUES(p_person,asset_row.symbol,CASE WHEN p_data->>'alertMinor' IS NOT NULL THEN core.consumer_minor(p_data->>'alertMinor') ELSE NULL END,coalesce((p_data->>'drip')::boolean,false)) ON CONFLICT(person_id,symbol) DO UPDATE SET alert_minor=coalesce(EXCLUDED.alert_minor,core.wealth_watchlist.alert_minor),drip=CASE WHEN p_data?'drip' THEN EXCLUDED.drip ELSE core.wealth_watchlist.drip END;ELSE DELETE FROM core.wealth_watchlist WHERE person_id=p_person AND symbol=asset_row.symbol;END IF;result:=jsonb_build_object('symbol',asset_row.symbol,'saved',true);
 ELSIF p_action='SCHEDULE_CREATE' THEN
  d:=core.consumer_approval(p_actor,p_person,(p_data->>'authorizationId')::uuid,'SCHEDULE_CREATE');bank:=(d->>'accountId')::uuid;PERFORM core.consumer_account(bank,p_person);PERFORM core.consumer_approval(p_actor,p_person,(p_data->>'authorizationId')::uuid,'SCHEDULE_CREATE');
  IF d->>'kind' NOT IN('SAVE','INVEST') OR d->>'frequency' NOT IN('DAILY','WEEKLY','BIWEEKLY','MONTHLY') OR core.consumer_minor(d->>'amountMinor')<500 OR (d->>'nextRun')::date<(clock_timestamp() AT TIME ZONE 'UTC')::date THEN RAISE EXCEPTION 'Exact schedule amount, frequency and future date required';END IF;
  IF d->>'kind'='SAVE' THEN IF NOT EXISTS(SELECT 1 FROM core.consumer_vaults WHERE id=(d->>'vaultId')::uuid AND person_id=p_person) THEN RAISE EXCEPTION 'Owned vault required';END IF;ELSE cash:=core.wealth_owner(p_person);PERFORM 1 FROM core.wealth_assets WHERE symbol=d->>'symbol';IF NOT FOUND THEN RAISE EXCEPTION 'Supported asset required';END IF;END IF;
  PERFORM core.consumer_approval(p_actor,p_person,(p_data->>'authorizationId')::uuid,'SCHEDULE_CREATE');
  INSERT INTO core.consumer_schedules(id,person_id,actor_context,authorization_id,kind,source_account_id,vault_id,symbol,amount_minor,frequency,next_run) VALUES(target,p_person,p_actor,(p_data->>'authorizationId')::uuid,d->>'kind',bank,(d->>'vaultId')::uuid,d->>'symbol',core.consumer_minor(d->>'amountMinor'),d->>'frequency',(d->>'nextRun')::date);result:=jsonb_build_object('id',target,'state','ACTIVE');
 ELSIF p_action='SCHEDULE_UPDATE' THEN
  IF p_data->>'state' NOT IN('ACTIVE','PAUSED','CANCELLED') THEN RAISE EXCEPTION 'Supported schedule state required';END IF;
  UPDATE core.consumer_schedules SET state=p_data->>'state',next_run=CASE WHEN p_data->>'state'='ACTIVE' THEN greatest(next_run,(clock_timestamp() AT TIME ZONE 'UTC')::date) ELSE next_run END WHERE id=(p_data->>'id')::uuid AND person_id=p_person AND state<>'CANCELLED';IF NOT FOUND THEN RAISE EXCEPTION 'Schedule unavailable';END IF;result:=jsonb_build_object('saved',true);
 END IF;
 INSERT INTO core.consumer_commands VALUES(target,p_actor->>'issuer',p_actor->>'subject',p_actor->>'clientId',p_key,p_person,p_action,p_data,result,clock_timestamp());RETURN result;
END $$;
CREATE FUNCTION core.wealth_snapshot(p_actor jsonb,p_person uuid) RETURNS jsonb LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,core AS $$
DECLARE result jsonb;BEGIN PERFORM core.consumer_actor(p_actor,p_person);
 SELECT jsonb_build_object('dataSource','SYNTHETIC_PRICE_FIXTURE','quoteObservedAt',clock_timestamp(),'notice','Simulated assets, prices and trading. TST has no monetary value. No FDIC/SIPC coverage, exchange connection or regulatory membership is provided.','account',
 (SELECT jsonb_build_object('id',w.cash_account_id,'state','ACTIVE_SYNTHETIC','cashMinor',c.booked::text,'heldMinor',c.held::text,'buyingPowerMinor',(c.booked-c.held)::text,'acceptedAt',w.accepted_at) FROM core.wealth_accounts w JOIN core.ledger_accounts c ON c.id=w.cash_account_id WHERE person_id=p_person),
 'assets',(SELECT jsonb_agg(jsonb_build_object('symbol',a.symbol,'name',a.name,'class',a.class,'sector',a.sector,'priceMinor',a.price_minor::text,'dividendBps',a.dividend_bps,'description',a.description,'watched',w.symbol IS NOT NULL,'drip',coalesce(w.drip,false),'alertMinor',w.alert_minor::text) ORDER BY a.symbol) FROM core.wealth_assets a LEFT JOIN core.wealth_watchlist w ON w.person_id=p_person AND w.symbol=a.symbol),
 'positions',coalesce((SELECT jsonb_agg(jsonb_build_object('symbol',p.symbol,'units',p.units::text,'reservedUnits',p.reserved_units::text,'costMinor',p.cost_minor::text,'realizedMinor',p.realized_minor::text,'valueMinor',trunc(p.units::numeric*a.price_minor/1000000)::text,'returnMinor',(trunc(p.units::numeric*a.price_minor/1000000)-p.cost_minor)::text)) FROM core.wealth_positions p JOIN core.wealth_assets a ON a.symbol=p.symbol WHERE person_id=p_person AND p.units>0),'[]'::jsonb),
 'orders',coalesce((SELECT jsonb_agg(q ORDER BY q->>'createdAt' DESC) FROM(SELECT jsonb_build_object('id',o.id,'symbol',o.symbol,'side',o.side,'orderType',o.order_type,'units',o.units::text,'limitMinor',o.limit_minor::text,'state',o.state,'createdAt',o.created_at,'expiresAt',o.expires_at,'fill',CASE WHEN f.order_id IS NULL THEN NULL ELSE jsonb_build_object('batchId',f.batch_id,'priceMinor',f.price_minor::text,'amountMinor',f.amount_minor::text,'filledAt',f.filled_at) END) q FROM core.wealth_orders o LEFT JOIN core.wealth_fills f ON f.order_id=o.id WHERE o.person_id=p_person ORDER BY o.created_at DESC LIMIT 100) t),'[]'::jsonb),
 'observations',coalesce((SELECT jsonb_agg(q ORDER BY q->>'at') FROM(SELECT jsonb_build_object('at',observed_at,'cashMinor',cash_minor::text,'equityMinor',equity_minor::text,'netFlowsMinor',net_flows_minor::text) q FROM core.wealth_observations WHERE person_id=p_person ORDER BY observed_at DESC LIMIT 100) x),'[]'::jsonb),
 'schedules',coalesce((SELECT jsonb_agg(jsonb_build_object('id',id,'kind',kind,'accountId',source_account_id,'vaultId',vault_id,'symbol',symbol,'amountMinor',amount_minor::text,'frequency',frequency,'nextRun',next_run,'state',state,'lastError',last_error)) FROM core.consumer_schedules WHERE person_id=p_person),'[]'::jsonb),
 'dividends',coalesce((SELECT jsonb_agg(jsonb_build_object('id',id,'symbol',symbol,'recordDate',record_date,'amountMinor',amount_minor::text,'batchId',batch_id)) FROM core.wealth_dividends WHERE person_id=p_person),'[]'::jsonb)) INTO result;RETURN result;END $$;
GRANT EXECUTE ON FUNCTION core.wealth_quote(jsonb,uuid,jsonb),core.wealth_command(jsonb,uuid,uuid,text,jsonb),core.wealth_snapshot(jsonb,uuid) TO bank_core;
CREATE FUNCTION core.consumer_review(p_actor jsonb,p_person uuid,p_action text,p_data jsonb) RETURNS jsonb LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,core AS $$
DECLARE keys text[];BEGIN PERFORM core.consumer_actor(p_actor,p_person);
 keys:=CASE p_action WHEN 'CARD_REVEAL' THEN ARRAY['id'] WHEN 'TOKEN_CREATE' THEN ARRAY['label','scope'] WHEN 'BROKER_WITHDRAW' THEN ARRAY['accountId','amountMinor'] WHEN 'SCHEDULE_CREATE' THEN ARRAY['kind','accountId','vaultId','symbol','amountMinor','frequency','nextRun'] ELSE NULL END;
 IF keys IS NULL OR jsonb_typeof(p_data)<>'object' OR EXISTS(SELECT 1 FROM jsonb_object_keys(p_data) k WHERE k<>ALL(keys)) THEN RAISE EXCEPTION 'Supported action details required';END IF;
 IF p_action='CARD_REVEAL' AND NOT EXISTS(SELECT 1 FROM core.consumer_cards WHERE id=(p_data->>'id')::uuid AND person_id=p_person) THEN RAISE EXCEPTION 'Card unavailable';END IF;
 IF p_action IN('BROKER_WITHDRAW','SCHEDULE_CREATE') THEN PERFORM core.consumer_account((p_data->>'accountId')::uuid,p_person);PERFORM core.consumer_minor(p_data->>'amountMinor');END IF;
 IF p_action='BROKER_WITHDRAW' THEN PERFORM core.wealth_owner(p_person);END IF;
 IF p_action='TOKEN_CREATE' AND (length(p_data->>'label') NOT BETWEEN 1 AND 64 OR p_data->>'scope' NOT IN('accounts:read','transactions:read','portfolio:read')) THEN RAISE EXCEPTION 'Supported scoped token details required';END IF;
 RETURN core.consumer_authorize(p_actor,p_person,p_action,p_data);END $$;
GRANT EXECUTE ON FUNCTION core.consumer_review(jsonb,uuid,text,jsonb) TO bank_core;
