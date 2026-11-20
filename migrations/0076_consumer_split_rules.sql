-- owner: core (consumer workspace). Official UI increment (schema 76), local synthetic:
-- * UI-20: equal multi-party expense split (one request per participant, exact remainder) and paid-request linking to a
--   settled native internal payment of the exact amount to the requester's account;
-- * UI-22: favourite receiving references;
-- * UI-13: merchant- or category-triggered fixed savings rules, approved by the issuer like round-ups, applied once per
--   purchase within a weekly cap;
-- * UI-17: a labelled safe-to-spend figure (available balance minus approved automations and bill reminders due by
--   month end). It is a local rule, not advice or a cash-flow forecast model.
-- Core remains the only money authority: rule savings post through core.consumer_post with a deterministic operation id.

ALTER TABLE core.consumer_requests ADD COLUMN split_id uuid;
CREATE UNIQUE INDEX consumer_requests_payment ON core.consumer_requests(payment_id) WHERE payment_id IS NOT NULL;
CREATE TABLE core.consumer_splits(id uuid PRIMARY KEY,person_id uuid NOT NULL,account_id uuid NOT NULL REFERENCES core.account_registry(id),
 batch_id uuid NOT NULL REFERENCES core.journal_batches(id),total_minor bigint NOT NULL CHECK(total_minor>0),include_self boolean NOT NULL,
 participants integer NOT NULL CHECK(participants BETWEEN 1 AND 10),self_share_minor bigint NOT NULL CHECK(self_share_minor>=0),memo text NOT NULL,
 created_at timestamptz NOT NULL DEFAULT clock_timestamp(),UNIQUE(person_id,batch_id));
CREATE TRIGGER immutable_consumer_splits BEFORE UPDATE OR DELETE OR TRUNCATE ON core.consumer_splits FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal();
CREATE TABLE core.consumer_favorites(person_id uuid NOT NULL,reference text NOT NULL,label text NOT NULL,created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 PRIMARY KEY(person_id,reference));
CREATE TABLE core.consumer_save_rules(id uuid PRIMARY KEY,person_id uuid NOT NULL,authorization_id uuid NOT NULL UNIQUE REFERENCES core.consumer_authorizations(id),
 card_id uuid NOT NULL REFERENCES core.consumer_cards(id),vault_id uuid NOT NULL REFERENCES core.consumer_vaults(id),match_kind text NOT NULL CHECK(match_kind IN('MERCHANT','CATEGORY')),
 match_value text NOT NULL CHECK(length(match_value) BETWEEN 1 AND 64),amount_minor bigint NOT NULL CHECK(amount_minor>0),weekly_cap_minor bigint NOT NULL CHECK(weekly_cap_minor>=amount_minor),
 enabled boolean NOT NULL DEFAULT true,created_at timestamptz NOT NULL DEFAULT clock_timestamp());
-- Only the enabled flag may change, and only from true to false.
CREATE FUNCTION core.guard_consumer_save_rule() RETURNS trigger LANGUAGE plpgsql SET search_path=pg_catalog AS $$ BEGIN
 IF TG_OP<>'UPDATE' OR (to_jsonb(NEW)-'enabled')<>(to_jsonb(OLD)-'enabled') OR (NOT OLD.enabled AND NEW.enabled) THEN RAISE EXCEPTION 'Saving rules are append-only; disable and create a new rule';END IF;
 RETURN NEW;END $$;
CREATE TRIGGER guard_consumer_save_rules BEFORE UPDATE OR DELETE ON core.consumer_save_rules FOR EACH ROW EXECUTE FUNCTION core.guard_consumer_save_rule();
CREATE TRIGGER truncate_consumer_save_rules BEFORE TRUNCATE ON core.consumer_save_rules FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal();
CREATE TABLE core.consumer_save_rule_facts(purchase_id uuid NOT NULL,rule_id uuid NOT NULL REFERENCES core.consumer_save_rules(id),person_id uuid NOT NULL,
 amount_minor bigint NOT NULL CHECK(amount_minor>0),batch_id uuid NOT NULL UNIQUE REFERENCES core.journal_batches(id),created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 PRIMARY KEY(purchase_id,rule_id));
