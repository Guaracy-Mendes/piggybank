-- Essential synthetic R1: UTC account-period close, unique native posting, durable job intents.
-- The migration-owner clock seam is only for disposable forward-time tests.
INSERT INTO core.ledger_accounts(id,kind,state,debit_blocked) VALUES('4f1a780b-3e31-48ef-a294-000000000005','CONTROL','ACTIVE',false) ON CONFLICT(id) DO NOTHING;
ALTER TABLE controls.change_proposals DROP CONSTRAINT change_proposals_kind_check;
ALTER TABLE controls.change_proposals ADD CONSTRAINT change_proposals_kind_check CHECK(kind IN ('PRODUCT','TERMS','CORRECTION','MASTER_REFERENCE','RAIL_POLICY','FINANCE_CHART','FINANCE_REPORT','PERIODIC_POLICY','FINANCE_CORRECTION','PERIODIC_OFFER','PERIODIC_ACTIVATION'));
CREATE TABLE core.periodic_activations(
 id uuid PRIMARY KEY,consent_id uuid NOT NULL UNIQUE REFERENCES documents.periodic_enrollment_consents(id),account_id uuid NOT NULL REFERENCES core.account_registry(id),policy_id uuid NOT NULL REFERENCES core.periodic_policies(id),offer_id uuid NOT NULL REFERENCES documents.periodic_offers(id),
 person_id uuid NOT NULL,counter_account_id uuid NOT NULL REFERENCES core.ledger_accounts(id),actor_context jsonb NOT NULL,maker_person uuid NOT NULL,checker_person uuid NOT NULL,details jsonb NOT NULL,details_digest text NOT NULL,
 first_period_start date NOT NULL,document jsonb NOT NULL,digest text NOT NULL,activated_at timestamptz NOT NULL,CHECK(maker_person<>checker_person),UNIQUE(account_id,policy_id)
);
CREATE TABLE core.periodic_activation_stops(id uuid PRIMARY KEY,activation_id uuid NOT NULL UNIQUE REFERENCES core.periodic_activations(id),actor_context jsonb NOT NULL,reason text NOT NULL,stopped_at timestamptz NOT NULL DEFAULT clock_timestamp());
CREATE TABLE core.periodic_finalizations(
 id uuid PRIMARY KEY,activation_id uuid NOT NULL REFERENCES core.periodic_activations(id),account_id uuid NOT NULL,policy_id uuid NOT NULL,period_start date NOT NULL,period_end date NOT NULL,
 document jsonb NOT NULL,digest text NOT NULL,finalized_at timestamptz NOT NULL,UNIQUE(account_id,policy_id,period_start),UNIQUE(activation_id,period_start)
);
CREATE TABLE core.periodic_postings(finalization_id uuid PRIMARY KEY REFERENCES core.periodic_finalizations(id),batch_id uuid UNIQUE REFERENCES core.journal_batches(id),posted_at timestamptz NOT NULL,amount_minor bigint NOT NULL CHECK(amount_minor>=0));
REVOKE ALL ON core.periodic_activations,core.periodic_activation_stops,core.periodic_finalizations,core.periodic_postings FROM bank_core;
GRANT SELECT ON core.periodic_activations,core.periodic_activation_stops,core.periodic_finalizations,core.periodic_postings TO bank_core;
DO $$ DECLARE t text; BEGIN FOREACH t IN ARRAY ARRAY['periodic_activations','periodic_activation_stops','periodic_finalizations','periodic_postings'] LOOP
 EXECUTE format('CREATE TRIGGER %I BEFORE UPDATE OR DELETE OR TRUNCATE ON core.%I FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal()','immutable_'||t,t);
END LOOP; END $$;

