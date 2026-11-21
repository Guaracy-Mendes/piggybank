-- owner: core (consumer workspace). Official UI increment (schema 80), local synthetic template.
-- UI-09/UI-14 savings interest with user-approved synthetic product rules (2026-11-21). Version 1 of the program:
--  * base 3.00% a year on the customer's own vault balances, daily ACT/365 on each day's closing booked balance;
--    the 50,000 TST balance cap applies across the customer's own vaults, oldest vault first; interest is summed per
--    vault and month, rounded half up once, and credited into each vault after the month closes;
--  * stacking rate boosts, total at most 4.50%: SAVING_STREAK +0.50% for a month after saving into a vault in every
--    week (Monday to Sunday) whose Thursday fell in the previous month; NEW_CUSTOMER +1.00% for the first 3 full
--    calendar months after the first account opened;
--  * a one-off challenge: save TST 500 into one own vault within 30 days for a TST 10 bonus;
--  * referrals: TST 25 to each side once the referee holds at least TST 100 in their accounts for 30 days; at most 5
--    paid referrals per referrer and calendar year; the code is entered within 30 days of the referee's first account.
-- Customers opt in by accepting the exact terms digest. Every credit is a native balanced journal from the approved
-- interest expense control account (...0005), keyed by a deterministic operation so a retry cannot pay twice.
-- Not reviewed under any jurisdiction's deposit rules; Brazilian requirements are pending.
CREATE TABLE core.savings_programs(version integer PRIMARY KEY CHECK(version>0),document jsonb NOT NULL,
 digest text NOT NULL UNIQUE CHECK(digest ~ '^[0-9a-f]{64}$'),created_at timestamptz NOT NULL DEFAULT clock_timestamp());
CREATE TRIGGER immutable_savings_programs BEFORE UPDATE OR DELETE OR TRUNCATE ON core.savings_programs FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal();
CREATE TABLE core.savings_enrollments(person_id uuid PRIMARY KEY,program_version integer NOT NULL REFERENCES core.savings_programs(version),
 digest text NOT NULL,actor_context jsonb NOT NULL,joined_on date NOT NULL,joined_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 referral_code text NOT NULL UNIQUE CHECK(referral_code ~ '^PB[0-9A-F]{8}$'),left_on date,left_at timestamptz,
 CHECK(left_on IS NULL OR left_on>=joined_on),CHECK((left_on IS NULL)=(left_at IS NULL)));
CREATE TABLE core.savings_interest_months(id uuid PRIMARY KEY,person_id uuid NOT NULL,month date NOT NULL CHECK(month=date_trunc('month',month)::date),
 program_version integer NOT NULL REFERENCES core.savings_programs(version),document jsonb NOT NULL,digest text NOT NULL CHECK(digest ~ '^[0-9a-f]{64}$'),
 total_minor bigint NOT NULL CHECK(total_minor>=0),finalized_at timestamptz NOT NULL DEFAULT clock_timestamp(),UNIQUE(person_id,month));
CREATE TRIGGER immutable_savings_interest_months BEFORE UPDATE OR DELETE OR TRUNCATE ON core.savings_interest_months FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal();
CREATE TABLE core.savings_interest_postings(month_id uuid NOT NULL REFERENCES core.savings_interest_months(id),vault_id uuid NOT NULL REFERENCES core.consumer_vaults(id),
 batch_id uuid NOT NULL UNIQUE REFERENCES core.journal_batches(id),amount_minor bigint NOT NULL CHECK(amount_minor>0),PRIMARY KEY(month_id,vault_id));
CREATE TRIGGER immutable_savings_interest_postings BEFORE UPDATE OR DELETE OR TRUNCATE ON core.savings_interest_postings FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal();
CREATE TABLE core.savings_challenges(person_id uuid PRIMARY KEY,program_version integer NOT NULL REFERENCES core.savings_programs(version),
 vault_id uuid NOT NULL REFERENCES core.consumer_vaults(id),started_on date NOT NULL,ends_on date NOT NULL,target_minor bigint NOT NULL CHECK(target_minor>0),
 bonus_minor bigint NOT NULL CHECK(bonus_minor>0),state text NOT NULL DEFAULT 'ACTIVE' CHECK(state IN('ACTIVE','COMPLETED','EXPIRED')),completed_on date,
 batch_id uuid UNIQUE REFERENCES core.journal_batches(id),checked_at timestamptz NOT NULL DEFAULT '-infinity',created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 CHECK(ends_on=started_on+29),CHECK((state='COMPLETED')=(batch_id IS NOT NULL AND completed_on IS NOT NULL)));
CREATE TABLE core.savings_referrals(id uuid PRIMARY KEY,referrer_person uuid NOT NULL,referee_person uuid NOT NULL UNIQUE,code text NOT NULL,
 claimed_on date NOT NULL,state text NOT NULL DEFAULT 'PENDING' CHECK(state IN('PENDING','PAID','EXPIRED')),qualified_on date,
 referee_batch uuid UNIQUE REFERENCES core.journal_batches(id),referrer_batch uuid UNIQUE REFERENCES core.journal_batches(id),
 referrer_outcome text CHECK(referrer_outcome IN('PAID','ANNUAL_LIMIT','NO_ACTIVE_ACCOUNT')),checked_at timestamptz NOT NULL DEFAULT '-infinity',
 created_at timestamptz NOT NULL DEFAULT clock_timestamp(),CHECK(referrer_person<>referee_person),
 CHECK((state='PAID')=(qualified_on IS NOT NULL AND referee_batch IS NOT NULL AND referrer_outcome IS NOT NULL)),
 CHECK((referrer_outcome='PAID')=(referrer_batch IS NOT NULL)));
CREATE INDEX savings_referrals_referrer ON core.savings_referrals(referrer_person,qualified_on);
CREATE TABLE core.savings_issues(person_id uuid NOT NULL,subject text NOT NULL CHECK(subject IN('interest','challenge','referral')),reason text NOT NULL,
 retry_after timestamptz NOT NULL,updated_at timestamptz NOT NULL DEFAULT clock_timestamp(),PRIMARY KEY(person_id,subject));
DO $$ DECLARE t text;BEGIN FOREACH t IN ARRAY ARRAY['savings_programs','savings_enrollments','savings_interest_months','savings_interest_postings','savings_challenges','savings_referrals','savings_issues'] LOOP
 EXECUTE format('REVOKE ALL ON core.%I FROM bank_core',t);EXECUTE format('GRANT SELECT ON core.%I TO bank_core',t);END LOOP;END $$;