CREATE TRIGGER immutable_consumer_save_rule_facts BEFORE UPDATE OR DELETE OR TRUNCATE ON core.consumer_save_rule_facts FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal();
DO $$ DECLARE t text;BEGIN FOREACH t IN ARRAY ARRAY['consumer_splits','consumer_favorites','consumer_save_rules','consumer_save_rule_facts'] LOOP
 EXECUTE format('REVOKE ALL ON core.%I FROM bank_core',t);EXECUTE format('GRANT SELECT ON core.%I TO bank_core',t);END LOOP;END $$;

-- ===== Review (issuer approval) for saving rules =====
CREATE OR REPLACE FUNCTION core.consumer_review(p_actor jsonb,p_person uuid,p_action text,p_data jsonb) RETURNS jsonb LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,core AS $$
DECLARE keys text[]:=ARRAY['cardId','vaultId','matchKind','matchValue','amountMinor','weeklyCapMinor'];s core.consumer_schedules;basis jsonb;latest date;BEGIN
 IF p_action IN('SCHEDULE_CREATE','SCHEDULE_REPLACE') THEN
  -- Unchanged schema-60 behaviour.
  PERFORM core.consumer_actor(p_actor,p_person);PERFORM pg_advisory_xact_lock(hashtextextended(p_person::text,307));
  PERFORM core.consumer_schedule_terms(p_actor,p_person,p_data,p_action='SCHEDULE_REPLACE');
  IF p_action='SCHEDULE_CREATE' THEN RETURN core.consumer_authorize(p_actor,p_person,p_action,p_data);END IF;
  SELECT * INTO STRICT s FROM core.consumer_schedules WHERE id=(p_data->>'id')::uuid AND person_id=p_person FOR SHARE;
  IF s.state NOT IN('ACTIVE','PAUSED') THEN RAISE EXCEPTION 'Only a current active or paused instruction can be edited';END IF;
  SELECT max(run_date) INTO latest FROM core.consumer_schedule_family_runs WHERE family_id=s.family_id;
  IF (p_data->>'nextRun')::date<=latest THEN RAISE EXCEPTION 'Next execution date must follow the last completed family run';END IF;
  basis:=core.consumer_schedule_basis(s);
  RETURN core.consumer_authorize(p_actor,p_person,p_action,p_data||jsonb_build_object('replaces',basis,'basisDigest',encode(sha256(convert_to(core.periodic_policy_canonical(basis),'UTF8')),'hex')));
 END IF;
 IF p_action<>'SAVE_RULE_SET' THEN RETURN core.consumer_review_v59(p_actor,p_person,p_action,p_data);END IF;
 PERFORM core.consumer_actor(p_actor,p_person);
 IF jsonb_typeof(p_data)<>'object' OR EXISTS(SELECT 1 FROM jsonb_object_keys(p_data) k WHERE k<>ALL(keys)) OR EXISTS(SELECT 1 FROM unnest(keys) k WHERE NOT p_data?k) THEN
  RAISE EXCEPTION 'Supported action fields required';END IF;
 IF p_data->>'matchKind' NOT IN('MERCHANT','CATEGORY') OR length(btrim(coalesce(p_data->>'matchValue',''))) NOT BETWEEN 1 AND 64 THEN RAISE EXCEPTION 'Merchant or category match required';END IF;
 IF core.consumer_minor(p_data->>'weeklyCapMinor')<core.consumer_minor(p_data->>'amountMinor') THEN RAISE EXCEPTION 'Weekly cap must cover one saving';END IF;
 PERFORM 1 FROM core.consumer_cards WHERE id=(p_data->>'cardId')::uuid AND person_id=p_person;IF NOT FOUND THEN RAISE EXCEPTION 'Own card required';END IF;
 PERFORM 1 FROM core.consumer_vaults WHERE id=(p_data->>'vaultId')::uuid AND person_id=p_person;IF NOT FOUND THEN RAISE EXCEPTION 'Own vault required';END IF;
 RETURN core.consumer_authorize(p_actor,p_person,p_action,p_data);
END $$;
GRANT EXECUTE ON FUNCTION core.consumer_review(jsonb,uuid,text,jsonb) TO bank_core;