CREATE FUNCTION core.periodic_consent_source(p_consent uuid) RETURNS jsonb LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,core AS $$
DECLARE c documents.periodic_enrollment_consents;v jsonb;
BEGIN SELECT * INTO STRICT c FROM documents.periodic_enrollment_consents WHERE id=p_consent;v:=documents.periodic_enrollment(c.account_id,c.offer_id,c.person_id);RETURN v;END $$;
GRANT EXECUTE ON FUNCTION core.periodic_consent_source(uuid) TO bank_core;
CREATE FUNCTION core.periodic_activation_receipt(p_id uuid) RETURNS jsonb LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,core AS $$
DECLARE a core.periodic_activations;
BEGIN SELECT * INTO STRICT a FROM core.periodic_activations WHERE id=p_id;
 IF encode(sha256(convert_to(core.periodic_policy_canonical(a.document),'UTF8')),'hex') IS DISTINCT FROM a.digest OR a.document->>'id' IS DISTINCT FROM a.id::text OR a.document->>'accountId' IS DISTINCT FROM a.account_id::text OR a.document->>'counterAccountId' IS DISTINCT FROM a.counter_account_id::text OR a.document->'policy'->>'id' IS DISTINCT FROM a.policy_id::text OR a.document->>'detailsDigest' IS DISTINCT FROM a.details_digest
 OR a.document->>'consentId' IS DISTINCT FROM a.consent_id::text OR a.document->>'consentDigest' IS DISTINCT FROM a.details->>'consentDigest' OR a.document->>'firstPeriodStart' IS DISTINCT FROM to_char(a.first_period_start,'YYYY-MM-DD') OR (a.document->>'activatedAt')::timestamptz IS DISTINCT FROM a.activated_at OR a.document->'policy' IS DISTINCT FROM (SELECT c.document->'offer'->'material'->'policy' FROM documents.periodic_enrollment_consents c WHERE c.id=a.consent_id) OR a.document->>'policyDigest' IS DISTINCT FROM encode(sha256(convert_to(core.periodic_policy_canonical(a.document->'policy'),'UTF8')),'hex') OR a.details_digest IS DISTINCT FROM encode(sha256(convert_to(core.periodic_policy_canonical(a.details),'UTF8')),'hex')
 OR a.document->>'authorityDigest' IS DISTINCT FROM encode(sha256(convert_to(core.periodic_policy_canonical(jsonb_build_object('id',a.id,'personId',a.person_id,'maker',a.maker_person,'checker',a.checker_person,'actor',a.actor_context)),'UTF8')),'hex') THEN RAISE EXCEPTION 'Activation integrity conflict';END IF;
 RETURN jsonb_build_object('activation',a.document,'digest',a.digest,'stopped',EXISTS(SELECT 1 FROM core.periodic_activation_stops WHERE activation_id=a.id));END $$;
GRANT EXECUTE ON FUNCTION core.periodic_activation_receipt(uuid) TO bank_core;