INSERT INTO core.savings_programs(version,document,digest) SELECT 1,d,encode(sha256(convert_to(core.periodic_policy_canonical(d),'UTF8')),'hex') FROM (SELECT jsonb_build_object(
 'code','PIGGYBANK_SAVINGS_INTEREST','version',1,'title','PiggyBank Savings Interest','label','SYNTHETIC_EVALUATION','currency','TST',
 'appliesTo','OWN_VAULTS','baseRateBps',300,'maxRateBps',450,'balanceCapMinor','5000000','dayCount','ACT/365','crediting','MONTHLY_AFTER_CLOSE',
 'boosts',jsonb_build_object(
  'savingStreak',jsonb_build_object('rateBps',50,'rule','Save into a vault in every week (Monday to Sunday) whose Thursday falls in a month to earn the boost for the following month.'),
  'newCustomer',jsonb_build_object('rateBps',100,'months',3,'rule','The first 3 full calendar months after your first account opened.')),
 'challenge',jsonb_build_object('code','SAVE_500_IN_30_DAYS','targetMinor','50000','days',30,'bonusMinor','1000','oncePerPerson',true),
 'referral',jsonb_build_object('bonusMinor','2500','fundMinor','10000','holdDays',30,'claimWithinDays',30,'maxPaidPerYear',5,'expiresAfterDays',180),
 'terms',jsonb_build_array(
  'Synthetic evaluation only. No real money, deposit or interest is involved.',
  'Your own vaults earn a base rate of 3.00% a year (about 3.04% APY), calculated daily on each day''s closing balance using actual days / 365.',
  'The rate applies to at most TST 50,000.00 across your own vaults, oldest vault first. Balances above the cap earn nothing.',
  'Interest is summed per vault for each calendar month (UTC), rounded half up once, and credited into the vault after the month closes.',
  'Boosts stack, up to 4.50% in total: +0.50% for a month after you saved into a vault in every week of the previous month, and +1.00% for the first 3 full calendar months after your first account opened.',
  'One-off challenge: save TST 500.00 into one of your vaults within 30 days and receive a TST 10.00 bonus in that vault.',
  'Referrals: when someone uses your code within 30 days of opening their first account and keeps at least TST 100.00 in their accounts for 30 days, you each receive TST 25.00. At most 5 paid referrals per calendar year.',
  'Interest accrues from the day you join until the day before you leave. Leaving is final for this program version.',
  'These rules have not been reviewed under any jurisdiction''s deposit or consumer rules.'),
 'decision','User-approved synthetic product rules recorded on 2026-11-21. Not reviewed under any jurisdiction; Brazilian requirements are pending.') d) q;

CREATE FUNCTION core.savings_apy(p_bps integer) RETURNS numeric LANGUAGE sql IMMUTABLE SET search_path=pg_catalog AS $$
 SELECT round((power(1+p_bps::numeric/120000,12)-1)*100,2) $$;
CREATE FUNCTION core.savings_operation(p_kind text,p_id uuid,p_day date) RETURNS uuid LANGUAGE sql IMMUTABLE SET search_path=pg_catalog AS $$
 SELECT md5('piggybank-savings:'||p_kind||':'||p_id::text||':'||p_day::text)::uuid $$;
CREATE FUNCTION core.savings_today() RETURNS date LANGUAGE sql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,core AS $$
 SELECT (core.periodic_clock() AT TIME ZONE 'UTC')::date $$;
-- The first business date the person's earliest owned account was opened.
CREATE FUNCTION core.savings_opened_on(p_person uuid) RETURNS date LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog,core AS $$
 SELECT min((r.created_at AT TIME ZONE 'UTC')::date) FROM core.account_registry r JOIN core.account_relationships x ON x.account_id=r.id
 WHERE x.person_id=p_person AND x.kind='OWNER' $$;
-- The oldest active account the person currently owns receives referral bonuses.
CREATE FUNCTION core.savings_payout_account(p_person uuid) RETURNS uuid LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog,core AS $$
 SELECT r.id FROM core.account_registry r JOIN core.account_relationships x ON x.account_id=r.id AND x.person_id=p_person AND x.kind='OWNER' AND x.revoked_at IS NULL
 JOIN core.ledger_accounts l ON l.id=r.id AND l.kind='CUSTOMER' AND l.state='ACTIVE' ORDER BY r.created_at,r.id LIMIT 1 $$;
-- Weeks (Monday to Sunday) whose Thursday falls in the month, and whether the person saved into a vault in each.
CREATE FUNCTION core.savings_weeks(p_person uuid,p_month date,p_today date) RETURNS jsonb LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog,core AS $$
 WITH facts AS(SELECT DISTINCT f.day FROM core.consumer_contribution_facts(p_person) f WHERE f.day BETWEEN p_month-3 AND (p_month+interval '1 month')::date+3),
 weeks AS(SELECT (g.t::date-3) AS monday,(g.t::date+3) AS sunday FROM generate_series(p_month::timestamp,(p_month+interval '1 month'-interval '1 day'),interval '1 day') g(t)
  WHERE extract(isodow FROM g.t)=4)
 SELECT jsonb_agg(jsonb_build_object('from',w.monday,'to',w.sunday,'saved',EXISTS(SELECT 1 FROM facts f WHERE f.day BETWEEN w.monday AND w.sunday),
  'state',CASE WHEN EXISTS(SELECT 1 FROM facts f WHERE f.day BETWEEN w.monday AND least(w.sunday,p_today)) THEN 'SAVED' WHEN w.sunday<p_today THEN 'MISSED' WHEN w.monday>p_today THEN 'UPCOMING' ELSE 'OPEN' END) ORDER BY w.monday)
 FROM weeks w $$;
-- The rate for one calendar month: base plus stacking boosts, capped.
CREATE FUNCTION core.savings_rate(p_person uuid,p_month date) RETURNS jsonb LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,core AS $$
DECLARE prog jsonb;opened date;first_full date;prev date:=(p_month-interval '1 month')::date;weeks jsonb;streak boolean;newc boolean;base integer;sb integer;nb integer;cap integer;raw integer;total integer;BEGIN
 SELECT document INTO STRICT prog FROM core.savings_programs WHERE version=1;
 base:=(prog->>'baseRateBps')::integer;cap:=(prog->>'maxRateBps')::integer;
 sb:=(prog->'boosts'->'savingStreak'->>'rateBps')::integer;nb:=(prog->'boosts'->'newCustomer'->>'rateBps')::integer;
 weeks:=core.savings_weeks(p_person,prev,(p_month-1));
 streak:=NOT EXISTS(SELECT 1 FROM jsonb_array_elements(weeks) w WHERE NOT (w->>'saved')::boolean);
 opened:=core.savings_opened_on(p_person);
 first_full:=CASE WHEN opened IS NULL THEN NULL WHEN opened=date_trunc('month',opened)::date THEN opened ELSE (date_trunc('month',opened)+interval '1 month')::date END;
 newc:=first_full IS NOT NULL AND p_month>=first_full AND p_month<(first_full+make_interval(months=>(prog->'boosts'->'newCustomer'->>'months')::integer))::date;
 raw:=base+CASE WHEN streak THEN sb ELSE 0 END+CASE WHEN newc THEN nb ELSE 0 END;total:=least(raw,cap);
 RETURN jsonb_build_object('month',p_month,'baseBps',base,'capBps',cap,'uncappedBps',raw,'totalBps',total,'apyPercent',core.savings_apy(total)::text,
  'components',jsonb_build_array(
   jsonb_build_object('code','BASE','bps',base,'applied',true),
   jsonb_build_object('code','SAVING_STREAK','bps',sb,'applied',streak,'basisMonth',prev,'weeks',weeks),
   jsonb_build_object('code','NEW_CUSTOMER','bps',nb,'applied',newc,'openedOn',opened,'firstMonth',first_full,
    'lastMonth',CASE WHEN first_full IS NULL THEN NULL ELSE (first_full+make_interval(months=>(prog->'boosts'->'newCustomer'->>'months')::integer-1))::date END)));