-- ===== Rule application after a posted card purchase (deterministic operation per purchase and rule) =====
CREATE FUNCTION core.consumer_apply_save_rules(p_person uuid,p_purchase uuid,p_account uuid,p_card uuid,p_merchant text,p_category text) RETURNS jsonb
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,core AS $$
DECLARE r core.consumer_save_rules;remaining bigint;amount bigint;batch uuid;applied jsonb:='[]'::jsonb;BEGIN
 FOR r IN SELECT * FROM core.consumer_save_rules WHERE person_id=p_person AND card_id=p_card AND enabled
   AND ((match_kind='MERCHANT' AND lower(match_value)=lower(btrim(coalesce(p_merchant,'')))) OR (match_kind='CATEGORY' AND lower(match_value)=lower(btrim(coalesce(p_category,'')))))
   ORDER BY created_at,id LOOP
  CONTINUE WHEN EXISTS(SELECT 1 FROM core.consumer_save_rule_facts WHERE purchase_id=p_purchase AND rule_id=r.id);
  SELECT greatest(0,r.weekly_cap_minor-coalesce(sum(f.amount_minor),0))::bigint INTO remaining FROM core.consumer_save_rule_facts f
   WHERE f.rule_id=r.id AND f.created_at>=date_trunc('week',clock_timestamp());
  amount:=least(r.amount_minor,remaining);
  CONTINUE WHEN amount<=0;
  BEGIN
   batch:=core.consumer_post(md5('save-rule:'||p_purchase||':'||r.id)::uuid,'MERCHANT_SAVE',p_account,r.vault_id,amount);
   INSERT INTO core.consumer_save_rule_facts VALUES(p_purchase,r.id,p_person,amount,batch,clock_timestamp());
   applied:=applied||jsonb_build_object('ruleId',r.id,'batchId',batch,'amountMinor',amount::text);
  EXCEPTION WHEN OTHERS THEN
   PERFORM core.consumer_notify(p_person,'Savings','Saving rule needs attention','The purchase posted; the separate rule saving did not.','/save');
  END;
 END LOOP;
 RETURN applied;
END $$;

