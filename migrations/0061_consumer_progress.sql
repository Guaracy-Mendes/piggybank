-- Savings progress, monthly net worth and lifetime interest are derived read models.
-- They read retained native facts only; nothing here posts money or stores new financial state.

-- Pure UTC date-island arithmetic. Days after p_today are ignored.
CREATE FUNCTION core.consumer_streaks(p_days date[],p_today date) RETURNS jsonb LANGUAGE sql IMMUTABLE SET search_path=pg_catalog AS $$
 WITH d AS(SELECT DISTINCT x AS day FROM unnest(p_days) x WHERE x IS NOT NULL AND x<=p_today),
 di AS(SELECT day,day-(row_number() OVER(ORDER BY day))::integer AS grp FROM d),
 islands AS(SELECT max(day) AS last_day,count(*)::integer AS len FROM di GROUP BY grp),
 m AS(SELECT DISTINCT date_trunc('month',day)::date AS mon FROM d),
 mi AS(SELECT mon,(extract(year FROM mon)*12+extract(month FROM mon))::integer-(row_number() OVER(ORDER BY mon))::integer AS grp FROM m),
 mislands AS(SELECT max(mon) AS last_month,count(*)::integer AS len FROM mi GROUP BY grp)
 SELECT jsonb_build_object(
  'currentDays',coalesce((SELECT len FROM islands WHERE last_day>=p_today-1),0),
  'longestDays',coalesce((SELECT max(len) FROM islands),0),
  'currentMonths',coalesce((SELECT len FROM mislands WHERE last_month>=(date_trunc('month',p_today)-interval '1 month')::date),0),
  'longestMonths',coalesce((SELECT max(len) FROM mislands),0),
  'savingDays',(SELECT count(*) FROM d),
  'lastSavingDate',(SELECT max(day) FROM d))
$$;
-- First day on which a run of p_n consecutive saving days was completed.
CREATE FUNCTION core.consumer_day_streak_reached(p_days date[],p_n integer) RETURNS date LANGUAGE sql IMMUTABLE SET search_path=pg_catalog AS $$
 WITH d AS(SELECT DISTINCT x AS day FROM unnest(p_days) x WHERE x IS NOT NULL),
 di AS(SELECT day,day-(row_number() OVER(ORDER BY day))::integer AS grp FROM d),
 islands AS(SELECT min(day) AS first_day,count(*)::integer AS len FROM di GROUP BY grp)
 SELECT min(first_day+(p_n-1)) FROM islands WHERE p_n>0 AND len>=p_n
$$;
-- First saving day in the month that completed a run of p_n consecutive saving months.
CREATE FUNCTION core.consumer_month_streak_reached(p_days date[],p_n integer) RETURNS date LANGUAGE sql IMMUTABLE SET search_path=pg_catalog AS $$
 WITH d AS(SELECT DISTINCT x AS day FROM unnest(p_days) x WHERE x IS NOT NULL),
 m AS(SELECT DISTINCT date_trunc('month',day)::date AS mon FROM d),
 mi AS(SELECT mon,(extract(year FROM mon)*12+extract(month FROM mon))::integer-(row_number() OVER(ORDER BY mon))::integer AS grp FROM m),
 islands AS(SELECT min(mon) AS first_month,count(*)::integer AS len FROM mi GROUP BY grp),
 reached AS(SELECT min((first_month+make_interval(months=>p_n-1))::date) AS mon FROM islands WHERE p_n>0 AND len>=p_n)
 SELECT min(day) FROM d,reached WHERE date_trunc('month',day)::date=reached.mon
$$;