END $$;
-- Interest for the person's own vaults over one month, through p_until (inclusive), from the day they joined until the
-- day before they left. Each vault's closing booked balance per day is capped across vaults (oldest first).
CREATE FUNCTION core.savings_accrual(p_person uuid,p_month date,p_until date) RETURNS jsonb LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,core AS $$
DECLARE e core.savings_enrollments;rate jsonb;bps integer;cap bigint;from_day date;to_day date;vaults jsonb;total numeric;BEGIN
 SELECT * INTO e FROM core.savings_enrollments WHERE person_id=p_person;
 IF NOT FOUND THEN RETURN NULL;END IF;
 rate:=core.savings_rate(p_person,p_month);bps:=(rate->>'totalBps')::integer;
 SELECT (document->>'balanceCapMinor')::bigint INTO STRICT cap FROM core.savings_programs WHERE version=e.program_version;
 from_day:=greatest(p_month,e.joined_on);
 to_day:=least((p_month+interval '1 month'-interval '1 day')::date,p_until,coalesce(e.left_on-1,'infinity'::date));
 WITH vs AS(SELECT v.id,row_number() OVER(ORDER BY v.created_at,v.id) AS ord FROM core.consumer_vaults v WHERE v.person_id=p_person),
 deltas AS(SELECT j.account_id AS id,b.business_date AS day,sum(j.delta) AS delta FROM core.journal_entries j JOIN core.journal_batches b ON b.id=j.batch_id
  WHERE j.account_id IN(SELECT id FROM vs) AND b.business_date<=to_day GROUP BY 1,2),
 bal AS(SELECT d.day::date AS day,vs.id,vs.ord,greatest((SELECT coalesce(sum(x.delta),0) FROM deltas x WHERE x.id=vs.id AND x.day<=d.day::date),0) AS closing
  FROM generate_series(from_day,to_day,interval '1 day') d(day) CROSS JOIN vs),
 capped AS(SELECT id,ord,least(closing,greatest(cap-coalesce(sum(closing) OVER(PARTITION BY day ORDER BY ord ROWS BETWEEN UNBOUNDED PRECEDING AND 1 PRECEDING),0),0)) AS eligible FROM bal),
 per AS(SELECT id,ord,count(*) AS days,sum(eligible) AS minor_days FROM capped GROUP BY id,ord)
 SELECT coalesce(jsonb_agg(jsonb_build_object('vaultId',id,'days',days,'eligibleMinorDays',minor_days::text,'amountMinor',round(minor_days*bps/3650000)::text) ORDER BY ord),'[]'::jsonb),
  coalesce(sum(round(minor_days*bps/3650000)),0) INTO vaults,total FROM per;
 RETURN jsonb_build_object('month',p_month,'from',from_day,'to',to_day,'rate',rate,'balanceCapMinor',cap::text,'vaults',vaults,'totalMinor',total::text,
  'formula','Each day: closing booked vault balance, capped across own vaults oldest first. Per vault and month: sum x rate bps / 3,650,000, rounded half up once.');
END $$;

