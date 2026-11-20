-- Schema 78 (UI-13 follow-up): approved merchant saving rules count as savings contributions.
-- consumer_contribution_facts feeds streaks, the saving heatmap, the FIRST_SAVE/STREAK/SAVED_1K badges and the lifetime
-- contributed total. Each rule saving is a posted MERCHANT_SAVE batch recorded once in consumer_save_rule_facts, so it
-- is attributed as source SAVE_RULE. The AUTO_SAVER badge stays tied to scheduled SAVE runs and ROUNDUPS_100 to
-- round-ups, so existing badge meanings do not change. Expand-only: the function keeps its signature and grants.
CREATE INDEX consumer_save_rule_facts_person ON core.consumer_save_rule_facts(person_id);
CREATE OR REPLACE FUNCTION core.consumer_contribution_facts(p_person uuid)
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
  UNION ALL
  SELECT f.batch_id,'SAVE_RULE' FROM core.consumer_save_rule_facts f WHERE f.person_id=p_person
 ) SELECT b.id,e.account_id,b.business_date,b.created_at,e.delta::numeric,o.source
 FROM own o JOIN core.journal_batches b ON b.id=o.batch_id JOIN core.journal_entries e ON e.batch_id=b.id
 JOIN core.consumer_vaults v ON v.id=e.account_id
 WHERE e.delta>0
$$;