-- ===== Commands =====
CREATE OR REPLACE FUNCTION core.consumer_command(p_actor jsonb,p_person uuid,p_key uuid,p_action text,p_data jsonb) RETURNS jsonb
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,core AS $$
DECLARE old core.consumer_commands;target uuid:=gen_random_uuid();result jsonb;keys text[];d jsonb;account uuid;total bigint;refs text[];people uuid[];n integer;share bigint;rest bigint;
 self_share bigint;i integer;other uuid;req core.consumer_requests;pe core.payment_executions;card core.consumer_cards;BEGIN
 IF p_action='CARD_PURCHASE' THEN
  -- The purchase itself is unchanged; approved saving rules apply afterwards, once per purchase and rule.
  result:=core.consumer_command_v59(p_actor,p_person,p_key,p_action,p_data);
  IF result->>'state'='POSTED' THEN
   SELECT details INTO d FROM core.consumer_authorizations WHERE id=(p_data->>'authorizationId')::uuid AND person_id=p_person;
   SELECT * INTO card FROM core.consumer_cards WHERE id=(d->>'id')::uuid AND person_id=p_person;
   IF FOUND THEN result:=result||jsonb_build_object('ruleSavings',core.consumer_apply_save_rules(p_person,(result->>'id')::uuid,card.account_id,card.id,d->>'merchant',d->>'category'));END IF;
  END IF;
  RETURN result;
 END IF;
 IF p_action NOT IN('SPLIT_EXPENSE','REQUEST_MARK_PAID','FAVORITE_SET','FAVORITE_REMOVE','SAVE_RULE_SET','SAVE_RULE_DISABLE') THEN
  IF p_action='SCHEDULE_REPLACE' THEN RETURN core.consumer_schedule_replace(p_actor,p_person,p_key,p_data);END IF;
  RETURN core.consumer_command_v59(p_actor,p_person,p_key,p_action,p_data);
 END IF;
 PERFORM core.consumer_actor(p_actor,p_person);
 IF p_key IS NULL OR p_key='00000000-0000-0000-0000-000000000000' THEN RAISE EXCEPTION 'Request key required';END IF;
 PERFORM pg_advisory_xact_lock(hashtextextended(p_person::text,307));
 SELECT * INTO old FROM core.consumer_commands WHERE issuer=p_actor->>'issuer' AND subject=p_actor->>'subject' AND client_id=p_actor->>'clientId' AND request_key=p_key;
 IF FOUND THEN IF old.person_id<>p_person OR old.action<>p_action OR old.input<>p_data THEN RAISE EXCEPTION 'Command scope conflict';END IF;RETURN old.result;END IF;
 keys:=CASE p_action WHEN 'SPLIT_EXPENSE' THEN ARRAY['accountId','batchId','references','includeSelf','memo'] WHEN 'REQUEST_MARK_PAID' THEN ARRAY['id','paymentId']
  WHEN 'FAVORITE_SET' THEN ARRAY['reference','label'] WHEN 'FAVORITE_REMOVE' THEN ARRAY['reference'] WHEN 'SAVE_RULE_SET' THEN ARRAY['authorizationId'] ELSE ARRAY['id'] END;
 IF jsonb_typeof(p_data) IS DISTINCT FROM 'object' OR EXISTS(SELECT 1 FROM jsonb_object_keys(p_data) k WHERE k<>ALL(keys))
  OR EXISTS(SELECT 1 FROM unnest(keys) k WHERE NOT p_data?k OR p_data->k='null'::jsonb) THEN RAISE EXCEPTION 'Complete supported command fields required';END IF;
 IF p_action='SPLIT_EXPENSE' THEN
  account:=(p_data->>'accountId')::uuid;PERFORM core.consumer_account(account,p_person);
  IF length(p_data->>'memo')>120 OR jsonb_typeof(p_data->'references')<>'array' OR jsonb_typeof(p_data->'includeSelf')<>'boolean' THEN RAISE EXCEPTION 'Valid split fields required';END IF;
  SELECT -e.delta INTO total FROM core.journal_entries e WHERE e.batch_id=(p_data->>'batchId')::uuid AND e.account_id=account AND e.delta<0;
  IF total IS NULL THEN RAISE EXCEPTION 'An outgoing transaction on your account is required';END IF;
  SELECT array_agg(DISTINCT btrim(t.x)) INTO refs FROM jsonb_array_elements_text(p_data->'references') t(x) WHERE btrim(t.x)<>'';
  IF refs IS NULL OR cardinality(refs)<>jsonb_array_length(p_data->'references') OR cardinality(refs) NOT BETWEEN 1 AND 9 THEN RAISE EXCEPTION 'Between one and nine distinct receiving references required';END IF;
  people:='{}';
  FOR i IN 1..cardinality(refs) LOOP
   SELECT r.person_id INTO other FROM core.receiving_references rr JOIN core.account_relationships r ON r.account_id=rr.account_id AND r.kind='OWNER' AND r.revoked_at IS NULL
    WHERE rr.reference=refs[i] LIMIT 1;
   IF other IS NULL OR other=p_person OR other=ANY(people) THEN RAISE EXCEPTION 'Each reference must belong to a different other customer';END IF;
   people:=people||other;
  END LOOP;
  n:=cardinality(refs)+CASE WHEN (p_data->>'includeSelf')::boolean THEN 1 ELSE 0 END;
  share:=total/n;rest:=total-share*n;
  IF share<=0 THEN RAISE EXCEPTION 'The amount is too small to split';END IF;
  -- With the requester included, the requester keeps the remainder; otherwise the first participants pay one more minor unit.
  self_share:=CASE WHEN (p_data->>'includeSelf')::boolean THEN share+rest ELSE 0 END;
  INSERT INTO core.consumer_splits VALUES(target,p_person,account,(p_data->>'batchId')::uuid,total,(p_data->>'includeSelf')::boolean,cardinality(refs),self_share,p_data->>'memo',clock_timestamp());
  FOR i IN 1..cardinality(refs) LOOP
   INSERT INTO core.consumer_requests(id,person_id,recipient_person,account_id,amount_minor,memo,state,payment_id,created_at,split_id)
    VALUES(gen_random_uuid(),p_person,people[i],account,share+CASE WHEN NOT (p_data->>'includeSelf')::boolean AND i<=rest THEN 1 ELSE 0 END,p_data->>'memo','REQUESTED',NULL,clock_timestamp(),target);
   PERFORM core.consumer_notify(people[i],'Transactions','Split request received',p_data->>'memo','/payments');
  END LOOP;
  result:=jsonb_build_object('id',target,'totalMinor',total::text,'selfShareMinor',self_share::text,'requests',cardinality(refs));
 ELSIF p_action='REQUEST_MARK_PAID' THEN
  SELECT * INTO req FROM core.consumer_requests WHERE id=(p_data->>'id')::uuid AND recipient_person=p_person FOR UPDATE;
  IF NOT FOUND OR req.state<>'REQUESTED' THEN RAISE EXCEPTION 'Open incoming request required';END IF;
  SELECT * INTO pe FROM core.payment_executions WHERE operation_id=(p_data->>'paymentId')::uuid;
  IF NOT FOUND OR pe.payee_account<>req.account_id OR pe.principal<>req.amount_minor
   OR NOT EXISTS(SELECT 1 FROM core.account_relationships WHERE account_id=pe.payer_account AND person_id=p_person AND kind='OWNER' AND revoked_at IS NULL) THEN
   RAISE EXCEPTION 'A settled payment of the exact amount from your account to the requester is required';END IF;
  IF EXISTS(SELECT 1 FROM core.consumer_requests WHERE payment_id=pe.operation_id) THEN RAISE EXCEPTION 'That payment already settles another request';END IF;
  UPDATE core.consumer_requests SET state='PAID',payment_id=pe.operation_id WHERE id=req.id;
  PERFORM core.consumer_notify(req.person_id,'Transactions','Request paid',req.memo,'/payments');
  result:=jsonb_build_object('id',req.id,'state','PAID','paymentId',pe.operation_id,'batchId',pe.batch_id);
 ELSIF p_action='FAVORITE_SET' THEN
  IF length(btrim(p_data->>'label')) NOT BETWEEN 1 AND 40 THEN RAISE EXCEPTION 'Favourite label of 1 to 40 characters required';END IF;
  IF NOT EXISTS(SELECT 1 FROM core.receiving_references WHERE reference=p_data->>'reference') THEN RAISE EXCEPTION 'Known receiving reference required';END IF;
  IF (SELECT count(*) FROM core.consumer_favorites WHERE person_id=p_person AND reference<>p_data->>'reference')>=20 THEN RAISE EXCEPTION 'At most 20 favourites';END IF;
  INSERT INTO core.consumer_favorites VALUES(p_person,p_data->>'reference',btrim(p_data->>'label'),clock_timestamp())
   ON CONFLICT(person_id,reference) DO UPDATE SET label=EXCLUDED.label;
  result:=jsonb_build_object('reference',p_data->>'reference','saved',true);
 ELSIF p_action='FAVORITE_REMOVE' THEN
  DELETE FROM core.consumer_favorites WHERE person_id=p_person AND reference=p_data->>'reference';
  result:=jsonb_build_object('reference',p_data->>'reference','removed',FOUND);
 ELSIF p_action='SAVE_RULE_SET' THEN
  d:=core.consumer_approval(p_actor,p_person,(p_data->>'authorizationId')::uuid,p_action);
  PERFORM 1 FROM core.consumer_cards WHERE id=(d->>'cardId')::uuid AND person_id=p_person;IF NOT FOUND THEN RAISE EXCEPTION 'Own card required';END IF;
  PERFORM 1 FROM core.consumer_vaults WHERE id=(d->>'vaultId')::uuid AND person_id=p_person;IF NOT FOUND THEN RAISE EXCEPTION 'Own vault required';END IF;
  IF (SELECT count(*) FROM core.consumer_save_rules WHERE person_id=p_person AND enabled)>=10 THEN RAISE EXCEPTION 'At most 10 active saving rules';END IF;
  INSERT INTO core.consumer_save_rules VALUES(target,p_person,(p_data->>'authorizationId')::uuid,(d->>'cardId')::uuid,(d->>'vaultId')::uuid,d->>'matchKind',btrim(d->>'matchValue'),
   core.consumer_minor(d->>'amountMinor'),core.consumer_minor(d->>'weeklyCapMinor'),true,clock_timestamp());
  result:=jsonb_build_object('id',target,'state','ACTIVE');
 ELSE
  UPDATE core.consumer_save_rules SET enabled=false WHERE id=(p_data->>'id')::uuid AND person_id=p_person AND enabled;
  IF NOT FOUND THEN RAISE EXCEPTION 'Active own saving rule required';END IF;
  result:=jsonb_build_object('id',p_data->>'id','state','DISABLED');
 END IF;
 INSERT INTO core.consumer_commands VALUES(target,p_actor->>'issuer',p_actor->>'subject',p_actor->>'clientId',p_key,p_person,p_action,p_data,result,clock_timestamp());
 RETURN result;