-- Customer commands: join (exact terms digest), leave (final), start the one-off challenge, claim a referral code.
CREATE FUNCTION core.savings_command(p_actor jsonb,p_person uuid,p_key uuid,p_action text,p_data jsonb) RETURNS jsonb
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,core AS $$
DECLARE old core.consumer_commands;target uuid:=gen_random_uuid();result jsonb;keys text[];prog core.savings_programs;e core.savings_enrollments;
 today date:=core.savings_today();code text;owner uuid;v core.consumer_vaults;opened date;BEGIN
 PERFORM core.consumer_actor(p_actor,p_person);
 IF p_key IS NULL OR p_key='00000000-0000-0000-0000-000000000000' THEN RAISE EXCEPTION 'Request key required';END IF;
 PERFORM pg_advisory_xact_lock(hashtextextended(p_person::text,307));
 SELECT * INTO old FROM core.consumer_commands WHERE issuer=p_actor->>'issuer' AND subject=p_actor->>'subject' AND client_id=p_actor->>'clientId' AND request_key=p_key;
 IF FOUND THEN IF old.person_id<>p_person OR old.action<>p_action OR old.input<>p_data THEN RAISE EXCEPTION 'Command scope conflict';END IF;RETURN old.result;END IF;
 keys:=CASE p_action WHEN 'SAVINGS_JOIN' THEN ARRAY['digest','accepted'] WHEN 'SAVINGS_LEAVE' THEN ARRAY['confirm'] WHEN 'SAVINGS_CHALLENGE' THEN ARRAY['vaultId'] ELSE ARRAY['code'] END;
 IF jsonb_typeof(p_data) IS DISTINCT FROM 'object' OR EXISTS(SELECT 1 FROM jsonb_object_keys(p_data) k WHERE k<>ALL(keys))
  OR EXISTS(SELECT 1 FROM unnest(keys) k WHERE NOT p_data?k OR p_data->k='null'::jsonb) THEN RAISE EXCEPTION 'Complete supported command fields required';END IF;
 SELECT * INTO STRICT prog FROM core.savings_programs WHERE version=1;
 SELECT * INTO e FROM core.savings_enrollments WHERE person_id=p_person;
 IF p_action='SAVINGS_JOIN' THEN
  IF e.person_id IS NOT NULL THEN RAISE EXCEPTION 'You already joined the savings interest program';END IF;
  IF p_data->'accepted'<>'true'::jsonb OR p_data->>'digest' IS DISTINCT FROM prog.digest THEN RAISE EXCEPTION 'Accept the current savings interest terms';END IF;
  IF core.savings_payout_account(p_person) IS NULL THEN RAISE EXCEPTION 'An active account you own is required';END IF;
  LOOP
   code:='PB'||upper(substr(encode(sha256(convert_to(gen_random_uuid()::text,'UTF8')),'hex'),1,8));
   EXIT WHEN NOT EXISTS(SELECT 1 FROM core.savings_enrollments WHERE referral_code=code);
  END LOOP;
  INSERT INTO core.savings_enrollments(person_id,program_version,digest,actor_context,joined_on,referral_code) VALUES(p_person,prog.version,prog.digest,p_actor,today,code);
  result:=jsonb_build_object('state','ENROLLED','joinedOn',today,'referralCode',code,'programVersion',prog.version);
 ELSIF e.person_id IS NULL AND p_action IN('SAVINGS_LEAVE','SAVINGS_CHALLENGE') OR e.left_on IS NOT NULL AND p_action IN('SAVINGS_LEAVE','SAVINGS_CHALLENGE') THEN
  RAISE EXCEPTION 'An active savings interest enrollment is required';
 ELSIF p_action='SAVINGS_LEAVE' THEN
  IF p_data->'confirm'<>'true'::jsonb THEN RAISE EXCEPTION 'Confirm that you want to leave the program';END IF;
  UPDATE core.savings_enrollments SET left_on=today,left_at=clock_timestamp() WHERE person_id=p_person;
  result:=jsonb_build_object('state','LEFT','leftOn',today);
 ELSIF p_action='SAVINGS_CHALLENGE' THEN
  IF EXISTS(SELECT 1 FROM core.savings_challenges WHERE person_id=p_person) THEN RAISE EXCEPTION 'The one-off challenge was already started';END IF;
  SELECT * INTO v FROM core.consumer_vaults WHERE id=(p_data->>'vaultId')::uuid AND person_id=p_person;
  IF NOT FOUND THEN RAISE EXCEPTION 'Choose one of your own vaults';END IF;
  INSERT INTO core.savings_challenges(person_id,program_version,vault_id,started_on,ends_on,target_minor,bonus_minor)
  VALUES(p_person,prog.version,v.id,today,today+29,(prog.document->'challenge'->>'targetMinor')::bigint,(prog.document->'challenge'->>'bonusMinor')::bigint);
  result:=jsonb_build_object('state','ACTIVE','vaultId',v.id,'startedOn',today,'endsOn',today+29);
 ELSIF p_action='REFERRAL_CLAIM' THEN
  code:=upper(btrim(p_data->>'code'));
  IF code !~ '^PB[0-9A-F]{8}$' THEN RAISE EXCEPTION 'Enter a referral code like PB1A2B3C4D';END IF;
  SELECT person_id INTO owner FROM core.savings_enrollments WHERE referral_code=code AND left_on IS NULL;
  IF NOT FOUND THEN RAISE EXCEPTION 'Unknown referral code';END IF;
  IF owner=p_person THEN RAISE EXCEPTION 'You cannot use your own referral code';END IF;
  IF EXISTS(SELECT 1 FROM core.savings_referrals WHERE referee_person=p_person) THEN RAISE EXCEPTION 'You already used a referral code';END IF;
  IF EXISTS(SELECT 1 FROM core.savings_referrals WHERE referrer_person=p_person AND referee_person=owner) THEN RAISE EXCEPTION 'Two people cannot refer each other';END IF;
  opened:=core.savings_opened_on(p_person);
  IF opened IS NOT NULL AND today>opened+(prog.document->'referral'->>'claimWithinDays')::integer THEN RAISE EXCEPTION 'Referral codes must be entered within 30 days of opening your first account';END IF;
  INSERT INTO core.savings_referrals(id,referrer_person,referee_person,code,claimed_on) VALUES(target,owner,p_person,code,today);
  result:=jsonb_build_object('id',target,'state','PENDING','claimedOn',today);
 ELSE RAISE EXCEPTION 'Unsupported savings command';
 END IF;
 INSERT INTO core.consumer_commands VALUES(target,p_actor->>'issuer',p_actor->>'subject',p_actor->>'clientId',p_key,p_person,p_action,p_data,result,clock_timestamp());
 RETURN result;
END $$;