-- Positive vault-side entries attributable to the person, driven from the person's own records:
-- their VAULT_DEPOSIT commands, their recurring SAVE runs and their round-up facts.
CREATE INDEX consumer_commands_person_action ON core.consumer_commands(person_id,action);
CREATE INDEX consumer_roundup_facts_person ON core.consumer_roundup_facts(person_id);
CREATE FUNCTION core.consumer_contribution_facts(p_person uuid)
RETURNS TABLE(batch_id uuid,vault_id uuid,day date,at timestamptz,amount_minor numeric,source text)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog,core AS $$
 WITH own(batch_id,source) AS(
  SELECT b.id,'DEPOSIT' FROM core.consumer_commands c JOIN core.journal_batches b ON b.operation_id=c.id
  WHERE c.person_id=p_person AND c.action='VAULT_DEPOSIT'
  UNION ALL
  SELECT r.batch_id,'AUTO_SAVE' FROM core.consumer_schedules s JOIN core.consumer_schedule_runs r ON r.schedule_id=s.id
  WHERE s.person_id=p_person AND s.kind='SAVE' AND r.batch_id IS NOT NULL
  UNION ALL
  SELECT f.batch_id,'ROUNDUP' FROM core.consumer_roundup_facts f WHERE f.person_id=p_person
 ) SELECT b.id,e.account_id,b.business_date,b.created_at,e.delta::numeric,o.source
 FROM own o JOIN core.journal_batches b ON b.id=o.batch_id JOIN core.journal_entries e ON e.batch_id=b.id
 JOIN core.consumer_vaults v ON v.id=e.account_id
 WHERE e.delta>0
$$;

-- Personal net worth under the same definitions as the dashboard: currently owned bank accounts,
-- personal (single active member, owned) vaults, brokerage cash and positions. Shared pools are excluded.
-- Historical values replay journal entries and immutable position events before p_at. Each held symbol is
-- valued at the latest retained price observation at or before p_at, else the person's latest fill price
-- before p_at; without either the value is reported incomplete instead of guessed.
CREATE FUNCTION core.consumer_net_worth_at(p_person uuid,p_at timestamptz) RETURNS jsonb
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,core AS $$
DECLARE bank numeric;vault numeric;cash numeric;equity numeric:=0;missing text[]:='{}';p record;price bigint;BEGIN
 SELECT coalesce(sum(e.delta),0) INTO bank FROM core.journal_entries e JOIN core.journal_batches b ON b.id=e.batch_id
 WHERE b.created_at<p_at AND e.account_id IN(SELECT x.account_id FROM core.account_relationships x JOIN core.account_registry r ON r.id=x.account_id WHERE x.person_id=p_person AND x.kind='OWNER' AND x.revoked_at IS NULL);
 SELECT coalesce(sum(e.delta),0) INTO vault FROM core.journal_entries e JOIN core.journal_batches b ON b.id=e.batch_id
 WHERE b.created_at<p_at AND e.account_id IN(SELECT v.id FROM core.consumer_vaults v WHERE v.person_id=p_person
  AND EXISTS(SELECT 1 FROM core.consumer_members m WHERE m.vault_id=v.id AND m.person_id=p_person AND m.state='ACTIVE')
  AND (SELECT count(*) FROM core.consumer_members m WHERE m.vault_id=v.id AND m.state='ACTIVE')=1);
 SELECT coalesce(sum(e.delta),0) INTO cash FROM core.journal_entries e JOIN core.journal_batches b ON b.id=e.batch_id
 WHERE b.created_at<p_at AND e.account_id=(SELECT cash_account_id FROM core.wealth_accounts WHERE person_id=p_person);
 FOR p IN SELECT symbol,sum(units_delta)::numeric AS units FROM core.wealth_position_events
  WHERE person_id=p_person AND created_at<p_at GROUP BY symbol HAVING sum(units_delta)<>0 ORDER BY symbol LOOP
  price:=NULL;
  SELECT o.price_minor INTO price FROM core.wealth_price_observations o WHERE o.symbol=p.symbol AND o.observed_at<=p_at ORDER BY o.observed_at DESC LIMIT 1;
  IF price IS NULL THEN
   SELECT f.price_minor INTO price FROM core.wealth_fills f JOIN core.wealth_orders w ON w.id=f.order_id
   WHERE w.person_id=p_person AND f.symbol=p.symbol AND f.filled_at<p_at ORDER BY f.filled_at DESC LIMIT 1;
  END IF;
  IF price IS NULL THEN missing:=missing||p.symbol;ELSE equity:=equity+trunc(p.units*price/1000000);END IF;
 END LOOP;
 RETURN jsonb_build_object('at',p_at,'bankMinor',bank::text,'vaultMinor',vault::text,'investmentMinor',(cash+equity)::text,
  'totalMinor',(bank+vault+cash+equity)::text,'complete',cardinality(missing)=0,'missingPrices',to_jsonb(missing));
