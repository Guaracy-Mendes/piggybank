-- Core-owned observations of native business-date facts. No money posting or enrollment command.
CREATE INDEX core_journal_entries_account_batch ON core.journal_entries(account_id,batch_id);
CREATE TABLE core.periodic_bases(
 id uuid PRIMARY KEY,issuer text NOT NULL,subject text NOT NULL,client_id text NOT NULL,request_key uuid NOT NULL,
 actor_context jsonb NOT NULL,person_id uuid NOT NULL,policy_id uuid NOT NULL REFERENCES core.periodic_policies(id),
 account_id uuid NOT NULL REFERENCES core.account_registry(id),period_start date NOT NULL,
 document jsonb NOT NULL,digest text NOT NULL CHECK(digest~'^[0-9a-f]{64}$'),captured_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 UNIQUE(issuer,subject,client_id,request_key)
);
REVOKE ALL ON core.periodic_bases FROM bank_core;
GRANT SELECT ON core.periodic_bases TO bank_core;
CREATE TRIGGER immutable_periodic_bases BEFORE UPDATE OR DELETE OR TRUNCATE ON core.periodic_bases FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal();
CREATE FUNCTION core.capture_periodic_basis(p_policy uuid,p_account uuid,p_start date,p_key uuid,p_actor jsonb,p_person uuid)
RETURNS uuid LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,core AS $$
DECLARE saved core.periodic_bases; policy core.periodic_policies; finish date; due timestamptz; candidate date;
 target uuid:=gen_random_uuid(); doc jsonb; count_value integer;