-- Durable processing, called by the worker after the consumer tick: finalize and credit the next closed month for each
-- enrolled person, settle the one-off challenge and pay qualified referrals. Every credit uses a deterministic operation.
CREATE FUNCTION core.savings_tick() RETURNS integer LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,core AS $$
DECLARE today date:=core.savings_today();closed date;c record;m date;acc jsonb;doc jsonb;mid uuid;v jsonb;amt bigint;batch uuid;done integer:=0;
 expense uuid:='4f1a780b-3e31-48ef-a294-000000000005';prog jsonb;ch core.savings_challenges;progress numeric;r core.savings_referrals;opened date;ok boolean;
 referee_account uuid;referrer_account uuid;paid integer;outcome text;rb uuid;BEGIN
 closed:=(date_trunc('month',today)-interval '1 month')::date;
 SELECT document INTO STRICT prog FROM core.savings_programs WHERE version=1;
 FOR c IN SELECT e.person_id,e.joined_on,e.left_on,e.program_version,(SELECT max(x.month) FROM core.savings_interest_months x WHERE x.person_id=e.person_id) AS last
  FROM core.savings_enrollments e WHERE date_trunc('month',e.joined_on)::date<=closed
  AND NOT EXISTS(SELECT 1 FROM core.savings_issues i WHERE i.person_id=e.person_id AND i.subject='interest' AND i.retry_after>clock_timestamp())
  ORDER BY e.person_id LOOP
  m:=coalesce((c.last+interval '1 month')::date,date_trunc('month',c.joined_on)::date);
  IF m>closed OR (c.left_on IS NOT NULL AND m>date_trunc('month',c.left_on)::date) THEN CONTINUE;END IF;
  IF done>=25 THEN EXIT;END IF;
  IF NOT pg_try_advisory_xact_lock(hashtextextended(c.person_id::text,307)) THEN CONTINUE;END IF;
  BEGIN
   acc:=core.savings_accrual(c.person_id,m,today-1);mid:=gen_random_uuid();
   doc:=acc||jsonb_build_object('personId',c.person_id,'programVersion',c.program_version);
   INSERT INTO core.savings_interest_months VALUES(mid,c.person_id,m,c.program_version,doc,encode(sha256(convert_to(core.periodic_policy_canonical(doc),'UTF8')),'hex'),(acc->>'totalMinor')::bigint,clock_timestamp());
   FOR v IN SELECT value FROM jsonb_array_elements(acc->'vaults') LOOP
    amt:=(v->>'amountMinor')::bigint;
    IF amt>0 THEN
     batch:=core.consumer_post(core.savings_operation('interest',(v->>'vaultId')::uuid,m),'SAVINGS_INTEREST',expense,(v->>'vaultId')::uuid,amt);
     INSERT INTO core.savings_interest_postings VALUES(mid,(v->>'vaultId')::uuid,batch,amt);
    END IF;
   END LOOP;
   IF (acc->>'totalMinor')::bigint>0 THEN
    INSERT INTO core.consumer_notifications(person_id,category,title,body,route) VALUES(c.person_id,'SAVINGS','Interest credited',
     'Your vaults earned TST '||to_char((acc->>'totalMinor')::numeric/100,'FM999999999990.00')||' in '||to_char(m,'FMMonth YYYY')||' at '||to_char((acc->'rate'->>'totalBps')::numeric/100,'FM0.00')||'%.','#/save/interest');
   END IF;
   DELETE FROM core.savings_issues WHERE person_id=c.person_id AND subject='interest';
   done:=done+1;
  EXCEPTION WHEN OTHERS THEN
   INSERT INTO core.savings_issues VALUES(c.person_id,'interest',left(SQLERRM,500),clock_timestamp()+interval '5 minutes',clock_timestamp())
   ON CONFLICT(person_id,subject) DO UPDATE SET reason=EXCLUDED.reason,retry_after=EXCLUDED.retry_after,updated_at=EXCLUDED.updated_at;
  END;
 END LOOP;

 FOR ch IN SELECT * FROM core.savings_challenges WHERE state='ACTIVE' AND checked_at<clock_timestamp()-interval '3 seconds'
  AND NOT EXISTS(SELECT 1 FROM core.savings_issues i WHERE i.person_id=savings_challenges.person_id AND i.subject='challenge' AND i.retry_after>clock_timestamp())
  ORDER BY checked_at,person_id LIMIT 25 FOR UPDATE SKIP LOCKED LOOP
  IF NOT pg_try_advisory_xact_lock(hashtextextended(ch.person_id::text,307)) THEN CONTINUE;END IF;
  UPDATE core.savings_challenges SET checked_at=clock_timestamp() WHERE person_id=ch.person_id;
  SELECT coalesce(sum(f.amount_minor),0) INTO progress FROM core.consumer_contribution_facts(ch.person_id) f
   WHERE f.vault_id=ch.vault_id AND f.day BETWEEN ch.started_on AND least(ch.ends_on,today);
  IF progress>=ch.target_minor THEN
   BEGIN
    batch:=core.consumer_post(core.savings_operation('challenge',ch.person_id,ch.started_on),'CHALLENGE_BONUS',expense,ch.vault_id,ch.bonus_minor);
    UPDATE core.savings_challenges SET state='COMPLETED',completed_on=today,batch_id=batch WHERE person_id=ch.person_id;
    INSERT INTO core.consumer_notifications(person_id,category,title,body,route) VALUES(ch.person_id,'SAVINGS','Challenge complete',
     'You saved TST 500.00 within 30 days. A TST 10.00 bonus was added to your vault.','#/save/interest');
    DELETE FROM core.savings_issues WHERE person_id=ch.person_id AND subject='challenge';
    done:=done+1;
   EXCEPTION WHEN OTHERS THEN
    INSERT INTO core.savings_issues VALUES(ch.person_id,'challenge',left(SQLERRM,500),clock_timestamp()+interval '5 minutes',clock_timestamp())
    ON CONFLICT(person_id,subject) DO UPDATE SET reason=EXCLUDED.reason,retry_after=EXCLUDED.retry_after,updated_at=EXCLUDED.updated_at;
   END;
  ELSIF today>ch.ends_on THEN
   UPDATE core.savings_challenges SET state='EXPIRED' WHERE person_id=ch.person_id;done:=done+1;
  END IF;
 END LOOP;

 FOR r IN SELECT * FROM core.savings_referrals WHERE state='PENDING' AND checked_at<clock_timestamp()-interval '3 seconds'
  AND NOT EXISTS(SELECT 1 FROM core.savings_issues i WHERE i.person_id=savings_referrals.referee_person AND i.subject='referral' AND i.retry_after>clock_timestamp())
  ORDER BY checked_at,id LIMIT 25 FOR UPDATE SKIP LOCKED LOOP
  UPDATE core.savings_referrals SET checked_at=clock_timestamp() WHERE id=r.id;
  opened:=core.savings_opened_on(r.referee_person);ok:=false;
  -- The referee's own accounts must close at or above the funding threshold on each of the last 30 closed days, all of
  -- them on or after both the first account opening and the claim.
  IF opened IS NOT NULL AND today-(prog->'referral'->>'holdDays')::integer>=greatest(opened,r.claimed_on) THEN
   WITH accounts AS(SELECT x.account_id FROM core.account_relationships x WHERE x.person_id=r.referee_person AND x.kind='OWNER' AND x.revoked_at IS NULL),
   deltas AS(SELECT b.business_date AS day,sum(j.delta) AS delta FROM core.journal_entries j JOIN core.journal_batches b ON b.id=j.batch_id
    WHERE j.account_id IN(SELECT account_id FROM accounts) AND b.business_date<today GROUP BY 1)
   SELECT bool_and((SELECT coalesce(sum(x.delta),0) FROM deltas x WHERE x.day<=d.day::date)>=(prog->'referral'->>'fundMinor')::bigint) INTO ok
   FROM generate_series(today-(prog->'referral'->>'holdDays')::integer,today-1,interval '1 day') d(day);
  END IF;
  IF coalesce(ok,false) THEN
   IF NOT pg_try_advisory_xact_lock(hashtextextended(r.referee_person::text,307)) OR NOT pg_try_advisory_xact_lock(hashtextextended(r.referrer_person::text,307)) THEN CONTINUE;END IF;
   BEGIN
    referee_account:=core.savings_payout_account(r.referee_person);
    IF referee_account IS NULL THEN RAISE EXCEPTION 'Referee has no active account';END IF;
    batch:=core.consumer_post(core.savings_operation('referee',r.id,r.claimed_on),'REFERRAL_BONUS',expense,referee_account,(prog->'referral'->>'bonusMinor')::bigint);
    SELECT count(*) INTO paid FROM core.savings_referrals WHERE referrer_person=r.referrer_person AND referrer_outcome='PAID' AND date_trunc('year',qualified_on)=date_trunc('year',today);
    referrer_account:=core.savings_payout_account(r.referrer_person);rb:=NULL;
    IF paid>=(prog->'referral'->>'maxPaidPerYear')::integer THEN outcome:='ANNUAL_LIMIT';
    ELSIF referrer_account IS NULL THEN outcome:='NO_ACTIVE_ACCOUNT';
    ELSE outcome:='PAID';rb:=core.consumer_post(core.savings_operation('referrer',r.id,r.claimed_on),'REFERRAL_BONUS',expense,referrer_account,(prog->'referral'->>'bonusMinor')::bigint);
    END IF;
    UPDATE core.savings_referrals SET state='PAID',qualified_on=today,referee_batch=batch,referrer_batch=rb,referrer_outcome=outcome WHERE id=r.id;
    INSERT INTO core.consumer_notifications(person_id,category,title,body,route) VALUES(r.referee_person,'SAVINGS','Referral bonus',
     'You kept TST 100.00 for 30 days. A TST 25.00 referral bonus was added to your account.','#/save/interest');
    IF outcome='PAID' THEN INSERT INTO core.consumer_notifications(person_id,category,title,body,route) VALUES(r.referrer_person,'SAVINGS','Referral bonus',
     'Someone you referred qualified. A TST 25.00 referral bonus was added to your account.','#/save/interest');END IF;
    DELETE FROM core.savings_issues WHERE person_id=r.referee_person AND subject='referral';
    done:=done+1;
   EXCEPTION WHEN OTHERS THEN
    INSERT INTO core.savings_issues VALUES(r.referee_person,'referral',left(SQLERRM,500),clock_timestamp()+interval '5 minutes',clock_timestamp())
    ON CONFLICT(person_id,subject) DO UPDATE SET reason=EXCLUDED.reason,retry_after=EXCLUDED.retry_after,updated_at=EXCLUDED.updated_at;
   END;
  ELSIF today>r.claimed_on+(prog->'referral'->>'expiresAfterDays')::integer THEN
   UPDATE core.savings_referrals SET state='EXPIRED' WHERE id=r.id;done:=done+1;
  END IF;
 END LOOP;
 RETURN done;