END $$;
-- Current net worth reads the same live balances and fixture prices the dashboard shows.
CREATE FUNCTION core.consumer_net_worth_now(p_person uuid) RETURNS jsonb
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,core AS $$
DECLARE bank numeric;vault numeric;cash numeric;equity numeric;BEGIN
 SELECT coalesce(sum(l.booked),0) INTO bank FROM core.account_relationships x JOIN core.account_registry r ON r.id=x.account_id JOIN core.ledger_accounts l ON l.id=x.account_id
 WHERE x.person_id=p_person AND x.kind='OWNER' AND x.revoked_at IS NULL;
 SELECT coalesce(sum(l.booked),0) INTO vault FROM core.consumer_vaults v JOIN core.ledger_accounts l ON l.id=v.id WHERE v.person_id=p_person
  AND EXISTS(SELECT 1 FROM core.consumer_members m WHERE m.vault_id=v.id AND m.person_id=p_person AND m.state='ACTIVE')
  AND (SELECT count(*) FROM core.consumer_members m WHERE m.vault_id=v.id AND m.state='ACTIVE')=1;
 SELECT coalesce(sum(l.booked),0) INTO cash FROM core.wealth_accounts w JOIN core.ledger_accounts l ON l.id=w.cash_account_id WHERE w.person_id=p_person;
 SELECT coalesce(sum(trunc(p.units::numeric*a.price_minor/1000000)),0) INTO equity FROM core.wealth_positions p JOIN core.wealth_assets a USING(symbol) WHERE p.person_id=p_person;
 RETURN jsonb_build_object('at',clock_timestamp(),'bankMinor',bank::text,'vaultMinor',vault::text,'investmentMinor',(cash+equity)::text,
  'totalMinor',(bank+vault+cash+equity)::text,'complete',true,'missingPrices','[]'::jsonb);
END $$;

CREATE FUNCTION core.consumer_progress_at(p_person uuid,p_now timestamptz) RETURNS jsonb
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
  WHERE e.account_id IN(SELECT x.account_id FROM core.account_relationships x WHERE x.person_id=p_person AND x.kind='OWNER' AND x.revoked_at IS NULL)
  AND (b.posting_kind='PERIODIC_ACCRUAL' OR (b.posting_kind='CORRECTION' AND o.posting_kind='PERIODIC_ACCRUAL'))
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

ALTER FUNCTION core.consumer_snapshot(jsonb,uuid) RENAME TO consumer_snapshot_v60;
REVOKE EXECUTE ON FUNCTION core.consumer_snapshot_v60(jsonb,uuid) FROM bank_core;
CREATE FUNCTION core.consumer_snapshot(p_actor jsonb,p_person uuid) RETURNS jsonb LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,core AS $$
DECLARE d jsonb;BEGIN
 -- The original snapshot validates the current actor and ownership before any progress is derived.
 d:=core.consumer_snapshot_v60(p_actor,p_person);
 RETURN d||jsonb_build_object('progress',core.consumer_progress_at(p_person,clock_timestamp()));
END $$;
GRANT EXECUTE ON FUNCTION core.consumer_snapshot(jsonb,uuid) TO bank_core;