CREATE FUNCTION core.activate_periodic(p_details jsonb,p_actor jsonb,p_person uuid) RETURNS uuid LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,core AS $$
<<activate_cmd>>
DECLARE c documents.periodic_enrollment_consents;saved core.periodic_activations;approval controls.change_proposals;v jsonb;material jsonb;policy jsonb;chart jsonb;expected_class text;chart_account text;activation_key uuid:=(p_details->>'id')::uuid;counter_id uuid:=(p_details->>'counterAccountId')::uuid;stamp timestamptz;first_date date;finish date;doc jsonb;
BEGIN
 SELECT * INTO STRICT c FROM documents.periodic_enrollment_consents WHERE id=(p_details->>'consentId')::uuid;
 PERFORM pg_advisory_xact_lock(hashtextextended(jsonb_build_array(c.account_id,c.offer_id)::text,204));
 PERFORM 1 FROM core.ledger_accounts WHERE id IN(c.account_id,counter_id) ORDER BY id FOR UPDATE;
 PERFORM documents.require_periodic_actor(p_actor,p_person,true);
 SELECT * INTO saved FROM core.periodic_activations WHERE core.periodic_activations.id=activate_cmd.activation_key;
 IF FOUND THEN IF saved.details IS DISTINCT FROM p_details OR saved.maker_person IS DISTINCT FROM p_person THEN RAISE EXCEPTION 'Activation replay scope conflict';END IF;PERFORM core.periodic_activation_receipt(activation_key);RETURN activation_key;END IF;
 v:=core.periodic_consent_source(c.id);material:=c.document->'offer'->'material';policy:=material->'policy';
 IF v->>'status' IS DISTINCT FROM 'PENDING_ACTIVATION' OR c.digest IS DISTINCT FROM p_details->>'consentDigest' OR (SELECT count(*) FROM jsonb_object_keys(p_details))<>4 OR activation_key IS NULL OR activation_key='00000000-0000-0000-0000-000000000000'::uuid
 OR documents.published_periodic_offer(c.offer_id) IS DISTINCT FROM c.document->'offer' OR documents.periodic_offer_material_v1(c.offer_id,(policy->>'id')::uuid) IS DISTINCT FROM material THEN RAISE EXCEPTION 'Exact unwithdrawn consent and published offer required'; END IF;
 IF NOT EXISTS(SELECT 1 FROM core.account_relationships WHERE account_id=c.account_id AND person_id=c.person_id AND kind='OWNER' AND revoked_at IS NULL) OR (SELECT count(*) FROM core.account_relationships WHERE account_id=c.account_id AND kind='OWNER' AND revoked_at IS NULL)<>1
 OR NOT EXISTS(SELECT 1 FROM core.account_registry WHERE core.account_registry.id=c.account_id AND product_version_id=(material->>'productVersionId')::uuid) OR NOT EXISTS(SELECT 1 FROM core.ledger_accounts WHERE core.ledger_accounts.id=c.account_id AND kind='CUSTOMER' AND state='ACTIVE') OR NOT EXISTS(SELECT 1 FROM core.ledger_accounts WHERE core.ledger_accounts.id=counter_id AND kind='CONTROL' AND state='ACTIVE' AND NOT debit_blocked AND NOT credit_blocked) THEN RAISE EXCEPTION 'Current eligible account and explicit control counter-account required'; END IF;
 SELECT * INTO STRICT approval FROM controls.change_proposals WHERE controls.change_proposals.id=activate_cmd.activation_key FOR SHARE;
 IF approval.kind IS DISTINCT FROM 'PERIODIC_ACTIVATION' OR approval.resource_id IS DISTINCT FROM c.id OR approval.digest IS DISTINCT FROM encode(sha256(convert_to(core.periodic_policy_canonical(p_details),'UTF8')),'hex') OR approval.maker_person IS DISTINCT FROM p_person OR approval.checker_person IS NULL OR approval.checker_person=p_person OR approval.state IS DISTINCT FROM 'APPROVED' OR approval.expires_at<=clock_timestamp() THEN RAISE EXCEPTION 'Current independent exact activation approval required'; END IF;
 PERFORM documents.require_periodic_finance_person(approval.checker_person,true);
 stamp:=core.periodic_clock();
 IF policy->'calculation'->>'kind'='MONTHLY_FIXED_FEE' THEN first_date:=greatest((c.document->>'earliestPeriodStart')::date,(date_trunc('month',stamp AT TIME ZONE 'UTC')+interval '1 month')::date);finish:=(first_date+interval '1 month')::date;expected_class:='REVENUE';
 ELSE first_date:=greatest((c.document->>'earliestPeriodStart')::date,(stamp AT TIME ZONE 'UTC')::date+1);finish:=first_date+1;expected_class:='EXPENSE';END IF;
 IF finish>(policy->>'effectiveUntil')::date THEN RAISE EXCEPTION 'No complete activation period remains';END IF;
 SELECT document INTO chart FROM finance.chart_versions WHERE chart_code='synthetic-chart-v1' AND (document->>'effectiveFrom')::date<=(stamp AT TIME ZONE 'UTC')::date AND (document->>'effectiveUntil')::date>(stamp AT TIME ZONE 'UTC')::date ORDER BY version DESC LIMIT 1;
 chart_account:=chart->'byAccount'->>counter_id::text;IF chart_account IS NULL OR chart->'accounts'->>chart_account IS DISTINCT FROM expected_class THEN RAISE EXCEPTION 'Explicit approved GL counter-account classification required';END IF;
 doc:=jsonb_build_object('id',activation_key,'source','CORE_PERIODIC_ACTIVATION_V1','accountId',c.account_id,'consentId',c.id,'consentDigest',c.digest,'counterAccountId',counter_id,'policy',policy,'policyDigest',material->>'policyDigest','detailsDigest',approval.digest,'firstPeriodStart',to_char(first_date,'YYYY-MM-DD'),'activatedAt',to_char(stamp AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'),'authorityDigest',encode(sha256(convert_to(core.periodic_policy_canonical(jsonb_build_object('id',activation_key,'personId',c.person_id,'maker',p_person,'checker',approval.checker_person,'actor',p_actor)),'UTF8')),'hex'));
 PERFORM documents.require_periodic_actor(p_actor,p_person,true);PERFORM documents.require_periodic_finance_person(approval.checker_person,true);IF approval.expires_at<=clock_timestamp() THEN RAISE EXCEPTION 'Activation approval expired while waiting';END IF;
 INSERT INTO core.periodic_activations VALUES(activation_key,c.id,c.account_id,(policy->>'id')::uuid,c.offer_id,c.person_id,counter_id,p_actor,p_person,approval.checker_person,p_details,approval.digest,first_date,doc,encode(sha256(convert_to(core.periodic_policy_canonical(doc),'UTF8')),'hex'),stamp);
 RETURN activation_key;
END $$;
GRANT EXECUTE ON FUNCTION core.activate_periodic(jsonb,jsonb,uuid) TO bank_core;

CREATE FUNCTION documents.guard_activated_withdrawal() RETURNS trigger LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,documents AS $$ BEGIN
 IF EXISTS(SELECT 1 FROM core.periodic_activations WHERE consent_id=NEW.consent_id) THEN RAISE EXCEPTION 'Active enrollment requires Finance suspension; pending withdrawal is unavailable';END IF;RETURN NEW;END $$;
CREATE TRIGGER activated_withdrawal_guard BEFORE INSERT ON documents.periodic_enrollment_withdrawals FOR EACH ROW EXECUTE FUNCTION documents.guard_activated_withdrawal();

CREATE FUNCTION core.guard_runtime_booking_date() RETURNS trigger LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,core AS $$ BEGIN
 IF session_user='bank_core' THEN NEW.business_date:=(core.periodic_clock() AT TIME ZONE 'UTC')::date; END IF;RETURN NEW;END $$;
CREATE TRIGGER authoritative_runtime_business_date BEFORE INSERT ON core.journal_batches FOR EACH ROW EXECUTE FUNCTION core.guard_runtime_booking_date();
CREATE FUNCTION core.guard_closed_period_entry() RETURNS trigger LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,core AS $$ BEGIN
 PERFORM 1 FROM core.ledger_accounts WHERE id=NEW.account_id FOR UPDATE;
 IF EXISTS(SELECT 1 FROM core.periodic_finalizations f JOIN core.journal_batches j ON j.id=NEW.batch_id WHERE f.account_id=NEW.account_id AND j.business_date<f.period_end) THEN RAISE EXCEPTION 'Closed account-period facts cannot be backdated';END IF;RETURN NEW;END $$;
CREATE TRIGGER closed_period_entry_guard BEFORE INSERT ON core.journal_entries FOR EACH ROW EXECUTE FUNCTION core.guard_closed_period_entry();

CREATE FUNCTION core.finalize_periodic_period(p_activation uuid,p_start date,p_actor jsonb,p_person uuid) RETURNS uuid LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,core AS $$
DECLARE a core.periodic_activations;f core.periodic_finalizations;policy jsonb;finish date;candidate date;due timestamptz;stamp timestamptz;total numeric;amount numeric;facts jsonb;expected date;target uuid:=gen_random_uuid();doc jsonb;source_count integer;
BEGIN
 SELECT * INTO STRICT a FROM core.periodic_activations WHERE id=p_activation;
 PERFORM pg_advisory_xact_lock(hashtextextended(jsonb_build_array(a.account_id,a.offer_id)::text,204));
 PERFORM 1 FROM core.ledger_accounts WHERE id IN(a.account_id,a.counter_account_id) ORDER BY id FOR UPDATE;
 PERFORM documents.require_periodic_actor(p_actor,p_person,true);PERFORM core.periodic_activation_receipt(a.id);
 SELECT * INTO f FROM core.periodic_finalizations WHERE activation_id=a.id AND period_start=p_start;
 IF FOUND THEN RETURN f.id;END IF;
 IF EXISTS(SELECT 1 FROM core.periodic_activation_stops WHERE activation_id=a.id) OR EXISTS(SELECT 1 FROM documents.periodic_enrollment_withdrawals WHERE consent_id=a.consent_id)
 OR NOT EXISTS(SELECT 1 FROM core.account_relationships WHERE account_id=a.account_id AND person_id=a.person_id AND kind='OWNER' AND revoked_at IS NULL) OR (SELECT count(*) FROM core.account_relationships WHERE account_id=a.account_id AND kind='OWNER' AND revoked_at IS NULL)<>1 THEN RAISE EXCEPTION 'Current active sole-owner enrollment required';END IF;
 policy:=a.document->'policy';
 IF EXISTS(SELECT 1 FROM core.periodic_finalizations z LEFT JOIN core.periodic_postings p ON p.finalization_id=z.id WHERE z.activation_id=a.id AND p.finalization_id IS NULL) THEN RAISE EXCEPTION 'Complete the original finalized period first';END IF;
 SELECT coalesce(max(period_end),a.first_period_start) INTO expected FROM core.periodic_finalizations WHERE activation_id=a.id;
 IF p_start IS DISTINCT FROM expected THEN RAISE EXCEPTION 'Next sequential economic period required';END IF;
 IF policy->'calculation'->>'kind'='MONTHLY_FIXED_FEE' THEN finish:=(p_start+interval '1 month')::date;ELSE finish:=p_start+1;END IF;
 SELECT d::date INTO candidate FROM generate_series(finish::timestamp,finish::timestamp+interval '14 days',interval '1 day') d WHERE extract(isodow FROM d)::integer IN(SELECT value::integer FROM jsonb_array_elements_text(policy->'calendar'->'businessWeekdays')) AND NOT (policy->'calendar'->'holidays' ? to_char(d::date,'YYYY-MM-DD')) ORDER BY d LIMIT 1;
 due:=(candidate+(policy->'calendar'->>'cutoff')::time) AT TIME ZONE 'UTC';stamp:=core.periodic_clock();
 IF due IS NULL OR finish>(policy->>'effectiveUntil')::date OR due>stamp OR finish>(stamp AT TIME ZONE 'UTC')::date THEN RAISE EXCEPTION USING ERRCODE='P5501',MESSAGE='Whole period must be closed and due';END IF;
 SELECT coalesce(jsonb_agg(jsonb_build_object('batchId',q.id,'businessDate',to_char(q.business_date,'YYYY-MM-DD'),'deltaMinor',q.delta::text,'fingerprint',q.fingerprint) ORDER BY q.business_date,q.created_at,q.id),'[]'::jsonb),coalesce(sum(q.delta::numeric),0),count(*) INTO facts,total,source_count FROM (SELECT j.*,e.delta FROM core.journal_entries e JOIN core.journal_batches j ON j.id=e.batch_id WHERE e.account_id=a.account_id AND j.business_date<finish ORDER BY j.business_date,j.created_at,j.id LIMIT 1001) q;
 IF source_count>1000 THEN RAISE EXCEPTION 'R1 native account history exceeds 1000-fact bound';END IF;
 IF policy->'calculation'->>'kind'='MONTHLY_FIXED_FEE' THEN amount:=(policy->'calculation'->'amount'->>'minorUnits')::numeric;
 ELSE IF total<0 OR total>9223372036854775807 THEN RAISE EXCEPTION 'Representable nonnegative finalized booked basis required';END IF;amount:=trunc((total*(policy->'calculation'->>'annualBasisPoints')::numeric+1825000)/3650000);END IF;
 IF amount<0 OR amount>9223372036854775807 THEN RAISE EXCEPTION 'Representable periodic amount required';END IF;
 doc:=jsonb_build_object('id',target,'source','CORE_ACCOUNT_PERIOD_FINALIZATION_V1','activationId',a.id,'accountId',a.account_id,'policyId',a.policy_id,'periodStart',to_char(p_start,'YYYY-MM-DD'),'periodEnd',to_char(finish,'YYYY-MM-DD'),'dueAt',to_char(due AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'),'finalizedAt',to_char(stamp AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'),'closingBookedMinor',total::text,'amountMinor',amount::text,'sourceCount',source_count,'sourceFacts',facts,'sourceDigest',encode(sha256(convert_to(core.periodic_policy_canonical(facts),'UTF8')),'hex'));
 INSERT INTO core.periodic_finalizations VALUES(target,a.id,a.account_id,a.policy_id,p_start,finish,doc,encode(sha256(convert_to(core.periodic_policy_canonical(doc),'UTF8')),'hex'),stamp);RETURN target;
END $$;
GRANT EXECUTE ON FUNCTION core.finalize_periodic_period(uuid,date,jsonb,uuid) TO bank_core;

CREATE FUNCTION core.periodic_period_receipt(p_id uuid) RETURNS jsonb LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,core AS $$
DECLARE f core.periodic_finalizations;a core.periodic_activations;p core.periodic_postings;facts jsonb;total numeric;amount numeric;expected_legs jsonb;legs jsonb;kind text;delta bigint;
BEGIN
 SELECT * INTO STRICT f FROM core.periodic_finalizations WHERE id=p_id;SELECT * INTO STRICT a FROM core.periodic_activations WHERE id=f.activation_id;PERFORM core.periodic_activation_receipt(a.id);
 SELECT coalesce(jsonb_agg(jsonb_build_object('batchId',q.id,'businessDate',to_char(q.business_date,'YYYY-MM-DD'),'deltaMinor',q.delta::text,'fingerprint',q.fingerprint) ORDER BY q.business_date,q.created_at,q.id),'[]'::jsonb),coalesce(sum(q.delta::numeric),0) INTO facts,total FROM (SELECT j.*,e.delta FROM core.journal_entries e JOIN core.journal_batches j ON j.id=e.batch_id WHERE e.account_id=f.account_id AND j.business_date<f.period_end ORDER BY j.business_date,j.created_at,j.id LIMIT 1001) q;
 IF a.document->'policy'->'calculation'->>'kind'='MONTHLY_FIXED_FEE' THEN amount:=(a.document->'policy'->'calculation'->'amount'->>'minorUnits')::numeric;kind:='PERIODIC_FEE';delta:=-amount::bigint;
 ELSE amount:=trunc((total*(a.document->'policy'->'calculation'->>'annualBasisPoints')::numeric+1825000)/3650000);kind:='PERIODIC_ACCRUAL';delta:=amount::bigint;END IF;
 IF encode(sha256(convert_to(core.periodic_policy_canonical(f.document),'UTF8')),'hex') IS DISTINCT FROM f.digest OR f.document->>'id' IS DISTINCT FROM f.id::text OR f.document->>'activationId' IS DISTINCT FROM a.id::text OR f.document->>'accountId' IS DISTINCT FROM f.account_id::text OR f.document->>'policyId' IS DISTINCT FROM f.policy_id::text
 OR f.document->>'periodStart' IS DISTINCT FROM to_char(f.period_start,'YYYY-MM-DD') OR f.document->>'periodEnd' IS DISTINCT FROM to_char(f.period_end,'YYYY-MM-DD') OR (f.document->>'finalizedAt')::timestamptz IS DISTINCT FROM f.finalized_at OR f.document->'sourceFacts' IS DISTINCT FROM facts OR f.document->>'closingBookedMinor' IS DISTINCT FROM total::text OR f.document->>'amountMinor' IS DISTINCT FROM amount::text
 OR f.document->>'sourceDigest' IS DISTINCT FROM encode(sha256(convert_to(core.periodic_policy_canonical(facts),'UTF8')),'hex') THEN RAISE EXCEPTION 'Native finalization source integrity conflict';END IF;
 SELECT * INTO p FROM core.periodic_postings WHERE finalization_id=f.id;
 IF FOUND THEN
  IF p.amount_minor IS DISTINCT FROM amount::bigint OR (amount=0 AND p.batch_id IS NOT NULL) OR (amount>0 AND p.batch_id IS NULL) THEN RAISE EXCEPTION 'Period posting receipt scope conflict';END IF;
  IF amount>0 THEN
   SELECT jsonb_agg(jsonb_build_object('accountId',account_id,'delta',e.delta) ORDER BY account_id) INTO legs FROM core.journal_entries e WHERE batch_id=p.batch_id;
   SELECT jsonb_agg(x ORDER BY x->>'accountId') INTO expected_legs FROM jsonb_array_elements(jsonb_build_array(jsonb_build_object('accountId',a.account_id,'delta',delta),jsonb_build_object('accountId',a.counter_account_id,'delta',-delta))) x;
   IF legs IS DISTINCT FROM expected_legs OR NOT EXISTS(SELECT 1 FROM core.journal_batches WHERE id=p.batch_id AND operation_id=f.id AND posting_kind=kind AND original_batch IS NULL AND business_date>=f.period_end) THEN RAISE EXCEPTION 'Authoritative periodic journal receipt required';END IF;
  END IF;
 END IF;
 RETURN jsonb_build_object('period',f.document,'digest',f.digest,'posted',p.finalization_id IS NOT NULL,'batchId',p.batch_id,'amountMinor',amount::text);
END $$;
GRANT EXECUTE ON FUNCTION core.periodic_period_receipt(uuid) TO bank_core;

CREATE FUNCTION core.post_periodic_period(p_finalization uuid,p_actor jsonb,p_person uuid) RETURNS uuid LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,core AS $$
DECLARE f core.periodic_finalizations;a core.periodic_activations;existing core.periodic_postings;target uuid;made boolean;amount bigint;delta bigint;chart jsonb;code text;class text;event jsonb;
BEGIN
 SELECT * INTO STRICT f FROM core.periodic_finalizations WHERE id=p_finalization;SELECT * INTO STRICT a FROM core.periodic_activations WHERE id=f.activation_id;
 PERFORM pg_advisory_xact_lock(hashtextextended(jsonb_build_array(a.account_id,a.offer_id)::text,204));
 PERFORM 1 FROM core.ledger_accounts WHERE id IN(a.account_id,a.counter_account_id) ORDER BY id FOR UPDATE;
 PERFORM documents.require_periodic_actor(p_actor,p_person,true);PERFORM core.periodic_activation_receipt(a.id);
 PERFORM core.periodic_period_receipt(f.id);
 SELECT * INTO existing FROM core.periodic_postings WHERE finalization_id=f.id;IF FOUND THEN RETURN existing.batch_id;END IF;
 IF EXISTS(SELECT 1 FROM core.periodic_activation_stops WHERE activation_id=a.id) OR EXISTS(SELECT 1 FROM documents.periodic_enrollment_withdrawals WHERE consent_id=a.consent_id) OR NOT EXISTS(SELECT 1 FROM core.account_relationships WHERE account_id=a.account_id AND person_id=a.person_id AND kind='OWNER' AND revoked_at IS NULL) OR (SELECT count(*) FROM core.account_relationships WHERE account_id=a.account_id AND kind='OWNER' AND revoked_at IS NULL)<>1 THEN RAISE EXCEPTION 'Current active enrollment required';END IF;
 SELECT document INTO chart FROM finance.chart_versions WHERE chart_code='synthetic-chart-v1' AND (document->>'effectiveFrom')::date<=(core.periodic_clock() AT TIME ZONE 'UTC')::date AND (document->>'effectiveUntil')::date>(core.periodic_clock() AT TIME ZONE 'UTC')::date ORDER BY version DESC LIMIT 1;
 code:=chart->'byAccount'->>a.counter_account_id::text;class:=CASE WHEN a.document->'policy'->'calculation'->>'kind'='MONTHLY_FIXED_FEE' THEN 'REVENUE' ELSE 'EXPENSE' END;
 IF code IS NULL OR chart->'accounts'->>code IS DISTINCT FROM class THEN RAISE EXCEPTION 'Current approved counter-account mapping required';END IF;
 IF NOT EXISTS(SELECT 1 FROM core.ledger_accounts WHERE id=a.account_id AND state='ACTIVE' AND kind='CUSTOMER' AND (CASE WHEN a.document->'policy'->'calculation'->>'kind'='MONTHLY_FIXED_FEE' THEN NOT debit_blocked ELSE NOT credit_blocked END)) THEN RAISE EXCEPTION 'Active customer account required';END IF;
 PERFORM documents.require_periodic_actor(p_actor,p_person,true);
 amount:=(f.document->>'amountMinor')::bigint;
 IF amount>0 THEN
  delta:=CASE WHEN class='REVENUE' THEN -amount ELSE amount END;
  SELECT r.batch_id,r.created INTO target,made FROM core.post(f.id,CASE WHEN class='REVENUE' THEN 'PERIODIC_FEE' ELSE 'PERIODIC_ACCRUAL' END,jsonb_build_array(jsonb_build_object('accountId',a.account_id,'delta',delta),jsonb_build_object('accountId',a.counter_account_id,'delta',-delta))) r;
 END IF;
 INSERT INTO core.periodic_postings VALUES(f.id,target,clock_timestamp(),amount);
 IF target IS NOT NULL THEN
  event:=jsonb_build_object('eventId',target,'eventType','journal.batch-posted','schemaVersion',1,'producer','core','aggregateId',target,'aggregateVersion',1,'occurredAt',to_char(clock_timestamp() AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'),'correlationId',f.id,'causationId',f.id::text,'data',jsonb_build_object('batchId',target,'operationId',f.id));
  INSERT INTO core.outbox(event_id,aggregate_id,aggregate_version,document,fingerprint) VALUES(target,target,1,event,encode(sha256(convert_to(core.periodic_policy_canonical(event),'UTF8')),'hex'));
 END IF;RETURN target;
END $$;
GRANT EXECUTE ON FUNCTION core.post_periodic_period(uuid,jsonb,uuid) TO bank_core;

CREATE FUNCTION core.stop_periodic_activation(p_id uuid,p_actor jsonb,p_person uuid,p_reason text) RETURNS void LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,core AS $$
DECLARE a core.periodic_activations;
BEGIN SELECT * INTO STRICT a FROM core.periodic_activations WHERE id=p_id;PERFORM pg_advisory_xact_lock(hashtextextended(jsonb_build_array(a.account_id,a.offer_id)::text,204));PERFORM documents.require_periodic_actor(p_actor,p_person,true);
 IF length(p_reason) NOT BETWEEN 4 AND 240 THEN RAISE EXCEPTION 'Stop reason required';END IF;
 INSERT INTO core.periodic_activation_stops VALUES(gen_random_uuid(),p_id,p_actor,p_reason,clock_timestamp()) ON CONFLICT(activation_id) DO NOTHING;END $$;
GRANT EXECUTE ON FUNCTION core.stop_periodic_activation(uuid,jsonb,uuid,text) TO bank_core;

CREATE TABLE finance.periodic_activation_requests(id uuid PRIMARY KEY,issuer text NOT NULL,subject text NOT NULL,client_id text NOT NULL,request_key uuid NOT NULL,actor_context jsonb NOT NULL,maker_person uuid NOT NULL,details jsonb NOT NULL,digest text NOT NULL,expires_at timestamptz NOT NULL,created_at timestamptz NOT NULL DEFAULT clock_timestamp(),UNIQUE(issuer,subject,client_id,request_key));
CREATE TRIGGER immutable_periodic_activation_requests BEFORE UPDATE OR DELETE OR TRUNCATE ON finance.periodic_activation_requests FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal();
CREATE TABLE finance.periodic_runs(id uuid PRIMARY KEY,issuer text NOT NULL,subject text NOT NULL,client_id text NOT NULL,request_key uuid NOT NULL,actor_context jsonb NOT NULL,activation_id uuid NOT NULL,period_start date NOT NULL,operation_id uuid,last_error text NOT NULL DEFAULT '',created_at timestamptz NOT NULL DEFAULT clock_timestamp(),UNIQUE(issuer,subject,client_id,request_key));
CREATE FUNCTION finance.guard_periodic_run_scope() RETURNS trigger LANGUAGE plpgsql SET search_path=pg_catalog AS $$ BEGIN
 IF TG_OP <> 'UPDATE' THEN RAISE EXCEPTION 'Periodic run identity is retained';END IF;
 IF (to_jsonb(NEW)-'operation_id'-'last_error') IS DISTINCT FROM (to_jsonb(OLD)-'operation_id'-'last_error') OR (OLD.operation_id IS NOT NULL AND NEW.operation_id IS DISTINCT FROM OLD.operation_id) THEN RAISE EXCEPTION 'Periodic run scope is immutable';END IF; RETURN NEW;
END $$;
CREATE TRIGGER periodic_run_scope_guard BEFORE UPDATE OR DELETE ON finance.periodic_runs FOR EACH ROW EXECUTE FUNCTION finance.guard_periodic_run_scope();
CREATE TRIGGER periodic_run_truncate_guard BEFORE TRUNCATE ON finance.periodic_runs FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal();
CREATE TABLE finance.periodic_run_cursor(id boolean PRIMARY KEY DEFAULT true CHECK(id),last_id uuid NOT NULL DEFAULT '00000000-0000-0000-0000-000000000000',last_activation uuid NOT NULL DEFAULT '00000000-0000-0000-0000-000000000000');INSERT INTO finance.periodic_run_cursor DEFAULT VALUES;
CREATE TABLE finance.periodic_issues(activation_id uuid PRIMARY KEY,reason text NOT NULL,case_id uuid,updated_at timestamptz NOT NULL DEFAULT clock_timestamp());

CREATE OR REPLACE FUNCTION documents.periodic_enrollment(p_account uuid,p_offer uuid,p_person uuid) RETURNS jsonb
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,documents AS $$
DECLARE c documents.periodic_enrollment_consents;w documents.periodic_enrollment_withdrawals;material jsonb;activation jsonb;aid uuid;stopped boolean;
BEGIN
 SELECT * INTO c FROM documents.periodic_enrollment_consents WHERE account_id=p_account AND offer_id=p_offer AND person_id=p_person;IF NOT FOUND THEN RETURN NULL; END IF;
 material:=c.document->'offer'->'material';
 IF encode(sha256(convert_to(core.periodic_policy_canonical(c.document),'UTF8')),'hex') IS DISTINCT FROM c.digest
 OR c.document->>'id' IS DISTINCT FROM c.id::text OR c.document->>'accountId' IS DISTINCT FROM c.account_id::text OR c.document->'offer'->'material'->>'id' IS DISTINCT FROM c.offer_id::text
 OR (c.document->>'acceptedAt')::timestamptz IS DISTINCT FROM c.accepted_at OR c.document->>'kind' IS DISTINCT FROM 'PERIODIC_ENROLLMENT_CONSENT'
 OR c.document->>'attributionDigest' IS DISTINCT FROM encode(sha256(convert_to(core.periodic_policy_canonical(jsonb_build_object('id',c.id,'personId',c.person_id,'actor',c.actor_context)),'UTF8')),'hex')
 OR encode(sha256(convert_to(core.periodic_policy_canonical(material),'UTF8')),'hex') IS DISTINCT FROM c.document->'offer'->>'digest'
 OR encode(sha256(convert_to(core.periodic_policy_canonical(material->'policy'),'UTF8')),'hex') IS DISTINCT FROM material->>'policyDigest'
 THEN RAISE EXCEPTION 'Consent integrity conflict'; END IF;
 SELECT * INTO w FROM documents.periodic_enrollment_withdrawals WHERE consent_id=c.id;
 IF FOUND THEN
  IF encode(sha256(convert_to(core.periodic_policy_canonical(w.document),'UTF8')),'hex') IS DISTINCT FROM w.digest
  OR w.document->>'id' IS DISTINCT FROM w.id::text OR w.document->>'consentId' IS DISTINCT FROM c.id::text OR w.document->>'consentDigest' IS DISTINCT FROM c.digest OR (w.document->>'withdrawnAt')::timestamptz IS DISTINCT FROM w.withdrawn_at OR w.person_id IS DISTINCT FROM c.person_id
  OR w.document->>'attributionDigest' IS DISTINCT FROM encode(sha256(convert_to(core.periodic_policy_canonical(jsonb_build_object('id',w.id,'personId',w.person_id,'actor',w.actor_context)),'UTF8')),'hex') THEN RAISE EXCEPTION 'Withdrawal integrity conflict'; END IF;
 END IF;
 SELECT id INTO aid FROM core.periodic_activations WHERE consent_id=c.id;
 IF aid IS NOT NULL THEN IF w.id IS NOT NULL THEN RAISE EXCEPTION 'Activated consent cannot have a pending withdrawal';END IF;activation:=core.periodic_activation_receipt(aid);stopped:=(activation->>'stopped')::boolean;END IF;
 RETURN jsonb_build_object('consent',c.document,'consentDigest',c.digest,'withdrawal',w.document,'withdrawalDigest',w.digest,'activation',activation,'status',CASE WHEN w.id IS NOT NULL THEN 'WITHDRAWN' WHEN aid IS NULL THEN 'PENDING_ACTIVATION' WHEN stopped THEN 'PAUSED' ELSE 'ACTIVE' END,'financialActivation',CASE WHEN aid IS NULL THEN 'DISABLED' WHEN stopped THEN 'PAUSED' ELSE 'ENABLED' END);
END $$;