END $$;
GRANT EXECUTE ON FUNCTION core.savings_tick() TO bank_core;

-- Read model for the savings hub (UI-09) and the interest tracker (UI-14).
CREATE FUNCTION core.savings_snapshot(p_person uuid) RETURNS jsonb LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,core AS $$
DECLARE prog core.savings_programs;e core.savings_enrollments;today date:=core.savings_today();month date;opened date;paid integer;BEGIN
 SELECT * INTO STRICT prog FROM core.savings_programs WHERE version=1;
 SELECT * INTO e FROM core.savings_enrollments WHERE person_id=p_person;
 month:=date_trunc('month',today)::date;opened:=core.savings_opened_on(p_person);
 SELECT count(*) INTO paid FROM core.savings_referrals WHERE referrer_person=p_person AND referrer_outcome='PAID' AND date_trunc('year',qualified_on)=date_trunc('year',today);
 RETURN jsonb_build_object('asOf',today,
  'program',prog.document||jsonb_build_object('version',prog.version,'digest',prog.digest,
   'baseApyPercent',core.savings_apy((prog.document->>'baseRateBps')::integer)::text,'maxApyPercent',core.savings_apy((prog.document->>'maxRateBps')::integer)::text),
  'enrollment',CASE WHEN e.person_id IS NULL THEN NULL ELSE jsonb_build_object('joinedOn',e.joined_on,'leftOn',e.left_on,'active',e.left_on IS NULL,'programVersion',e.program_version) END,
  'canJoin',e.person_id IS NULL AND core.savings_payout_account(p_person) IS NOT NULL,
  'rate',core.savings_rate(p_person,month),
  'streakWeeks',core.savings_weeks(p_person,month,today),
  'monthToDate',CASE WHEN e.person_id IS NULL OR e.left_on IS NOT NULL THEN NULL ELSE core.savings_accrual(p_person,month,today-1) END,
  'months',coalesce((SELECT jsonb_agg(jsonb_build_object('month',m.month,'totalMinor',m.total_minor::text,'rateBps',(m.document->'rate'->>'totalBps')::integer,
    'apyPercent',m.document->'rate'->>'apyPercent','components',m.document->'rate'->'components','vaults',m.document->'vaults','digest',m.digest,'finalizedAt',m.finalized_at) ORDER BY m.month DESC)
   FROM core.savings_interest_months m WHERE m.person_id=p_person),'[]'::jsonb),
  'lifetimeInterestMinor',(SELECT coalesce(sum(m.total_minor),0)::text FROM core.savings_interest_months m WHERE m.person_id=p_person),
  'vaultInterest',coalesce((SELECT jsonb_object_agg(q.vault_id,q.total::text) FROM(SELECT p.vault_id,sum(p.amount_minor) AS total FROM core.savings_interest_postings p
   JOIN core.savings_interest_months m ON m.id=p.month_id WHERE m.person_id=p_person GROUP BY p.vault_id) q),'{}'::jsonb),
  'challenge',(SELECT jsonb_build_object('vaultId',c.vault_id,'startedOn',c.started_on,'endsOn',c.ends_on,'targetMinor',c.target_minor::text,'bonusMinor',c.bonus_minor::text,
    'state',c.state,'completedOn',c.completed_on,'batchId',c.batch_id,
    'progressMinor',(SELECT coalesce(sum(f.amount_minor),0)::text FROM core.consumer_contribution_facts(p_person) f WHERE f.vault_id=c.vault_id AND f.day BETWEEN c.started_on AND least(c.ends_on,today)))
   FROM core.savings_challenges c WHERE c.person_id=p_person),
  'referral',jsonb_build_object(
   'code',CASE WHEN e.left_on IS NULL THEN e.referral_code END,'paidThisYear',paid,'maxPaidPerYear',(prog.document->'referral'->>'maxPaidPerYear')::integer,
   'referrals',coalesce((SELECT jsonb_agg(jsonb_build_object('id',r.id,'claimedOn',r.claimed_on,'state',r.state,'qualifiedOn',r.qualified_on,'yourBonus',r.referrer_outcome) ORDER BY r.claimed_on DESC,r.id)
    FROM core.savings_referrals r WHERE r.referrer_person=p_person),'[]'::jsonb),
   'claimed',(SELECT jsonb_build_object('claimedOn',r.claimed_on,'state',r.state,'qualifiedOn',r.qualified_on,
     'holdFrom',greatest(opened,r.claimed_on),'holdUntil',greatest(opened,r.claimed_on)+(prog.document->'referral'->>'holdDays')::integer)
    FROM core.savings_referrals r WHERE r.referee_person=p_person),
   'canClaim',NOT EXISTS(SELECT 1 FROM core.savings_referrals r WHERE r.referee_person=p_person) AND (opened IS NULL OR today<=opened+(prog.document->'referral'->>'claimWithinDays')::integer),
   'claimUntil',CASE WHEN opened IS NULL THEN NULL ELSE opened+(prog.document->'referral'->>'claimWithinDays')::integer END));
END $$;