END $$;
GRANT EXECUTE ON FUNCTION core.consumer_command(jsonb,uuid,uuid,text,jsonb) TO bank_core;

-- ===== Safe-to-spend (labelled local rule) =====
CREATE FUNCTION core.consumer_safe_to_spend(p_person uuid,p_at timestamptz) RETURNS jsonb LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,core AS $$
DECLARE month_end date:=(date_trunc('month',p_at AT TIME ZONE 'UTC')+interval '1 month - 1 day')::date;s record;run date;automations numeric:=0;per_account jsonb:='{}'::jsonb;
 available numeric;bills numeric;BEGIN
 FOR s IN SELECT c.* FROM core.consumer_schedules c WHERE c.person_id=p_person AND c.state='ACTIVE' LOOP
  run:=s.next_run;
  WHILE run<=month_end LOOP
   automations:=automations+s.amount_minor;
   per_account:=jsonb_set(per_account,ARRAY[s.source_account_id::text],to_jsonb(coalesce((per_account->>s.source_account_id::text)::numeric,0)+s.amount_minor));
   run:=core.consumer_next_run(run,s.frequency,s.anchor_day);
  END LOOP;
 END LOOP;
 SELECT coalesce(sum(a.booked-a.held),0) INTO available FROM core.ledger_accounts a JOIN core.account_relationships r ON r.account_id=a.id AND r.kind='OWNER' AND r.revoked_at IS NULL
  WHERE r.person_id=p_person AND a.kind='CUSTOMER' AND a.state='ACTIVE' AND NOT EXISTS(SELECT 1 FROM core.consumer_vaults v WHERE v.id=a.id);
 SELECT coalesce(sum(amount_minor),0) INTO bills FROM core.consumer_bills WHERE person_id=p_person AND state='ACTIVE' AND next_date<=month_end;
 RETURN jsonb_build_object('safeToSpendMinor',greatest(available-automations-bills,0)::text,'availableMinor',available::text,'automationsDueMinor',automations::text,
  'billsDueMinor',bills::text,'through',month_end,'rule','AVAILABLE_MINUS_APPROVED_AUTOMATIONS_AND_BILL_REMINDERS_DUE_BY_MONTH_END','basis','LOCAL_RULE_NOT_ADVICE');