BEGIN
 PERFORM pg_advisory_xact_lock(hashtextextended(jsonb_build_array(p_actor->>'issuer',p_actor->>'subject',p_actor->>'clientId',p_key)::text,179));
 IF p_actor->>'trust' IS DISTINCT FROM 'WORKFORCE' OR p_key IS NULL OR p_key='00000000-0000-0000-0000-000000000000'::uuid
 OR NOT EXISTS(SELECT 1 FROM controls.person_links WHERE issuer=p_actor->>'issuer' AND subject=p_actor->>'subject' AND person_id=p_person)
 OR NOT EXISTS(SELECT 1 FROM controls.role_grants WHERE person_id=p_person AND role='finance_adjuster' AND scope IS NULL AND revoked_at IS NULL AND valid_from<=clock_timestamp() AND valid_until>clock_timestamp())
 OR NOT EXISTS(SELECT 1 FROM controls.case_queue_members WHERE person_id=p_person AND queue='finance-operations' AND capability='REVIEW' AND revoked_at IS NULL) THEN RAISE EXCEPTION 'Current canonical Finance capture authority required'; END IF;
 SELECT * INTO saved FROM core.periodic_bases WHERE issuer=p_actor->>'issuer' AND subject=p_actor->>'subject' AND client_id=p_actor->>'clientId' AND request_key=p_key;
 IF FOUND THEN
  IF saved.policy_id IS DISTINCT FROM p_policy OR saved.account_id IS DISTINCT FROM p_account OR saved.period_start IS DISTINCT FROM p_start OR saved.person_id IS DISTINCT FROM p_person THEN RAISE EXCEPTION 'Native basis request scope conflict'; END IF;
  RETURN saved.id;
 END IF;
 SELECT * INTO STRICT policy FROM core.periodic_policies WHERE id=p_policy;
 IF policy.document->>'activation' IS DISTINCT FROM 'PREVIEW_ONLY' OR policy.document->'calendar'->>'timezone' IS DISTINCT FROM 'UTC' OR p_start IS NULL OR p_start>(clock_timestamp() AT TIME ZONE 'UTC')::date THEN RAISE EXCEPTION 'Observed synthetic UTC policy period required'; END IF;
 CASE policy.document->'calculation'->>'kind'
 WHEN 'DAILY_CREDIT_ACCRUAL' THEN finish:=p_start+1;
 WHEN 'MONTHLY_FIXED_FEE' THEN
  IF extract(day FROM p_start)<>1 THEN RAISE EXCEPTION 'Whole calendar month required'; END IF;
  finish:=(p_start+interval '1 month')::date;
 ELSE RAISE EXCEPTION 'Unsupported native basis policy';
 END CASE;
 IF p_start<policy.effective_from OR finish>policy.effective_until THEN RAISE EXCEPTION 'Basis period outside exact policy'; END IF;
 SELECT d::date INTO candidate FROM generate_series(finish::timestamp,finish::timestamp+interval '14 days',interval '1 day') d
 WHERE d::date>=(policy.document->'calendar'->>'coverageFrom')::date AND d::date<(policy.document->'calendar'->>'coverageUntil')::date
 AND extract(isodow FROM d)::integer IN(SELECT value::integer FROM jsonb_array_elements_text(policy.document->'calendar'->'businessWeekdays'))
 AND NOT (policy.document->'calendar'->'holidays' ? to_char(d::date,'YYYY-MM-DD')) ORDER BY d LIMIT 1;
 IF candidate IS NULL THEN RAISE EXCEPTION 'Covered native basis due date required'; END IF;
 due:=(candidate+(policy.document->'calendar'->>'cutoff')::time) AT TIME ZONE 'UTC';
 -- This single statement pins one MVCC source view for account metadata and ALL selected facts.
 WITH scope AS MATERIALIZED(
  SELECT r.product_version_id,r.created_at registered_at,a.state,a.debit_blocked,a.credit_blocked,a.booked,a.held,clock_timestamp() as_of
  FROM core.account_registry r JOIN core.ledger_accounts a ON a.id=r.id
  WHERE r.id=p_account AND r.product_version_id=policy.product_version_id AND a.kind='CUSTOMER'
 ), facts AS MATERIALIZED(
  SELECT j.*,e.delta FROM core.journal_entries e JOIN core.journal_batches j ON j.id=e.batch_id
  WHERE e.account_id=p_account AND j.business_date<finish ORDER BY j.business_date,j.created_at,j.id LIMIT 1001
 ), rows AS(
  SELECT jsonb_build_object('ordinal',row_number() OVER(ORDER BY business_date,created_at,id),'batchId',id,'operationId',operation_id,'postingKind',posting_kind,'businessDate',business_date,'originalBatchId',original_batch,'postedAt',to_char(created_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'),'deltaMinor',delta::text,'journalFingerprint',fingerprint) item,* FROM facts
 ), totals AS(
  SELECT count(*) n,coalesce(sum(delta::numeric) FILTER(WHERE business_date<p_start),0)::text opening,
  coalesce(sum(delta::numeric) FILTER(WHERE business_date>=p_start),0)::text change,
  coalesce(sum(delta::numeric),0)::text closing,coalesce(jsonb_agg(item ORDER BY business_date,created_at,id),'[]'::jsonb) items FROM rows
 )
 SELECT jsonb_build_object('formatVersion',1,'source','CORE_BUSINESS_DATE_BASIS_V1','activation','PREVIEW_ONLY','currency',jsonb_build_object('code','TST','scale',2),'id',target,'policy',policy.document,'policyDigest',policy.digest,
 'accountId',p_account,'productVersionId',s.product_version_id,'periodStart',p_start,'periodEnd',finish,'dueAt',to_char(due AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'),
 'coreAsOf',to_char(s.as_of AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'),'capturedPersonId',p_person,'accountRegisteredAt',to_char(s.registered_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'),
 'accountState',s.state,'debitBlocked',s.debit_blocked,'creditBlocked',s.credit_blocked,'currentBooked',s.booked::text,'currentHeld',s.held::text,'currentAvailable',(s.booked::numeric-s.held::numeric)::text,
 'periodStatus',CASE WHEN finish<=(s.as_of AT TIME ZONE 'UTC')::date THEN 'CLOSED_PERIOD_OBSERVATION' ELSE 'OPEN_PERIOD_PROVISIONAL' END,
 'openingBooked',t.opening,'periodChange',t.change,'observedClosingBooked',t.closing,'sourceCount',t.n,'sourceDigest',encode(sha256(convert_to(core.periodic_policy_canonical(t.items),'UTF8')),'hex'),'items',t.items)
 INTO doc FROM scope s CROSS JOIN totals t;
 IF doc IS NULL THEN RAISE EXCEPTION 'Exact account/product policy binding required'; END IF;
 count_value:=(doc->>'sourceCount')::integer;
 IF count_value>1000 THEN RAISE EXCEPTION USING ERRCODE='P5101',MESSAGE='Native basis source limit exceeded'; END IF;
 INSERT INTO core.periodic_bases(id,issuer,subject,client_id,request_key,actor_context,person_id,policy_id,account_id,period_start,document,digest,captured_at)
 VALUES(target,p_actor->>'issuer',p_actor->>'subject',p_actor->>'clientId',p_key,p_actor,p_person,p_policy,p_account,p_start,doc,encode(sha256(convert_to(core.periodic_policy_canonical(doc),'UTF8')),'hex'),(doc->>'coreAsOf')::timestamptz);
 RETURN target;
END $$;
GRANT EXECUTE ON FUNCTION core.capture_periodic_basis(uuid,uuid,date,uuid,jsonb,uuid) TO bank_core;
CREATE TABLE finance.basis_audit_cursor(id boolean PRIMARY KEY DEFAULT true CHECK(id),last_id uuid NOT NULL DEFAULT '00000000-0000-0000-0000-000000000000');
INSERT INTO finance.basis_audit_cursor DEFAULT VALUES;