-- Lifetime interest now includes savings interest (copied from schema 61 with one changed filter).
CREATE OR REPLACE FUNCTION core.consumer_progress_at(p_person uuid,p_now timestamptz) RETURNS jsonb
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,core AS $$
DECLARE today date:=(p_now AT TIME ZONE 'UTC')::date;month_start date:=date_trunc('month',(p_now AT TIME ZONE 'UTC')::date)::date;
 days date[];streaks jsonb;cur jsonb;start jsonb;history jsonb;delta numeric;start_total numeric;
 n_all bigint;n_auto bigint;n_roundup bigint;lifetime numeric;first_save date;first_auto date;roundup_100 date;saved_1k date;goal_on date;
 badges jsonb;heatmap jsonb;interest jsonb;BEGIN
 SELECT coalesce(array_agg(DISTINCT f.day),'{}'),count(*),count(*) FILTER(WHERE f.source='AUTO_SAVE'),count(*) FILTER(WHERE f.source='ROUNDUP'),
  coalesce(sum(f.amount_minor),0),min(f.day),min(f.day) FILTER(WHERE f.source='AUTO_SAVE')
 INTO days,n_all,n_auto,n_roundup,lifetime,first_save,first_auto FROM core.consumer_contribution_facts(p_person) f WHERE f.day<=today;
 streaks:=core.consumer_streaks(days,today);
 SELECT f.day INTO roundup_100 FROM core.consumer_contribution_facts(p_person) f WHERE f.day<=today AND f.source='ROUNDUP' ORDER BY f.at,f.batch_id OFFSET 99 LIMIT 1;
 SELECT min(q.day) INTO saved_1k FROM(SELECT f.day,sum(f.amount_minor) OVER(ORDER BY f.at,f.batch_id) AS total FROM core.consumer_contribution_facts(p_person) f WHERE f.day<=today) q WHERE q.total>=100000;
 -- A goal is reached on the first business date a vault's running native balance met its current target.
 SELECT min(q.day) INTO goal_on FROM(
  SELECT b.business_date AS day,v.target_minor,sum(e.delta) OVER(PARTITION BY e.account_id ORDER BY b.created_at,b.id) AS running
  FROM core.journal_entries e JOIN core.journal_batches b ON b.id=e.batch_id JOIN core.consumer_vaults v ON v.id=e.account_id
  WHERE EXISTS(SELECT 1 FROM core.consumer_members m WHERE m.vault_id=v.id AND m.person_id=p_person AND m.state='ACTIVE')) q
 WHERE q.running>=q.target_minor AND q.day<=today;
 badges:=jsonb_build_array(
  jsonb_build_object('code','FIRST_SAVE','title','First deposit','description','Make your first contribution to a vault.','unit','COUNT','progress',least(n_all,1)::text,'target','1','unlockedOn',first_save),
  jsonb_build_object('code','AUTO_SAVER','title','On autopilot','description','Complete your first automatic saving.','unit','COUNT','progress',least(n_auto,1)::text,'target','1','unlockedOn',first_auto),
  jsonb_build_object('code','STREAK_7','title','7-Day Saver','description','Save on seven consecutive days.','unit','DAYS','progress',least((streaks->>'longestDays')::integer,7)::text,'target','7','unlockedOn',core.consumer_day_streak_reached(days,7)),
  jsonb_build_object('code','GOAL_REACHED','title','Goal reached','description','Fill a vault to its current target.','unit','COUNT','progress',CASE WHEN goal_on IS NULL THEN '0' ELSE '1' END,'target','1','unlockedOn',goal_on),
  jsonb_build_object('code','STREAK_30','title','30-Day Saver','description','Save on thirty consecutive days.','unit','DAYS','progress',least((streaks->>'longestDays')::integer,30)::text,'target','30','unlockedOn',core.consumer_day_streak_reached(days,30)),
  jsonb_build_object('code','SAVED_1K','title','First TST 1K','description','Contribute TST 1,000.00 to your vaults in total.','unit','MINOR','progress',least(lifetime,100000)::text,'target','100000','unlockedOn',saved_1k),
  jsonb_build_object('code','ROUNDUPS_100','title','100 Round-Ups','description','Save spare change from one hundred card purchases.','unit','COUNT','progress',least(n_roundup,100)::text,'target','100','unlockedOn',roundup_100),
  jsonb_build_object('code','STREAK_6M','title','6-Month Streak','description','Save in six consecutive calendar months.','unit','MONTHS','progress',least((streaks->>'longestMonths')::integer,6)::text,'target','6','unlockedOn',core.consumer_month_streak_reached(days,6)));
 SELECT jsonb_agg(t.b||jsonb_build_object('unlocked',t.b->'unlockedOn'<>'null'::jsonb) ORDER BY t.ord) INTO badges FROM jsonb_array_elements(badges) WITH ORDINALITY AS t(b,ord);
 SELECT coalesce(jsonb_agg(jsonb_build_object('date',q.day,'amountMinor',q.total::text,'count',q.n) ORDER BY q.day),'[]'::jsonb) INTO heatmap
 FROM(SELECT f.day,sum(f.amount_minor) AS total,count(*) AS n FROM core.consumer_contribution_facts(p_person) f WHERE f.day<=today AND f.day>today-182 GROUP BY f.day) q;
 cur:=core.consumer_net_worth_now(p_person);
 -- Six month starts, oldest first; the last one is the current month start.
 SELECT jsonb_agg(core.consumer_net_worth_at(p_person,m AT TIME ZONE 'UTC')||jsonb_build_object('month',to_char(m,'YYYY-MM')) ORDER BY m) INTO history
 FROM generate_series(month_start-interval '5 months',month_start::timestamp,interval '1 month') m;
 start:=history->-1;
 start_total:=(start->>'totalMinor')::numeric;
 IF (start->>'complete')::boolean THEN delta:=(cur->>'totalMinor')::numeric-start_total;END IF;
 WITH interest_facts AS(
  SELECT b.business_date AS day,e.delta::numeric AS amount FROM core.journal_entries e JOIN core.journal_batches b ON b.id=e.batch_id
  LEFT JOIN core.journal_batches o ON o.id=b.original_batch
  WHERE (e.account_id IN(SELECT x.account_id FROM core.account_relationships x WHERE x.person_id=p_person AND x.kind='OWNER' AND x.revoked_at IS NULL)
  AND (b.posting_kind='PERIODIC_ACCRUAL' OR (b.posting_kind='CORRECTION' AND o.posting_kind='PERIODIC_ACCRUAL')))
  -- Schema 80: savings interest credited into the person's own vaults is interest too.
  OR (b.posting_kind='SAVINGS_INTEREST' AND e.account_id IN(SELECT v.id FROM core.consumer_vaults v WHERE v.person_id=p_person))
 ) SELECT jsonb_build_object('lifetimeMinor',coalesce((SELECT sum(amount) FROM interest_facts),0)::text,'postings',(SELECT count(*) FROM interest_facts),
  'lastPostedOn',(SELECT max(day) FROM interest_facts),
  'months',coalesce((SELECT jsonb_agg(jsonb_build_object('month',q.mon,'amountMinor',q.total::text) ORDER BY q.mon) FROM(
   SELECT to_char(day,'YYYY-MM') AS mon,sum(amount) AS total FROM interest_facts WHERE day>=(month_start-interval '11 months')::date GROUP BY 1) q),'[]'::jsonb))
 INTO interest;
 RETURN jsonb_build_object('asOf',today,
  'netWorth',jsonb_build_object('current',cur,'monthStart',start,'monthStartDate',month_start,
   'deltaMinor',delta::text,'deltaBasisPoints',CASE WHEN delta IS NOT NULL AND start_total>0 THEN trunc(delta*10000/start_total)::text END,
   'history',history),
  'savings',streaks||jsonb_build_object('contributions',n_all,'lifetimeContributedMinor',lifetime::text,'heatmap',heatmap,'heatmapFrom',today-181),
  'badges',badges,
  'interest',interest);