END $$;

-- ===== Snapshot additions =====
CREATE OR REPLACE FUNCTION core.consumer_snapshot(p_actor jsonb,p_person uuid) RETURNS jsonb LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,core AS $$
DECLARE d jsonb;BEGIN
 -- Schema-61 composition: the original snapshot validates actor and ownership first.
 d:=core.consumer_snapshot_v60(p_actor,p_person)||jsonb_build_object('progress',core.consumer_progress_at(p_person,clock_timestamp()));
 RETURN d||jsonb_build_object(
  'favorites',coalesce((SELECT jsonb_agg(jsonb_build_object('reference',reference,'label',label) ORDER BY label,reference) FROM core.consumer_favorites WHERE person_id=p_person),'[]'::jsonb),
  'splits',coalesce((SELECT jsonb_agg(jsonb_build_object('id',s.id,'batchId',s.batch_id,'totalMinor',s.total_minor::text,'selfShareMinor',s.self_share_minor::text,
    'participants',s.participants,'memo',s.memo,'createdAt',s.created_at,
    'paid',(SELECT count(*) FROM core.consumer_requests q WHERE q.split_id=s.id AND q.state='PAID'),
    'requests',(SELECT jsonb_agg(jsonb_build_object('id',q.id,'amountMinor',q.amount_minor::text,'state',q.state) ORDER BY q.amount_minor DESC,q.id) FROM core.consumer_requests q WHERE q.split_id=s.id))
    ORDER BY s.created_at DESC) FROM core.consumer_splits s WHERE s.person_id=p_person),'[]'::jsonb),
  'requestLinks',coalesce((SELECT jsonb_object_agg(q.id,jsonb_build_object('splitId',q.split_id,'paymentId',q.payment_id))
    FROM core.consumer_requests q WHERE (q.person_id=p_person OR q.recipient_person=p_person) AND (q.split_id IS NOT NULL OR q.payment_id IS NOT NULL)),'{}'::jsonb),
  'saveRules',coalesce((SELECT jsonb_agg(jsonb_build_object('id',r.id,'cardId',r.card_id,'vaultId',r.vault_id,'matchKind',r.match_kind,'matchValue',r.match_value,
    'amountMinor',r.amount_minor::text,'weeklyCapMinor',r.weekly_cap_minor::text,'enabled',r.enabled,'createdAt',r.created_at,
    'savedThisWeekMinor',(SELECT coalesce(sum(f.amount_minor),0)::text FROM core.consumer_save_rule_facts f WHERE f.rule_id=r.id AND f.created_at>=date_trunc('week',clock_timestamp())),
    'savedLifetimeMinor',(SELECT coalesce(sum(f.amount_minor),0)::text FROM core.consumer_save_rule_facts f WHERE f.rule_id=r.id)) ORDER BY r.created_at DESC)
    FROM core.consumer_save_rules r WHERE r.person_id=p_person),'[]'::jsonb),
  'safeToSpend',core.consumer_safe_to_spend(p_person,clock_timestamp()));
