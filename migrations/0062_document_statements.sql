-- owner: documents. WP-25 period statements, short-lived retrieval, fixture retention, legal holds,
-- permitted disposal evidence and data-subject requests. Core journals remain the only financial source.
CREATE FUNCTION documents.record_digest(p_value jsonb) RETURNS text LANGUAGE sql IMMUTABLE STRICT SECURITY DEFINER SET search_path=pg_catalog AS
$$ SELECT encode(sha256(convert_to(core.periodic_policy_canonical(p_value),'UTF8')),'hex') $$;
CREATE FUNCTION documents.records_today() RETURNS date LANGUAGE sql VOLATILE SECURITY DEFINER SET search_path=pg_catalog AS
$$ SELECT (core.periodic_clock() AT TIME ZONE 'UTC')::date $$;
CREATE FUNCTION documents.utc_text(p_at timestamptz) RETURNS text LANGUAGE sql IMMUTABLE STRICT SET search_path=pg_catalog AS
$$ SELECT to_char(p_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"') $$;

-- Versioned fixture policies. A record keeps the policy assigned when it was created; later versions never shorten it.
CREATE TABLE documents.retention_policies(
 id uuid PRIMARY KEY,record_class text NOT NULL CHECK(record_class IN ('STATEMENT','DATA_EXPORT','ACCESS_EVIDENCE')),
 version integer NOT NULL CHECK(version>0),retain_days integer NOT NULL CHECK(retain_days BETWEEN 0 AND 36500),
 anchor text NOT NULL CHECK(anchor IN ('PERIOD_END','CREATED')),disposal text NOT NULL CHECK(disposal IN ('PERMITTED','NOT_ENABLED')),
 label text NOT NULL CHECK(label='SYNTHETIC_FIXTURE'),effective_from date NOT NULL,document jsonb NOT NULL,digest text NOT NULL CHECK(length(digest)=64),
 UNIQUE(record_class,version)
);
CREATE TRIGGER immutable_retention_policies BEFORE UPDATE OR DELETE OR TRUNCATE ON documents.retention_policies FOR EACH STATEMENT EXECUTE FUNCTION documents.immutable_record();
INSERT INTO documents.retention_policies(id,record_class,version,retain_days,anchor,disposal,label,effective_from,document,digest)
SELECT id,c,1,d,a,p,'SYNTHETIC_FIXTURE','2026-01-01',doc,documents.record_digest(doc) FROM (VALUES
 ('7a3c0e52-25d0-4c8e-9d61-000000000001'::uuid,'STATEMENT',2557,'PERIOD_END','PERMITTED','Synthetic fixture: statement objects are retained seven years after the period end, then may be disposed unless a legal hold applies.'),
 ('7a3c0e52-25d0-4c8e-9d61-000000000002'::uuid,'DATA_EXPORT',30,'CREATED','PERMITTED','Synthetic fixture: data-access export manifests are retained thirty days, then may be disposed unless a legal hold applies.'),
 ('7a3c0e52-25d0-4c8e-9d61-000000000003'::uuid,'ACCESS_EVIDENCE',3650,'CREATED','NOT_ENABLED','Synthetic fixture: access, hold, disposal and request evidence is retained; disposal is not enabled in this release.')
) v(id,c,d,a,p,note), LATERAL (SELECT jsonb_build_object('id',id,'recordClass',c,'version',1,'retainDays',d,'anchor',a,'disposal',p,'label','SYNTHETIC_FIXTURE','effectiveFrom','2026-01-01','note',note) AS doc) x;

CREATE TABLE documents.statement_versions(
 id uuid PRIMARY KEY,account_id uuid NOT NULL REFERENCES core.account_registry(id),owners uuid[] NOT NULL CHECK(cardinality(owners)>0),
 period_start date NOT NULL CHECK(extract(day FROM period_start)=1),period_end date NOT NULL,
 version integer NOT NULL CHECK(version>0),supersedes uuid UNIQUE REFERENCES documents.statement_versions(id),
 reason text NOT NULL CHECK(reason IN ('INITIAL','LINKED_CORRECTION','SOURCE_CHANGED')),
 snapshot_digest text NOT NULL CHECK(length(snapshot_digest)=64),content_digest text NOT NULL CHECK(length(content_digest)=64),size_bytes integer NOT NULL CHECK(size_bytes>0),
 retention_policy_id uuid NOT NULL REFERENCES documents.retention_policies(id),retain_until date NOT NULL,
 actor_context jsonb NOT NULL,created_at timestamptz NOT NULL,
 UNIQUE(account_id,period_start,version),CHECK(period_end=(period_start+interval '1 month')::date),
 CHECK((version=1)=(supersedes IS NULL)),CHECK((version=1)=(reason='INITIAL')),CHECK(retain_until>=period_end)
);
CREATE TRIGGER immutable_statement_versions BEFORE UPDATE OR DELETE OR TRUNCATE ON documents.statement_versions FOR EACH STATEMENT EXECUTE FUNCTION documents.immutable_record();
-- The restricted object. Its metadata row stays as a minimized tombstone after permitted disposal.
CREATE TABLE documents.statement_objects(version_id uuid PRIMARY KEY REFERENCES documents.statement_versions(id),document jsonb NOT NULL,digest text NOT NULL CHECK(length(digest)=64));
CREATE TABLE documents.statement_disposals(
 version_id uuid PRIMARY KEY REFERENCES documents.statement_versions(id),person_id uuid NOT NULL,actor_context jsonb NOT NULL,
 reason text NOT NULL CHECK(length(reason) BETWEEN 3 AND 500),data_request_id uuid,business_date date NOT NULL,
 evidence jsonb NOT NULL,evidence_digest text NOT NULL CHECK(length(evidence_digest)=64),disposed_at timestamptz NOT NULL
);
CREATE TRIGGER immutable_statement_disposals BEFORE UPDATE OR DELETE OR TRUNCATE ON documents.statement_disposals FOR EACH STATEMENT EXECUTE FUNCTION documents.immutable_record();
CREATE TABLE documents.statement_links(
 id uuid PRIMARY KEY,version_id uuid NOT NULL REFERENCES documents.statement_versions(id),token_digest text NOT NULL UNIQUE CHECK(length(token_digest)=64),
 issuer text NOT NULL,subject text NOT NULL,client_id text NOT NULL,trust text NOT NULL CHECK(trust IN ('CUSTOMER','WORKFORCE')),person_id uuid NOT NULL,
 created_at timestamptz NOT NULL,expires_at timestamptz NOT NULL,used_at timestamptz,
 CHECK(expires_at>created_at AND expires_at<=created_at+interval '5 minutes'),CHECK(used_at IS NULL OR used_at>=created_at)
);
CREATE INDEX statement_links_version ON documents.statement_links(version_id);
CREATE TABLE documents.legal_holds(
 id uuid PRIMARY KEY,issuer text NOT NULL,subject text NOT NULL,client_id text NOT NULL,request_key uuid NOT NULL,
 scope text NOT NULL CHECK(scope IN ('ACCOUNT','PERSON')),account_id uuid REFERENCES core.account_registry(id),person_id uuid,
 reason text NOT NULL CHECK(length(reason) BETWEEN 3 AND 500),reference text NOT NULL CHECK(length(reference) BETWEEN 1 AND 64),
 placed_by uuid NOT NULL,actor_context jsonb NOT NULL,placed_at timestamptz NOT NULL,
 CHECK((scope='ACCOUNT')=(account_id IS NOT NULL) AND (scope='PERSON')=(person_id IS NOT NULL)),UNIQUE(issuer,subject,client_id,request_key)
);
CREATE TABLE documents.legal_hold_releases(
 hold_id uuid PRIMARY KEY REFERENCES documents.legal_holds(id),released_by uuid NOT NULL,actor_context jsonb NOT NULL,
 reason text NOT NULL CHECK(length(reason) BETWEEN 3 AND 500),released_at timestamptz NOT NULL
);
CREATE TABLE documents.data_requests(
 id uuid PRIMARY KEY,issuer text NOT NULL,subject text NOT NULL,client_id text NOT NULL,request_key uuid NOT NULL,person_id uuid NOT NULL,
 kind text NOT NULL CHECK(kind IN ('ACCESS','CORRECTION','DELETION')),detail text NOT NULL CHECK(length(detail)<=500),
 verification text NOT NULL CHECK(verification='AUTHENTICATED_CUSTOMER_SESSION'),actor_context jsonb NOT NULL,submitted_at timestamptz NOT NULL,
 UNIQUE(issuer,subject,client_id,request_key)
);
CREATE TABLE documents.data_request_outcomes(
 request_id uuid PRIMARY KEY REFERENCES documents.data_requests(id),processed_by uuid NOT NULL,actor_context jsonb NOT NULL,
 outcome text NOT NULL CHECK(outcome IN ('ALLOWED','PARTIALLY_RESTRICTED','RESTRICTED')),items jsonb NOT NULL CHECK(jsonb_typeof(items)='array'),
 digest text NOT NULL CHECK(length(digest)=64),processed_at timestamptz NOT NULL
);
CREATE TABLE documents.data_exports(
 request_id uuid PRIMARY KEY REFERENCES documents.data_request_outcomes(request_id),person_id uuid NOT NULL,document jsonb NOT NULL,digest text NOT NULL CHECK(length(digest)=64),
 retention_policy_id uuid NOT NULL REFERENCES documents.retention_policies(id),retain_until date NOT NULL,created_at timestamptz NOT NULL
);
CREATE TABLE documents.data_export_disposals(
 request_id uuid PRIMARY KEY REFERENCES documents.data_requests(id),person_id uuid NOT NULL,actor_context jsonb NOT NULL,
 reason text NOT NULL CHECK(length(reason) BETWEEN 3 AND 500),business_date date NOT NULL,evidence jsonb NOT NULL,
 evidence_digest text NOT NULL CHECK(length(evidence_digest)=64),disposed_at timestamptz NOT NULL
);
CREATE TABLE documents.document_access(
 id uuid PRIMARY KEY DEFAULT gen_random_uuid(),version_id uuid,account_id uuid,link_id uuid,
 issuer text NOT NULL,subject text NOT NULL,trust text NOT NULL,person_id uuid,
 action text NOT NULL CHECK(action IN ('GENERATE','LINK','RETRIEVE','LEGAL_HOLD','HOLD_RELEASE','DISPOSE','DATA_REQUEST','DATA_REQUEST_PROCESSED','EXPORT_DISPOSE')),
 outcome text NOT NULL CHECK(outcome IN ('GRANTED','DENIED','NOT_READY','CREATED','REPLAYED','COMPLETED','RESTRICTED')),
 reason text NOT NULL CHECK(length(reason) BETWEEN 1 AND 200),occurred_at timestamptz NOT NULL DEFAULT clock_timestamp()
);
CREATE INDEX document_access_account ON documents.document_access(account_id,occurred_at DESC);
CREATE INDEX document_access_version ON documents.document_access(version_id);
CREATE INDEX statement_versions_owners ON documents.statement_versions USING gin(owners);
CREATE INDEX legal_holds_account ON documents.legal_holds(account_id) WHERE account_id IS NOT NULL;
CREATE INDEX legal_holds_person ON documents.legal_holds(person_id) WHERE person_id IS NOT NULL;
CREATE INDEX data_requests_person ON documents.data_requests(person_id,submitted_at DESC);
DO $$ DECLARE t text;BEGIN FOREACH t IN ARRAY ARRAY['legal_holds','legal_hold_releases','data_requests','data_request_outcomes','data_export_disposals','document_access'] LOOP
 EXECUTE format('CREATE TRIGGER immutable_%s BEFORE UPDATE OR DELETE OR TRUNCATE ON documents.%I FOR EACH STATEMENT EXECUTE FUNCTION documents.immutable_record()',t,t);END LOOP;END $$;
DO $$ DECLARE t text;BEGIN FOREACH t IN ARRAY ARRAY['retention_policies','statement_versions','statement_objects','statement_disposals','statement_links','legal_holds','legal_hold_releases','data_requests','data_request_outcomes','data_exports','data_export_disposals','document_access'] LOOP
 EXECUTE format('REVOKE ALL ON documents.%I FROM bank_documents',t);END LOOP;END $$;
GRANT SELECT ON documents.retention_policies TO bank_documents;

-- Links may only be consumed once; nothing else about a grant changes.
CREATE FUNCTION documents.guard_statement_link() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN
 IF TG_OP<>'UPDATE' OR OLD.used_at IS NOT NULL OR NEW.used_at IS NULL OR (to_jsonb(NEW)-'used_at') IS DISTINCT FROM (to_jsonb(OLD)-'used_at') THEN RAISE EXCEPTION 'Immutable document record'; END IF;RETURN NEW;END $$;
CREATE TRIGGER statement_link_guard BEFORE UPDATE OR DELETE ON documents.statement_links FOR EACH ROW EXECUTE FUNCTION documents.guard_statement_link();
CREATE TRIGGER statement_link_truncate BEFORE TRUNCATE ON documents.statement_links FOR EACH STATEMENT EXECUTE FUNCTION documents.immutable_record();

CREATE FUNCTION documents.account_owners(p_account uuid) RETURNS uuid[] LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS
$$ SELECT coalesce(array_agg(person_id ORDER BY person_id),'{}') FROM core.account_relationships WHERE account_id=p_account AND kind='OWNER' AND revoked_at IS NULL $$;
CREATE FUNCTION documents.is_owner(p_account uuid,p_person uuid) RETURNS boolean LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS
$$ SELECT EXISTS(SELECT 1 FROM core.account_relationships WHERE account_id=p_account AND person_id=p_person AND kind='OWNER' AND revoked_at IS NULL) $$;
-- An unreleased hold covers an account, the owners recorded on a version, and the account's current owners.
CREATE FUNCTION documents.hold_active(p_account uuid,p_owners uuid[]) RETURNS boolean LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS
$$ SELECT EXISTS(SELECT 1 FROM documents.legal_holds h WHERE NOT EXISTS(SELECT 1 FROM documents.legal_hold_releases r WHERE r.hold_id=h.id)
   AND (h.account_id=p_account OR h.person_id=ANY(p_owners||documents.account_owners(p_account)))) $$;
CREATE FUNCTION documents.person_hold_active(p_person uuid) RETURNS boolean LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS
$$ SELECT EXISTS(SELECT 1 FROM documents.legal_holds h WHERE NOT EXISTS(SELECT 1 FROM documents.legal_hold_releases r WHERE r.hold_id=h.id)
   AND (h.person_id=p_person OR h.account_id IN (SELECT account_id FROM core.account_relationships WHERE person_id=p_person AND kind='OWNER'))) $$;

-- Defense in depth: even the owner path cannot record disposal of a retained or held object.
CREATE FUNCTION documents.guard_statement_disposal() RETURNS trigger LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,documents AS $$
DECLARE v documents.statement_versions;BEGIN
 SELECT * INTO STRICT v FROM documents.statement_versions WHERE id=NEW.version_id;
 IF v.retain_until>documents.records_today() THEN RAISE EXCEPTION 'Retention period has not ended'; END IF;
 IF documents.hold_active(v.account_id,v.owners) THEN RAISE EXCEPTION 'Active legal hold prevents disposal'; END IF;
 RETURN NEW;END $$;
CREATE TRIGGER statement_disposal_guard BEFORE INSERT ON documents.statement_disposals FOR EACH ROW EXECUTE FUNCTION documents.guard_statement_disposal();
CREATE FUNCTION documents.guard_statement_object() RETURNS trigger LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,documents AS $$ BEGIN
 IF TG_OP='DELETE' AND EXISTS(SELECT 1 FROM documents.statement_disposals WHERE version_id=OLD.version_id) THEN RETURN OLD; END IF;
 RAISE EXCEPTION 'Statement objects change only through recorded permitted disposal';END $$;
CREATE TRIGGER statement_object_guard BEFORE UPDATE OR DELETE ON documents.statement_objects FOR EACH ROW EXECUTE FUNCTION documents.guard_statement_object();
CREATE TRIGGER statement_object_truncate BEFORE TRUNCATE ON documents.statement_objects FOR EACH STATEMENT EXECUTE FUNCTION documents.immutable_record();
CREATE FUNCTION documents.guard_export_disposal() RETURNS trigger LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,documents AS $$
DECLARE x documents.data_exports;BEGIN
 SELECT * INTO STRICT x FROM documents.data_exports WHERE request_id=NEW.request_id;
 IF x.retain_until>documents.records_today() THEN RAISE EXCEPTION 'Retention period has not ended'; END IF;
 IF documents.person_hold_active(x.person_id) THEN RAISE EXCEPTION 'Active legal hold prevents disposal'; END IF;
 RETURN NEW;END $$;
CREATE TRIGGER export_disposal_guard BEFORE INSERT ON documents.data_export_disposals FOR EACH ROW EXECUTE FUNCTION documents.guard_export_disposal();
CREATE FUNCTION documents.guard_data_export() RETURNS trigger LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,documents AS $$ BEGIN
 IF TG_OP='DELETE' AND EXISTS(SELECT 1 FROM documents.data_export_disposals WHERE request_id=OLD.request_id) THEN RETURN OLD; END IF;
 RAISE EXCEPTION 'Exports change only through recorded permitted disposal';END $$;
CREATE TRIGGER data_export_guard BEFORE UPDATE OR DELETE ON documents.data_exports FOR EACH ROW EXECUTE FUNCTION documents.guard_data_export();
CREATE TRIGGER data_export_truncate BEFORE TRUNCATE ON documents.data_exports FOR EACH STATEMENT EXECUTE FUNCTION documents.immutable_record();
CREATE FUNCTION documents.guard_hold_release() RETURNS trigger LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,documents AS $$ BEGIN
 IF EXISTS(SELECT 1 FROM documents.legal_holds WHERE id=NEW.hold_id AND placed_by=NEW.released_by) THEN RAISE EXCEPTION 'Independent legal hold release required'; END IF;RETURN NEW;END $$;
CREATE TRIGGER hold_release_guard BEFORE INSERT ON documents.legal_hold_releases FOR EACH ROW EXECUTE FUNCTION documents.guard_hold_release();

CREATE FUNCTION documents.require_records_actor(p_actor jsonb,p_person uuid,p_roles text[]) RETURNS void
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,documents AS $$ BEGIN
 IF p_actor->>'trust' IS DISTINCT FROM 'WORKFORCE' THEN RAISE EXCEPTION 'Current trust context required'; END IF;
 PERFORM 1 FROM controls.person_links WHERE issuer=p_actor->>'issuer' AND subject=p_actor->>'subject' AND person_id=p_person FOR SHARE;
 IF NOT FOUND THEN RAISE EXCEPTION 'Current canonical identity required'; END IF;
 IF coalesce(length(p_actor->>'clientId'),0) NOT BETWEEN 1 AND 256 THEN RAISE EXCEPTION 'Current client required'; END IF;
 PERFORM 1 FROM controls.role_grants WHERE person_id=p_person AND role=ANY(p_roles) AND scope IS NULL AND revoked_at IS NULL AND valid_from<=clock_timestamp() AND valid_until>clock_timestamp() FOR SHARE;
 IF NOT FOUND THEN RAISE EXCEPTION 'Current records role required'; END IF;
END $$;
CREATE FUNCTION documents.statement_actor(p_actor jsonb,p_person uuid,p_staff boolean) RETURNS void
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,documents AS $$ BEGIN
 IF p_staff THEN PERFORM documents.require_records_actor(p_actor,p_person,ARRAY['account_support','records_officer']);
 ELSE PERFORM documents.require_periodic_actor(p_actor,p_person,false); END IF;
END $$;
CREATE FUNCTION documents.audit(p_actor jsonb,p_person uuid,p_action text,p_outcome text,p_reason text,p_version uuid DEFAULT NULL,p_account uuid DEFAULT NULL,p_link uuid DEFAULT NULL) RETURNS void
LANGUAGE sql VOLATILE SECURITY DEFINER SET search_path=pg_catalog AS $$
 INSERT INTO documents.document_access(version_id,account_id,link_id,issuer,subject,trust,person_id,action,outcome,reason)
 VALUES(p_version,p_account,p_link,coalesce(p_actor->>'issuer','unknown'),coalesce(p_actor->>'subject','unknown'),coalesce(p_actor->>'trust','unknown'),p_person,p_action,p_outcome,left(p_reason,200)) $$;

-- Readiness: the UTC month has ended in Core's clock and every enrolled periodic policy is finalized through it.
CREATE FUNCTION documents.statement_readiness(p_account uuid,p_start date) RETURNS jsonb
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,documents AS $$
DECLARE finish date;today date:=documents.records_today();reasons text[]:='{}';periodic jsonb:='[]';a record;watermark date;opened date;
BEGIN
 IF p_start IS NULL OR p_start<>date_trunc('month',p_start)::date THEN RAISE EXCEPTION 'Calendar-month statement period required' USING ERRCODE='22023'; END IF;
 finish:=(p_start+interval '1 month')::date;
 SELECT (created_at AT TIME ZONE 'UTC')::date INTO opened FROM core.account_registry WHERE id=p_account;
 IF NOT FOUND THEN RAISE EXCEPTION 'Account unavailable' USING ERRCODE='P0002'; END IF;
 IF opened>=finish THEN reasons:=array_append(reasons,'ACCOUNT_NOT_OPEN_IN_PERIOD'); END IF;
 IF finish>today THEN reasons:=array_append(reasons,'PERIOD_OPEN'); END IF;
 FOR a IN SELECT pa.id,pa.policy_id,s.stopped_at FROM core.periodic_activations pa LEFT JOIN core.periodic_activation_stops s ON s.activation_id=pa.id
  WHERE pa.account_id=p_account AND pa.first_period_start<finish ORDER BY pa.id LOOP
  SELECT max(period_end) INTO watermark FROM core.periodic_finalizations WHERE activation_id=a.id;
  periodic:=periodic||jsonb_build_object('activationId',a.id,'policyId',a.policy_id,'finalizedThrough',watermark,'stoppedAt',documents.utc_text(a.stopped_at));
  IF (a.stopped_at IS NULL OR a.stopped_at>=(finish::timestamp AT TIME ZONE 'UTC')) AND (watermark IS NULL OR watermark<finish) AND NOT ('PERIODIC_FINALIZATION_PENDING'=ANY(reasons)) THEN
   reasons:=array_append(reasons,'PERIODIC_FINALIZATION_PENDING');
  END IF;
 END LOOP;
 RETURN jsonb_build_object('periodStart',p_start,'periodEnd',finish,'asOfDate',today,'ready',cardinality(reasons)=0,'reasons',to_jsonb(reasons),'periodic',periodic);
END $$;

-- Exact native facts for one account-month. Refuses rather than truncating or substituting a current balance.
CREATE FUNCTION documents.statement_snapshot(p_account uuid,p_start date,p_finish date) RETURNS jsonb
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,documents AS $$
DECLARE n integer;items jsonb;opening numeric;closing numeric;moved numeric;credits numeric;debits numeric;fees numeric;interest numeric;corrections numeric;linked jsonb;linked_count integer;
BEGIN
 SELECT count(*) INTO n FROM core.journal_entries e JOIN core.journal_batches j ON j.id=e.batch_id WHERE e.account_id=p_account AND j.business_date>=p_start AND j.business_date<p_finish;
 IF n>1000 THEN RAISE EXCEPTION 'Statement exceeds 1000 source facts; refusing truncation'; END IF;
 SELECT coalesce(jsonb_agg(jsonb_build_object('batchId',j.id,'businessDate',to_char(j.business_date,'YYYY-MM-DD'),'postedAt',documents.utc_text(j.created_at),'kind',j.posting_kind,'deltaMinor',e.delta::text,'originalBatchId',j.original_batch) ORDER BY j.business_date,j.created_at,j.id),'[]'::jsonb),
  coalesce(sum(e.delta::numeric),0),coalesce(sum(e.delta::numeric) FILTER(WHERE e.delta>0),0),coalesce(-sum(e.delta::numeric) FILTER(WHERE e.delta<0),0),
  coalesce(sum(e.delta::numeric) FILTER(WHERE j.posting_kind='PERIODIC_FEE'),0),coalesce(sum(e.delta::numeric) FILTER(WHERE j.posting_kind='PERIODIC_ACCRUAL'),0),
  coalesce(sum(e.delta::numeric) FILTER(WHERE j.original_batch IS NOT NULL),0)
 INTO items,moved,credits,debits,fees,interest,corrections
 FROM core.journal_entries e JOIN core.journal_batches j ON j.id=e.batch_id WHERE e.account_id=p_account AND j.business_date>=p_start AND j.business_date<p_finish;
 SELECT coalesce(sum(e.delta::numeric),0) INTO opening FROM core.journal_entries e JOIN core.journal_batches j ON j.id=e.batch_id WHERE e.account_id=p_account AND j.business_date<p_start;
 SELECT coalesce(sum(e.delta::numeric),0) INTO closing FROM core.journal_entries e JOIN core.journal_batches j ON j.id=e.batch_id WHERE e.account_id=p_account AND j.business_date<p_finish;
 IF closing IS DISTINCT FROM opening+moved THEN RAISE EXCEPTION 'Statement totals do not reconcile'; END IF;
 -- Linked corrections booked after the period are disclosed beside, never inside, the closed period totals.
 SELECT coalesce(jsonb_agg(jsonb_build_object('batchId',j.id,'businessDate',to_char(j.business_date,'YYYY-MM-DD'),'postedAt',documents.utc_text(j.created_at),'kind',j.posting_kind,'deltaMinor',e.delta::text,'originalBatchId',j.original_batch) ORDER BY j.business_date,j.created_at,j.id),'[]'::jsonb),count(*)
 INTO linked,linked_count FROM core.journal_entries e JOIN core.journal_batches j ON j.id=e.batch_id JOIN core.journal_batches o ON o.id=j.original_batch
 WHERE e.account_id=p_account AND j.business_date>=p_finish AND o.business_date>=p_start AND o.business_date<p_finish
   AND EXISTS(SELECT 1 FROM core.journal_entries oe WHERE oe.batch_id=o.id AND oe.account_id=p_account);
 IF linked_count>1000 THEN RAISE EXCEPTION 'Statement exceeds 1000 linked corrections; refusing truncation'; END IF;
 RETURN jsonb_build_object('openingMinor',opening::text,'closingMinor',closing::text,'items',items,'linkedCorrections',linked,
  'totals',jsonb_build_object('creditsMinor',credits::text,'debitsMinor',debits::text,'netMinor',moved::text,'feesMinor',fees::text,'interestMinor',interest::text,'correctionsMinor',corrections::text,'itemCount',n,'linkedCorrectionCount',linked_count));
END $$;

CREATE FUNCTION documents.generate_statement_internal(p_actor jsonb,p_person uuid,p_account uuid,p_start date) RETURNS jsonb
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,documents AS $$
DECLARE ready jsonb;finish date;snap jsonb;basis jsonb;sdigest text;prior documents.statement_versions;prior_doc jsonb;reason text;policy documents.retention_policies;
 owners uuid[];doc jsonb;cdigest text;target uuid:=gen_random_uuid();stamp timestamptz;next_version integer;
BEGIN
 PERFORM pg_advisory_xact_lock(hashtextextended('documents.statement:'||p_account||':'||p_start,0));
 -- Wait for in-flight posters to commit; every later runtime booking takes Core's current UTC date.
 PERFORM 1 FROM core.ledger_accounts WHERE id=p_account FOR SHARE;
 ready:=documents.statement_readiness(p_account,p_start);
 IF NOT (ready->>'ready')::boolean THEN
  PERFORM documents.audit(p_actor,p_person,'GENERATE','NOT_READY',(SELECT string_agg(x,',') FROM jsonb_array_elements_text(ready->'reasons') x),NULL,p_account);
  RETURN jsonb_build_object('state','NOT_READY','accountId',p_account,'readiness',ready);
 END IF;
 finish:=(p_start+interval '1 month')::date;
 snap:=documents.statement_snapshot(p_account,p_start,finish);
 basis:=jsonb_build_object('accountId',p_account,'periodStart',p_start,'periodEnd',finish,'currency','TST','openingMinor',snap->'openingMinor','closingMinor',snap->'closingMinor','items',snap->'items','linkedCorrections',snap->'linkedCorrections');
 sdigest:=documents.record_digest(basis);
 SELECT * INTO prior FROM documents.statement_versions WHERE account_id=p_account AND period_start=p_start ORDER BY version DESC LIMIT 1;
 IF FOUND THEN
  IF EXISTS(SELECT 1 FROM documents.statement_disposals WHERE version_id=prior.id) THEN RAISE EXCEPTION 'Statement period was disposed under retention policy'; END IF;
  IF prior.snapshot_digest=sdigest THEN
   PERFORM documents.audit(p_actor,p_person,'GENERATE','REPLAYED','SAME_SNAPSHOT',prior.id,p_account);
   RETURN jsonb_build_object('state','READY','created',false,'id',prior.id,'accountId',p_account,'periodStart',p_start,'version',prior.version,'reason',prior.reason,'contentDigest',prior.content_digest);
  END IF;
  SELECT document INTO STRICT prior_doc FROM documents.statement_objects WHERE version_id=prior.id;
  reason:=CASE WHEN prior_doc->'items'=snap->'items' AND prior_doc->'openingMinor'=snap->'openingMinor' THEN 'LINKED_CORRECTION' ELSE 'SOURCE_CHANGED' END;
  next_version:=prior.version+1;
 ELSE reason:='INITIAL';next_version:=1;
 END IF;
 SELECT * INTO STRICT policy FROM documents.retention_policies WHERE record_class='STATEMENT' AND effective_from<=documents.records_today() ORDER BY version DESC LIMIT 1;
 owners:=documents.account_owners(p_account);
 IF cardinality(owners)=0 THEN RAISE EXCEPTION 'Current account owner required'; END IF;
 stamp:=clock_timestamp();
 doc:=jsonb_build_object('formatVersion',1,'source','DOCUMENTS_STATEMENT_V1','label','SYNTHETIC_TST_NOT_A_TAX_FORM','id',target,'accountId',p_account,
  'periodStart',p_start,'periodEnd',finish,'version',next_version,'supersedes',prior.id,'reason',reason,'currency','TST','scale',2,
  'openingMinor',snap->'openingMinor','closingMinor',snap->'closingMinor','totals',snap->'totals','items',snap->'items','linkedCorrections',snap->'linkedCorrections',
  'snapshotDigest',sdigest,'readiness',ready,
  'retention',jsonb_build_object('policyId',policy.id,'policyVersion',policy.version,'retainUntil',finish+policy.retain_days),'generatedAt',documents.utc_text(stamp));
 cdigest:=documents.record_digest(doc);
 INSERT INTO documents.statement_versions VALUES(target,p_account,owners,p_start,finish,next_version,prior.id,reason,sdigest,cdigest,octet_length(doc::text),policy.id,finish+policy.retain_days,p_actor,stamp);
 INSERT INTO documents.statement_objects VALUES(target,doc,cdigest);
 PERFORM documents.audit(p_actor,p_person,'GENERATE','CREATED',reason,target,p_account);
 RETURN jsonb_build_object('state','READY','created',true,'id',target,'accountId',p_account,'periodStart',p_start,'version',next_version,'supersedes',prior.id,'reason',reason,'contentDigest',cdigest);
END $$;
CREATE FUNCTION documents.generate_statement(p_actor jsonb,p_person uuid,p_account uuid,p_start date,p_staff boolean) RETURNS jsonb
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,documents AS $$ BEGIN
 PERFORM documents.statement_actor(p_actor,p_person,p_staff);
 IF NOT p_staff AND NOT documents.is_owner(p_account,p_person) THEN RAISE EXCEPTION 'Account unavailable' USING ERRCODE='P0002'; END IF;
 RETURN documents.generate_statement_internal(p_actor,p_person,p_account,p_start);
END $$;
-- Worker cycle: the most recent closed month for each account, only when ready. Not-ready months are retried later.
CREATE FUNCTION documents.generate_due_statements(p_limit integer) RETURNS integer
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,documents AS $$
DECLARE r record;month date:=(date_trunc('month',documents.records_today())-interval '1 month')::date;created integer:=0;result jsonb;
 actor jsonb:=jsonb_build_object('issuer','urn:piggybank:worker','subject','statement-cycle','trust','SERVICE','clientId','bank-worker');
BEGIN
 IF p_limit NOT BETWEEN 1 AND 500 THEN RAISE EXCEPTION 'Bounded statement cycle required'; END IF;
 FOR r IN SELECT a.id FROM core.account_registry a WHERE (a.created_at AT TIME ZONE 'UTC')::date<(month+interval '1 month')::date
  AND EXISTS(SELECT 1 FROM core.account_relationships x WHERE x.account_id=a.id AND x.kind='OWNER' AND x.revoked_at IS NULL)
  AND NOT EXISTS(SELECT 1 FROM documents.statement_versions v WHERE v.account_id=a.id AND v.period_start=month)
  AND NOT EXISTS(SELECT 1 FROM core.periodic_activations pa LEFT JOIN core.periodic_activation_stops s ON s.activation_id=pa.id WHERE pa.account_id=a.id AND s.id IS NULL
    AND pa.first_period_start<(month+interval '1 month')::date AND coalesce((SELECT max(period_end) FROM core.periodic_finalizations f WHERE f.activation_id=pa.id),'-infinity')<(month+interval '1 month')::date)
  -- Least recently attempted first, so accounts that keep failing cannot starve the others.
  ORDER BY (SELECT max(d.occurred_at) FROM documents.document_access d WHERE d.account_id=a.id AND d.action='GENERATE' AND d.subject='statement-cycle') NULLS FIRST,a.id
  LIMIT p_limit LOOP
  BEGIN
   result:=documents.generate_statement_internal(actor,NULL,r.id,month);
   IF (result->>'created')::boolean THEN created:=created+1; END IF;
  EXCEPTION WHEN OTHERS THEN PERFORM documents.audit(actor,NULL,'GENERATE','NOT_READY',left(SQLERRM,200),NULL,r.id);
  END;
 END LOOP;
 RETURN created;
END $$;

CREATE FUNCTION documents.statement_link(p_actor jsonb,p_person uuid,p_version uuid,p_staff boolean) RETURNS jsonb
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,documents AS $$
DECLARE v documents.statement_versions;token text;link uuid:=gen_random_uuid();stamp timestamptz:=clock_timestamp();
BEGIN
 PERFORM documents.statement_actor(p_actor,p_person,p_staff);
 SELECT * INTO v FROM documents.statement_versions WHERE id=p_version;
 IF NOT FOUND OR (NOT p_staff AND NOT documents.is_owner(v.account_id,p_person)) THEN
  PERFORM documents.audit(p_actor,p_person,'LINK','DENIED','NOT_AUTHORIZED',p_version);
  RETURN jsonb_build_object('outcome','DENIED','reason','NOT_AUTHORIZED');
 END IF;
 IF EXISTS(SELECT 1 FROM documents.statement_disposals WHERE version_id=v.id) THEN
  PERFORM documents.audit(p_actor,p_person,'LINK','DENIED','DISPOSED',v.id,v.account_id);
  RETURN jsonb_build_object('outcome','DENIED','reason','DISPOSED');
 END IF;
 token:=replace(gen_random_uuid()::text,'-','')||replace(gen_random_uuid()::text,'-','');
 INSERT INTO documents.statement_links VALUES(link,v.id,encode(sha256(convert_to(token,'UTF8')),'hex'),p_actor->>'issuer',p_actor->>'subject',p_actor->>'clientId',p_actor->>'trust',p_person,stamp,stamp+interval '2 minutes',NULL);
 PERFORM documents.audit(p_actor,p_person,'LINK','CREATED','SINGLE_USE_TWO_MINUTES',v.id,v.account_id,link);
 RETURN jsonb_build_object('outcome','GRANTED','linkId',link,'token',token,'expiresAt',documents.utc_text(stamp+interval '2 minutes'));
END $$;
CREATE FUNCTION documents.redeem_statement_link(p_actor jsonb,p_person uuid,p_token text,p_staff boolean) RETURNS jsonb
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,documents AS $$
DECLARE l documents.statement_links;v documents.statement_versions;o documents.statement_objects;denied text;successor uuid;
BEGIN
 PERFORM documents.statement_actor(p_actor,p_person,p_staff);
 IF p_token IS NULL OR p_token!~'^[0-9a-f]{64}$' THEN denied:='UNKNOWN_REFERENCE';
 ELSE
  SELECT * INTO l FROM documents.statement_links WHERE token_digest=encode(sha256(convert_to(p_token,'UTF8')),'hex') FOR UPDATE;
  IF NOT FOUND THEN denied:='UNKNOWN_REFERENCE';
  ELSIF l.issuer<>p_actor->>'issuer' OR l.subject<>p_actor->>'subject' OR l.trust<>p_actor->>'trust' OR l.person_id<>p_person THEN denied:='ACTOR_MISMATCH';
  ELSIF l.used_at IS NOT NULL THEN denied:='ALREADY_USED';
  ELSIF l.expires_at<=clock_timestamp() THEN denied:='EXPIRED';
  ELSE
   SELECT * INTO STRICT v FROM documents.statement_versions WHERE id=l.version_id;
   IF NOT p_staff AND NOT documents.is_owner(v.account_id,p_person) THEN denied:='NOT_AUTHORIZED';
   ELSE
    SELECT * INTO o FROM documents.statement_objects WHERE version_id=v.id;
    IF NOT FOUND THEN denied:='DISPOSED';
    ELSIF o.digest<>v.content_digest OR documents.record_digest(o.document)<>v.content_digest THEN denied:='INTEGRITY_CONFLICT';
    END IF;
   END IF;
  END IF;
 END IF;
 IF denied IS NOT NULL THEN
  PERFORM documents.audit(p_actor,p_person,'RETRIEVE','DENIED',denied,l.version_id,v.account_id,l.id);
  RETURN jsonb_build_object('outcome','DENIED','reason',denied);
 END IF;
 UPDATE documents.statement_links SET used_at=clock_timestamp() WHERE id=l.id;
 SELECT id INTO successor FROM documents.statement_versions WHERE supersedes=v.id;
 PERFORM documents.audit(p_actor,p_person,'RETRIEVE','GRANTED','SINGLE_USE_CONSUMED',v.id,v.account_id,l.id);
 RETURN jsonb_build_object('outcome','GRANTED','document',o.document,'contentDigest',o.digest,'supersededBy',successor);
END $$;

CREATE FUNCTION documents.statement_version_json(v documents.statement_versions,p_staff boolean) RETURNS jsonb
LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
 SELECT jsonb_build_object('id',v.id,'accountId',v.account_id,'periodStart',v.period_start,'periodEnd',v.period_end,'version',v.version,'supersedes',v.supersedes,
  'supersededBy',(SELECT s.id FROM documents.statement_versions s WHERE s.supersedes=v.id),'reason',v.reason,'contentDigest',v.content_digest,'snapshotDigest',v.snapshot_digest,
  'createdAt',documents.utc_text(v.created_at),'retainUntil',v.retain_until,'disposed',d.version_id IS NOT NULL,'disposedAt',documents.utc_text(d.disposed_at),
  'openingMinor',o.document->'openingMinor','closingMinor',o.document->'closingMinor','totals',o.document->'totals')
 ||CASE WHEN p_staff THEN jsonb_build_object('owners',to_jsonb(v.owners),'retentionPolicyId',v.retention_policy_id,'sizeBytes',v.size_bytes,'legalHold',documents.hold_active(v.account_id,v.owners),
  'disposalEvidenceDigest',d.evidence_digest,'disposalReason',d.reason) ELSE '{}'::jsonb END
 FROM (SELECT 1) one LEFT JOIN documents.statement_objects o ON o.version_id=v.id LEFT JOIN documents.statement_disposals d ON d.version_id=v.id $$;
CREATE FUNCTION documents.account_statement_periods(p_account uuid,p_staff boolean) RETURNS jsonb
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,documents AS $$
DECLARE opened date;newest date:=date_trunc('month',documents.records_today())::date;m date;result jsonb:='[]';
BEGIN
 SELECT date_trunc('month',created_at AT TIME ZONE 'UTC')::date INTO STRICT opened FROM core.account_registry WHERE id=p_account;
 m:=newest;
 WHILE m>=opened AND m>newest-interval '24 months' LOOP
  result:=result||jsonb_build_object('periodStart',m,'periodEnd',(m+interval '1 month')::date,'readiness',documents.statement_readiness(p_account,m),
   'versions',coalesce((SELECT jsonb_agg(documents.statement_version_json(v,p_staff) ORDER BY v.version DESC) FROM documents.statement_versions v WHERE v.account_id=p_account AND v.period_start=m),'[]'::jsonb));
  m:=(m-interval '1 month')::date;
 END LOOP;
 RETURN result;
END $$;
CREATE FUNCTION documents.data_request_json(r documents.data_requests,p_staff boolean) RETURNS jsonb
LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
 SELECT jsonb_build_object('id',r.id,'kind',r.kind,'detail',r.detail,'verification',r.verification,'submittedAt',documents.utc_text(r.submitted_at),
  'state',CASE WHEN o.request_id IS NULL THEN 'SUBMITTED' ELSE 'COMPLETED' END,'outcome',o.outcome,'items',o.items,'processedAt',documents.utc_text(o.processed_at),
  'export',x.document,'exportRetainUntil',x.retain_until,'exportDisposed',xd.request_id IS NOT NULL)
 ||CASE WHEN p_staff THEN jsonb_build_object('personId',r.person_id,'outcomeDigest',o.digest,'personLegalHold',documents.person_hold_active(r.person_id)) ELSE '{}'::jsonb END
 FROM (SELECT 1) one LEFT JOIN documents.data_request_outcomes o ON o.request_id=r.id LEFT JOIN documents.data_exports x ON x.request_id=r.id LEFT JOIN documents.data_export_disposals xd ON xd.request_id=r.id $$;
CREATE FUNCTION documents.customer_documents(p_actor jsonb,p_person uuid) RETURNS jsonb
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,documents AS $$ BEGIN
 PERFORM documents.require_periodic_actor(p_actor,p_person,false);
 RETURN jsonb_build_object('asOfDate',documents.records_today(),
  'accounts',coalesce((SELECT jsonb_agg(jsonb_build_object('accountId',r.account_id,'periods',documents.account_statement_periods(r.account_id,false)) ORDER BY r.account_id)
    FROM core.account_relationships r WHERE r.person_id=p_person AND r.kind='OWNER' AND r.revoked_at IS NULL),'[]'::jsonb),
  'dataRequests',coalesce((SELECT jsonb_agg(documents.data_request_json(d,false) ORDER BY d.submitted_at DESC) FROM documents.data_requests d WHERE d.person_id=p_person),'[]'::jsonb),
  'policies',(SELECT jsonb_agg(document ORDER BY record_class,version) FROM documents.retention_policies));
END $$;
CREATE FUNCTION documents.staff_documents(p_actor jsonb,p_person uuid,p_account uuid) RETURNS jsonb
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,documents AS $$
DECLARE owners uuid[];BEGIN
 PERFORM documents.require_records_actor(p_actor,p_person,ARRAY['account_support','records_officer','records_approver']);
 PERFORM 1 FROM core.account_registry WHERE id=p_account;IF NOT FOUND THEN RAISE EXCEPTION 'Account unavailable' USING ERRCODE='P0002'; END IF;
 owners:=documents.account_owners(p_account);
 RETURN jsonb_build_object('accountId',p_account,'owners',to_jsonb(owners),'asOfDate',documents.records_today(),'periods',documents.account_statement_periods(p_account,true),
  'legalHolds',coalesce((SELECT jsonb_agg(jsonb_build_object('id',h.id,'scope',h.scope,'accountId',h.account_id,'personId',h.person_id,'reason',h.reason,'reference',h.reference,'placedBy',h.placed_by,'placedAt',documents.utc_text(h.placed_at),
    'active',r.hold_id IS NULL,'releasedBy',r.released_by,'releasedAt',documents.utc_text(r.released_at),'releaseReason',r.reason) ORDER BY h.placed_at DESC)
    FROM documents.legal_holds h LEFT JOIN documents.legal_hold_releases r ON r.hold_id=h.id WHERE h.account_id=p_account OR h.person_id=ANY(owners)),'[]'::jsonb),
  'access',coalesce((SELECT jsonb_agg(jsonb_build_object('id',a.id,'versionId',a.version_id,'linkId',a.link_id,'trust',a.trust,'personId',a.person_id,'action',a.action,'outcome',a.outcome,'reason',a.reason,'occurredAt',documents.utc_text(a.occurred_at)) ORDER BY a.occurred_at DESC)
    FROM (SELECT * FROM documents.document_access WHERE account_id=p_account OR version_id IN (SELECT id FROM documents.statement_versions WHERE account_id=p_account) ORDER BY occurred_at DESC LIMIT 50) a),'[]'::jsonb),
  'dataRequests',coalesce((SELECT jsonb_agg(documents.data_request_json(d,true) ORDER BY d.submitted_at DESC) FROM documents.data_requests d WHERE d.person_id=ANY(owners)),'[]'::jsonb),
  'policies',(SELECT jsonb_agg(document ORDER BY record_class,version) FROM documents.retention_policies));
END $$;
CREATE FUNCTION documents.staff_data_requests(p_actor jsonb,p_person uuid) RETURNS jsonb
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,documents AS $$ BEGIN
 PERFORM documents.require_records_actor(p_actor,p_person,ARRAY['records_officer','records_approver']);
 RETURN coalesce((SELECT jsonb_agg(documents.data_request_json(d,true) ORDER BY d.submitted_at DESC) FROM (SELECT * FROM documents.data_requests ORDER BY submitted_at DESC LIMIT 100) d),'[]'::jsonb);
END $$;

-- Hold placement/release serialize against disposal, so a hold placed first always prevents a later disposal.
CREATE FUNCTION documents.place_legal_hold(p_actor jsonb,p_person uuid,p_key uuid,p_data jsonb) RETURNS jsonb
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,documents AS $$
DECLARE h documents.legal_holds;target uuid:=gen_random_uuid();account uuid;person uuid;
BEGIN
 PERFORM documents.require_records_actor(p_actor,p_person,ARRAY['records_officer']);
 IF p_key IS NULL OR jsonb_typeof(p_data)<>'object' OR EXISTS(SELECT 1 FROM jsonb_object_keys(p_data) k WHERE k NOT IN ('scope','accountId','personId','reason','reference')) THEN RAISE EXCEPTION 'Invalid legal hold request' USING ERRCODE='22023'; END IF;
 PERFORM pg_advisory_xact_lock(hashtextextended('documents.legal-holds',0));
 SELECT * INTO h FROM documents.legal_holds WHERE issuer=p_actor->>'issuer' AND subject=p_actor->>'subject' AND client_id=p_actor->>'clientId' AND request_key=p_key;
 IF FOUND THEN
  IF h.scope IS DISTINCT FROM p_data->>'scope' OR h.account_id IS DISTINCT FROM (p_data->>'accountId')::uuid OR h.person_id IS DISTINCT FROM (p_data->>'personId')::uuid OR h.reason IS DISTINCT FROM trim(p_data->>'reason') OR h.reference IS DISTINCT FROM trim(p_data->>'reference') THEN RAISE EXCEPTION 'Legal hold request key conflict'; END IF;
  RETURN jsonb_build_object('id',h.id,'created',false);
 END IF;
 IF p_data->>'scope'='ACCOUNT' THEN account:=(p_data->>'accountId')::uuid;
  IF p_data ? 'personId' OR NOT EXISTS(SELECT 1 FROM core.account_registry WHERE id=account) THEN RAISE EXCEPTION 'Existing account required for account hold' USING ERRCODE='P0002'; END IF;
 ELSIF p_data->>'scope'='PERSON' THEN person:=(p_data->>'personId')::uuid;
  IF p_data ? 'accountId' OR NOT EXISTS(SELECT 1 FROM controls.person_links WHERE person_id=person) THEN RAISE EXCEPTION 'Existing person required for person hold' USING ERRCODE='P0002'; END IF;
 ELSE RAISE EXCEPTION 'Supported legal hold scope required' USING ERRCODE='22023';
 END IF;
 INSERT INTO documents.legal_holds VALUES(target,p_actor->>'issuer',p_actor->>'subject',p_actor->>'clientId',p_key,p_data->>'scope',account,person,trim(p_data->>'reason'),trim(p_data->>'reference'),p_person,p_actor,clock_timestamp());
 PERFORM documents.audit(p_actor,p_person,'LEGAL_HOLD','CREATED',p_data->>'scope',NULL,account);
 RETURN jsonb_build_object('id',target,'created',true);
END $$;
CREATE FUNCTION documents.release_legal_hold(p_actor jsonb,p_person uuid,p_hold uuid,p_reason text) RETURNS jsonb
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,documents AS $$
DECLARE h documents.legal_holds;r documents.legal_hold_releases;
BEGIN
 PERFORM documents.require_records_actor(p_actor,p_person,ARRAY['records_approver']);
 PERFORM pg_advisory_xact_lock(hashtextextended('documents.legal-holds',0));
 SELECT * INTO h FROM documents.legal_holds WHERE id=p_hold;
 IF NOT FOUND THEN RAISE EXCEPTION 'Legal hold unavailable' USING ERRCODE='P0002'; END IF;
 IF h.placed_by=p_person THEN RAISE EXCEPTION 'Independent legal hold release required'; END IF;
 SELECT * INTO r FROM documents.legal_hold_releases WHERE hold_id=p_hold;
 IF FOUND THEN
  IF r.released_by<>p_person OR r.reason<>trim(p_reason) THEN RAISE EXCEPTION 'Legal hold already released'; END IF;
  RETURN jsonb_build_object('id',p_hold,'released',false);
 END IF;
 INSERT INTO documents.legal_hold_releases VALUES(p_hold,p_person,p_actor,trim(p_reason),clock_timestamp());
 PERFORM documents.audit(p_actor,p_person,'HOLD_RELEASE','COMPLETED',h.scope,NULL,h.account_id);
 RETURN jsonb_build_object('id',p_hold,'released',true);
END $$;

CREATE FUNCTION documents.dispose_statement_internal(p_actor jsonb,p_person uuid,p_version uuid,p_reason text,p_request uuid) RETURNS jsonb
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,documents AS $$
DECLARE v documents.statement_versions;o documents.statement_objects;today date:=documents.records_today();evidence jsonb;stamp timestamptz:=clock_timestamp();policy documents.retention_policies;
BEGIN
 PERFORM pg_advisory_xact_lock_shared(hashtextextended('documents.legal-holds',0));
 PERFORM pg_advisory_xact_lock(hashtextextended('documents.dispose:'||p_version,0));
 SELECT * INTO v FROM documents.statement_versions WHERE id=p_version;
 IF NOT FOUND THEN RAISE EXCEPTION 'Statement unavailable' USING ERRCODE='P0002'; END IF;
 IF EXISTS(SELECT 1 FROM documents.statement_disposals WHERE version_id=v.id) THEN RETURN jsonb_build_object('versionId',v.id,'status','ALREADY_DISPOSED'); END IF;
 SELECT * INTO STRICT policy FROM documents.retention_policies WHERE id=v.retention_policy_id;
 IF policy.disposal<>'PERMITTED' THEN
  PERFORM documents.audit(p_actor,p_person,'DISPOSE','RESTRICTED','DISPOSAL_NOT_ENABLED',v.id,v.account_id);
  RETURN jsonb_build_object('versionId',v.id,'status','RESTRICTED_POLICY');
 END IF;
 IF v.retain_until>today THEN
  PERFORM documents.audit(p_actor,p_person,'DISPOSE','RESTRICTED','RETENTION_UNTIL_'||v.retain_until,v.id,v.account_id);
  RETURN jsonb_build_object('versionId',v.id,'status','RESTRICTED_RETENTION','retainUntil',v.retain_until);
 END IF;
 IF documents.hold_active(v.account_id,v.owners) THEN
  PERFORM documents.audit(p_actor,p_person,'DISPOSE','RESTRICTED','LEGAL_HOLD',v.id,v.account_id);
  RETURN jsonb_build_object('versionId',v.id,'status','RESTRICTED_LEGAL_HOLD');
 END IF;
 SELECT * INTO STRICT o FROM documents.statement_objects WHERE version_id=v.id;
 evidence:=jsonb_build_object('source','DOCUMENTS_STATEMENT_DISPOSAL_V1','versionId',v.id,'accountId',v.account_id,'periodStart',v.period_start,'version',v.version,
  'contentDigest',v.content_digest,'objectDigest',o.digest,'sizeBytes',v.size_bytes,'policyId',policy.id,'policyVersion',policy.version,'retainUntil',v.retain_until,
  'businessDate',today,'holdCheck','NO_ACTIVE_HOLD','reason',trim(p_reason),'dataRequestId',p_request,'disposedBy',p_person,'disposedAt',documents.utc_text(stamp));
 INSERT INTO documents.statement_disposals VALUES(v.id,p_person,p_actor,trim(p_reason),p_request,today,evidence,documents.record_digest(evidence),stamp);
 DELETE FROM documents.statement_objects WHERE version_id=v.id;
 PERFORM documents.audit(p_actor,p_person,'DISPOSE','COMPLETED',coalesce('DATA_REQUEST_'||p_request,'RETENTION_ENDED'),v.id,v.account_id);
 RETURN jsonb_build_object('versionId',v.id,'status','DISPOSED','evidenceDigest',documents.record_digest(evidence));
END $$;
CREATE FUNCTION documents.dispose_statement(p_actor jsonb,p_person uuid,p_version uuid,p_reason text) RETURNS jsonb
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,documents AS $$ BEGIN
 PERFORM documents.require_records_actor(p_actor,p_person,ARRAY['records_officer']);
 IF length(trim(coalesce(p_reason,''))) NOT BETWEEN 3 AND 500 THEN RAISE EXCEPTION 'Disposal reason required' USING ERRCODE='22023'; END IF;
 RETURN documents.dispose_statement_internal(p_actor,p_person,p_version,p_reason,NULL);
END $$;
CREATE FUNCTION documents.dispose_data_export(p_actor jsonb,p_person uuid,p_request uuid,p_reason text) RETURNS jsonb
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,documents AS $$
DECLARE x documents.data_exports;today date:=documents.records_today();evidence jsonb;stamp timestamptz:=clock_timestamp();
BEGIN
 PERFORM documents.require_records_actor(p_actor,p_person,ARRAY['records_officer']);
 IF length(trim(coalesce(p_reason,''))) NOT BETWEEN 3 AND 500 THEN RAISE EXCEPTION 'Disposal reason required' USING ERRCODE='22023'; END IF;
 PERFORM pg_advisory_xact_lock_shared(hashtextextended('documents.legal-holds',0));
 PERFORM pg_advisory_xact_lock(hashtextextended('documents.export:'||p_request,0));
 IF EXISTS(SELECT 1 FROM documents.data_export_disposals WHERE request_id=p_request) THEN RETURN jsonb_build_object('requestId',p_request,'status','ALREADY_DISPOSED'); END IF;
 SELECT * INTO x FROM documents.data_exports WHERE request_id=p_request;
 IF NOT FOUND THEN RAISE EXCEPTION 'Export unavailable' USING ERRCODE='P0002'; END IF;
 IF x.retain_until>today THEN
  PERFORM documents.audit(p_actor,p_person,'EXPORT_DISPOSE','RESTRICTED','RETENTION_UNTIL_'||x.retain_until);
  RETURN jsonb_build_object('requestId',p_request,'status','RESTRICTED_RETENTION','retainUntil',x.retain_until);
 END IF;
 IF documents.person_hold_active(x.person_id) THEN
  PERFORM documents.audit(p_actor,p_person,'EXPORT_DISPOSE','RESTRICTED','LEGAL_HOLD');
  RETURN jsonb_build_object('requestId',p_request,'status','RESTRICTED_LEGAL_HOLD');
 END IF;
 evidence:=jsonb_build_object('source','DOCUMENTS_EXPORT_DISPOSAL_V1','requestId',p_request,'exportDigest',x.digest,'policyId',x.retention_policy_id,'retainUntil',x.retain_until,'businessDate',today,'holdCheck','NO_ACTIVE_HOLD','reason',trim(p_reason),'disposedBy',p_person,'disposedAt',documents.utc_text(stamp));
 INSERT INTO documents.data_export_disposals VALUES(p_request,p_person,p_actor,trim(p_reason),today,evidence,documents.record_digest(evidence),stamp);
 DELETE FROM documents.data_exports WHERE request_id=p_request;
 PERFORM documents.audit(p_actor,p_person,'EXPORT_DISPOSE','COMPLETED','RETENTION_ENDED');
 RETURN jsonb_build_object('requestId',p_request,'status','DISPOSED','evidenceDigest',documents.record_digest(evidence));
END $$;

CREATE FUNCTION documents.submit_data_request(p_actor jsonb,p_person uuid,p_key uuid,p_kind text,p_detail text) RETURNS jsonb
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,documents AS $$
DECLARE r documents.data_requests;target uuid:=gen_random_uuid();BEGIN
 PERFORM documents.require_periodic_actor(p_actor,p_person,false);
 IF p_key IS NULL OR p_kind NOT IN ('ACCESS','CORRECTION','DELETION') OR length(coalesce(p_detail,''))>500 THEN RAISE EXCEPTION 'Invalid data request' USING ERRCODE='22023'; END IF;
 SELECT * INTO r FROM documents.data_requests WHERE issuer=p_actor->>'issuer' AND subject=p_actor->>'subject' AND client_id=p_actor->>'clientId' AND request_key=p_key;
 IF FOUND THEN
  IF r.kind<>p_kind OR r.detail<>coalesce(p_detail,'') THEN RAISE EXCEPTION 'Data request key conflict'; END IF;
  RETURN documents.data_request_json(r,false);
 END IF;
 INSERT INTO documents.data_requests VALUES(target,p_actor->>'issuer',p_actor->>'subject',p_actor->>'clientId',p_key,p_person,p_kind,coalesce(p_detail,''),'AUTHENTICATED_CUSTOMER_SESSION',p_actor,clock_timestamp()) RETURNING * INTO r;
 PERFORM documents.audit(p_actor,p_person,'DATA_REQUEST','CREATED',p_kind);
 RETURN documents.data_request_json(r,false);
END $$;
-- Each statement version receives a traceable allowed/restricted result under the assigned policy and current holds.
CREATE FUNCTION documents.process_data_request(p_actor jsonb,p_person uuid,p_request uuid) RETURNS jsonb
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,documents AS $$
DECLARE r documents.data_requests;v documents.statement_versions;items jsonb:='[]';item jsonb;status text;allowed integer:=0;restricted integer:=0;outcome text;
 export jsonb;policy documents.retention_policies;stamp timestamptz:=clock_timestamp();
BEGIN
 PERFORM documents.require_records_actor(p_actor,p_person,ARRAY['records_officer']);
 PERFORM pg_advisory_xact_lock(hashtextextended('documents.data-request:'||p_request,0));
 SELECT * INTO r FROM documents.data_requests WHERE id=p_request;
 IF NOT FOUND THEN RAISE EXCEPTION 'Data request unavailable' USING ERRCODE='P0002'; END IF;
 IF r.person_id=p_person THEN RAISE EXCEPTION 'Independent processing required'; END IF;
 IF EXISTS(SELECT 1 FROM documents.data_request_outcomes WHERE request_id=r.id) THEN RETURN documents.data_request_json(r,true); END IF;
 FOR v IN SELECT * FROM documents.statement_versions s WHERE r.person_id=ANY(s.owners) OR documents.is_owner(s.account_id,r.person_id) ORDER BY s.account_id,s.period_start,s.version LOOP
  IF r.kind='ACCESS' THEN
   status:=CASE WHEN EXISTS(SELECT 1 FROM documents.statement_disposals WHERE version_id=v.id) THEN 'ALREADY_DISPOSED' ELSE 'INCLUDED' END;
   item:=jsonb_build_object('status',status);
  ELSIF r.kind='DELETION' THEN
   item:=documents.dispose_statement_internal(p_actor,p_person,v.id,'Verified customer deletion request',r.id);status:=item->>'status';
  ELSE
   status:='RESTRICTED_FINANCIAL_RECORD';
   item:=jsonb_build_object('status',status,'note','Statements reproduce immutable Core journal facts. Errors are corrected by linked journals and a superseding statement version.');
  END IF;
  IF status IN ('INCLUDED','DISPOSED','ALREADY_DISPOSED') THEN allowed:=allowed+1; ELSE restricted:=restricted+1; END IF;
  items:=items||(jsonb_build_object('recordClass','STATEMENT','versionId',v.id,'accountId',v.account_id,'periodStart',v.period_start,'version',v.version)||item);
 END LOOP;
 IF r.kind<>'ACCESS' THEN
  items:=items||jsonb_build_object('recordClass','CORE_JOURNAL','status','RESTRICTED_RETENTION','note','Native financial journals are immutable Core records outside document disposal.');
  restricted:=restricted+1;
 END IF;
 outcome:=CASE WHEN restricted=0 THEN 'ALLOWED' WHEN allowed=0 THEN 'RESTRICTED' ELSE 'PARTIALLY_RESTRICTED' END;
 INSERT INTO documents.data_request_outcomes VALUES(r.id,p_person,p_actor,outcome,items,documents.record_digest(jsonb_build_object('requestId',r.id,'outcome',outcome,'items',items)),stamp);
 IF r.kind='ACCESS' THEN
  SELECT * INTO STRICT policy FROM documents.retention_policies WHERE record_class='DATA_EXPORT' AND effective_from<=documents.records_today() ORDER BY version DESC LIMIT 1;
  export:=jsonb_build_object('formatVersion',1,'source','DOCUMENTS_DATA_EXPORT_V1','label','SYNTHETIC_TST','requestId',r.id,'generatedAt',documents.utc_text(stamp),
   'statements',coalesce((SELECT jsonb_agg(x-'status' ORDER BY x->>'accountId',x->>'periodStart',(x->>'version')::int) FROM jsonb_array_elements(items) x WHERE x->>'status'='INCLUDED'),'[]'::jsonb),
   'retrieval','Each listed statement is retrieved separately through an authenticated single-use link.');
  INSERT INTO documents.data_exports VALUES(r.id,r.person_id,export,documents.record_digest(export),policy.id,documents.records_today()+policy.retain_days,stamp);
 END IF;
 PERFORM documents.audit(p_actor,p_person,'DATA_REQUEST_PROCESSED','COMPLETED',r.kind||'_'||outcome);
 RETURN documents.data_request_json(r,true);
END $$;

GRANT EXECUTE ON FUNCTION documents.generate_statement(jsonb,uuid,uuid,date,boolean),documents.generate_due_statements(integer),
 documents.statement_link(jsonb,uuid,uuid,boolean),documents.redeem_statement_link(jsonb,uuid,text,boolean),
 documents.customer_documents(jsonb,uuid),documents.staff_documents(jsonb,uuid,uuid),documents.staff_data_requests(jsonb,uuid),
 documents.place_legal_hold(jsonb,uuid,uuid,jsonb),documents.release_legal_hold(jsonb,uuid,uuid,text),
 documents.dispose_statement(jsonb,uuid,uuid,text),documents.dispose_data_export(jsonb,uuid,uuid,text),
 documents.submit_data_request(jsonb,uuid,uuid,text,text),documents.process_data_request(jsonb,uuid,uuid) TO bank_documents;