END $$;

-- Schema-77 command router, unchanged except for the savings dispatch.
CREATE OR REPLACE FUNCTION core.consumer_command(p_actor jsonb,p_person uuid,p_key uuid,p_action text,p_data jsonb) RETURNS jsonb
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,core AS $$
DECLARE old core.consumer_commands;target uuid:=gen_random_uuid();result jsonb;keys text[];d jsonb;account uuid;total bigint;refs text[];people uuid[];n integer;share bigint;rest bigint;
 self_share bigint;i integer;other uuid;req core.consumer_requests;pe core.payment_executions;card core.consumer_cards;BEGIN
 IF p_action IN('SAVINGS_JOIN','SAVINGS_LEAVE','SAVINGS_CHALLENGE','REFERRAL_CLAIM') THEN RETURN core.savings_command(p_actor,p_person,p_key,p_action,p_data);END IF;
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
 IF p_action NOT IN('SPLIT_EXPENSE','REQUEST_MARK_PAID','FAVORITE_SET','FAVORITE_REMOVE','SAVE_RULE_SET','SAVE_RULE_DISABLE','VAULT_PHOTO') THEN
  IF p_action='SCHEDULE_REPLACE' THEN RETURN core.consumer_schedule_replace(p_actor,p_person,p_key,p_data);END IF;
  RETURN core.consumer_command_v59(p_actor,p_person,p_key,p_action,p_data);
 END IF;
 PERFORM core.consumer_actor(p_actor,p_person);
 IF p_key IS NULL OR p_key='00000000-0000-0000-0000-000000000000' THEN RAISE EXCEPTION 'Request key required';END IF;
 PERFORM pg_advisory_xact_lock(hashtextextended(p_person::text,307));
 SELECT * INTO old FROM core.consumer_commands WHERE issuer=p_actor->>'issuer' AND subject=p_actor->>'subject' AND client_id=p_actor->>'clientId' AND request_key=p_key;
 IF FOUND THEN IF old.person_id<>p_person OR old.action<>p_action OR old.input<>p_data THEN RAISE EXCEPTION 'Command scope conflict';END IF;RETURN old.result;END IF;
 keys:=CASE p_action WHEN 'SPLIT_EXPENSE' THEN ARRAY['accountId','batchId','references','includeSelf','memo'] WHEN 'REQUEST_MARK_PAID' THEN ARRAY['id','paymentId']
  WHEN 'FAVORITE_SET' THEN ARRAY['reference','label'] WHEN 'FAVORITE_REMOVE' THEN ARRAY['reference'] WHEN 'SAVE_RULE_SET' THEN ARRAY['authorizationId'] WHEN 'VAULT_PHOTO' THEN ARRAY['id','photo'] ELSE ARRAY['id'] END;
 IF jsonb_typeof(p_data) IS DISTINCT FROM 'object' OR EXISTS(SELECT 1 FROM jsonb_object_keys(p_data) k WHERE k<>ALL(keys))
  OR EXISTS(SELECT 1 FROM unnest(keys) k WHERE NOT p_data?k OR (p_data->k='null'::jsonb AND NOT (p_action='VAULT_PHOTO' AND k='photo'))) THEN RAISE EXCEPTION 'Complete supported command fields required';END IF;
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
 ELSIF p_action='VAULT_PHOTO' THEN
  PERFORM 1 FROM core.consumer_vaults WHERE id=(p_data->>'id')::uuid AND person_id=p_person FOR UPDATE;IF NOT FOUND THEN RAISE EXCEPTION 'Own vault required';END IF;
  IF p_data->'photo'='null'::jsonb THEN
   DELETE FROM core.consumer_vault_photos WHERE vault_id=(p_data->>'id')::uuid;result:=jsonb_build_object('id',p_data->>'id','photo',false);
  ELSE
   IF jsonb_typeof(p_data->'photo')<>'object' OR p_data->'photo'->>'mime' NOT IN('image/png','image/jpeg') OR coalesce(p_data->'photo'->>'content','')!~'^[A-Za-z0-9+/]+={0,2}$'
    OR length(p_data->'photo'->>'content')>43692 OR EXISTS(SELECT 1 FROM jsonb_object_keys(p_data->'photo') k WHERE k NOT IN('mime','content')) THEN
    RAISE EXCEPTION 'A PNG or JPEG photo of at most 32 KiB is required';END IF;
   INSERT INTO core.consumer_vault_photos VALUES((p_data->>'id')::uuid,p_person,p_data->'photo'->>'mime',p_data->'photo'->>'content',clock_timestamp())
    ON CONFLICT(vault_id) DO UPDATE SET mime=EXCLUDED.mime,content=EXCLUDED.content,updated_at=EXCLUDED.updated_at;
   result:=jsonb_build_object('id',p_data->>'id','photo',true);
  END IF;
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

-- Schema-77 snapshot, unchanged except for the savings read model.
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
  'safeToSpend',core.consumer_safe_to_spend(p_person,clock_timestamp()),
  'vaultPhotos',coalesce((SELECT jsonb_object_agg(p.vault_id,'data:'||p.mime||';base64,'||p.content) FROM core.consumer_vault_photos p
   WHERE p.vault_id IN (SELECT m.vault_id FROM core.consumer_members m WHERE m.person_id=p_person AND m.state='ACTIVE') OR p.person_id=p_person),'{}'::jsonb),
  'savings',core.savings_snapshot(p_person));
END $$;
GRANT EXECUTE ON FUNCTION core.consumer_snapshot(jsonb,uuid) TO bank_core;