END $$;
GRANT EXECUTE ON FUNCTION core.consumer_snapshot(jsonb,uuid) TO bank_core;

-- Rule savings are transfers to the customer's own vault, not spending: exclude them from spending analytics as round-ups are.
CREATE OR REPLACE FUNCTION core.consumer_analytics(p_person uuid) RETURNS jsonb LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,core AS $$
DECLARE start_day date:=(date_trunc('month',clock_timestamp() AT TIME ZONE 'UTC'))::date;BEGIN
 RETURN (WITH movements AS(
 SELECT j.id,j.business_date,j.posting_kind,sum(e.delta::numeric) delta,max(coalesce(m.category,'Other')) category,max(coalesce(m.note,'')) merchant
 FROM core.journal_entries e JOIN core.journal_batches j ON j.id=e.batch_id JOIN core.account_relationships r ON r.account_id=e.account_id AND r.person_id=p_person AND r.kind='OWNER' AND r.revoked_at IS NULL LEFT JOIN core.consumer_metadata m ON m.batch_id=e.batch_id AND m.account_id=e.account_id
 WHERE j.business_date>=start_day-interval '3 months' AND j.business_date<start_day+interval '1 month' AND j.posting_kind NOT IN('VAULT_DEPOSIT','VAULT_WITHDRAWAL','AUTO_SAVE','SPARE_CHANGE','MERCHANT_SAVE','SHARED_WITHDRAWAL','BROKER_DEPOSIT','BROKER_WITHDRAWAL') GROUP BY j.id
 ),cats AS(SELECT category,coalesce(sum(-delta) FILTER(WHERE delta<0 AND business_date>=start_day),0) spent,coalesce(sum(-delta) FILTER(WHERE delta<0 AND business_date>=start_day-interval '1 month' AND business_date<start_day),0) previous,coalesce(sum(-delta) FILTER(WHERE delta<0 AND business_date<start_day),0)/3 average FROM movements GROUP BY category)
 SELECT jsonb_build_object('from',start_day,'to',(start_day+interval '1 month')::date,'incomeMinor',coalesce((SELECT sum(delta) FROM movements WHERE delta>0 AND business_date>=start_day),0)::text,'spentMinor',coalesce((SELECT sum(-delta) FROM movements WHERE delta<0 AND business_date>=start_day),0)::text,
 'categories',coalesce((SELECT jsonb_agg(jsonb_build_object('category',c.category,'spentMinor',trunc(c.spent)::text,'averageMinor',trunc(c.average)::text,'budgetMinor',(b.limit_minor+CASE WHEN b.rollover THEN greatest(0,b.limit_minor-c.previous) ELSE 0 END)::text)) FROM cats c LEFT JOIN core.consumer_budgets b ON b.person_id=p_person AND b.category=c.category),'[]'::jsonb),
 'merchants',coalesce((SELECT jsonb_agg(x) FROM(SELECT jsonb_build_object('name',coalesce(nullif(merchant,''),'Unspecified merchant'),'spentMinor',sum(-delta)::text,'visits',count(*)) x FROM movements WHERE delta<0 AND business_date>=start_day GROUP BY merchant ORDER BY sum(-delta) DESC LIMIT 10) m),'[]'::jsonb)));
END $$;
