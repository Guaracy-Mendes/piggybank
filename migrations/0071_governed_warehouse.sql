-- owners: data (analytics schema: governed warehouse adapter registry, purposes, datasets and versioned schemas, ingestion
-- runs, immutable facts, transforms, snapshots, reproductions and denials). WP-37: one local warehouse adapter (user
-- decision 2026-11-18) as the template for later analytical storage/processing adapters; no external product is selected
-- (DEC-05/06/08). Analytics reads Data projections and Core's immutable journal through definer functions only. It never
-- calls a Core function and never writes a Core table; bank_data receives EXECUTE on the analytics API and nothing else.

CREATE SCHEMA analytics AUTHORIZATION bank_migrator;
GRANT USAGE ON SCHEMA analytics TO bank_data;
CREATE INDEX IF NOT EXISTS transaction_views_recorded ON data.transaction_views(recorded_at);
CREATE INDEX IF NOT EXISTS transaction_views_business_date ON data.transaction_views((document->>'businessDate'));
CREATE INDEX IF NOT EXISTS account_views_recorded ON data.account_views(recorded_at);
CREATE INDEX IF NOT EXISTS journal_batches_business_date ON core.journal_batches(business_date);

-- ===== Registries (immutable fixtures) =====
CREATE TABLE analytics.adapters(code text PRIMARY KEY,kind text NOT NULL CHECK(kind='STORAGE_AND_PROCESSING'),
 status text NOT NULL CHECK(status IN ('LOCAL_TEMPLATE','PENDING_SELECTION')),description text NOT NULL);
INSERT INTO analytics.adapters VALUES('LOCAL_WAREHOUSE','STORAGE_AND_PROCESSING','LOCAL_TEMPLATE',
 'Local PostgreSQL analytics schema with deterministic SQL transforms. Synthetic TST data only; not a selected product.');
CREATE TABLE analytics.purposes(code text PRIMARY KEY,mandatory boolean NOT NULL,description text NOT NULL);
INSERT INTO analytics.purposes VALUES
 ('FINANCIAL_REPORTING',true,'Mandatory financial and control reporting. Every fact is included.'),
 ('OPERATIONS_ANALYTICS',false,'Optional operational analysis. Parties with an ANALYTICS processing restriction are excluded.'),
 ('MARKETING',false,'Optional marketing use. No warehouse dataset permits it (audiences are WP-38).');
CREATE TABLE analytics.purpose_roles(purpose text NOT NULL REFERENCES analytics.purposes,role text NOT NULL,PRIMARY KEY(purpose,role));
INSERT INTO analytics.purpose_roles VALUES
 ('FINANCIAL_REPORTING','data_steward'),('FINANCIAL_REPORTING','finance_adjuster'),('FINANCIAL_REPORTING','finance_approver'),
 ('OPERATIONS_ANALYTICS','data_steward'),('OPERATIONS_ANALYTICS','data_analyst'),
 ('MARKETING','data_steward'),('MARKETING','data_analyst');
CREATE TABLE analytics.datasets(code text PRIMARY KEY,source text NOT NULL,economic_key text[] NOT NULL,economic_fields text[] NOT NULL,
 purposes text[] NOT NULL,stale_after interval NOT NULL,description text NOT NULL);
INSERT INTO analytics.datasets VALUES
 ('ledger_entries','DATA_TRANSACTION_VIEWS',ARRAY['operationId','postingKind','accountId'],ARRAY['amountMinor','currency','businessDate','originalBatchId'],
  ARRAY['FINANCIAL_REPORTING','OPERATIONS_ANALYTICS'],interval '15 minutes','Posted journal entries projected from Core journal.batch-posted events.'),
 ('account_states','DATA_ACCOUNT_VIEWS',ARRAY['accountId','sourceVersion'],ARRAY['partyId','productVersionId','state'],
  ARRAY['FINANCIAL_REPORTING','OPERATIONS_ANALYTICS'],interval '15 minutes','Observed account versions (party, product, state) from Core account.changed events.');
-- Fields each source can supply. A schema version may record any subset that keeps backward compatibility.
CREATE TABLE analytics.source_fields(dataset text NOT NULL REFERENCES analytics.datasets,name text NOT NULL,
 type text NOT NULL CHECK(type IN ('uuid','text','bigint','date','timestamp','boolean')),PRIMARY KEY(dataset,name));
INSERT INTO analytics.source_fields VALUES
 ('ledger_entries','batchId','uuid'),('ledger_entries','operationId','uuid'),('ledger_entries','postingKind','text'),
 ('ledger_entries','accountId','uuid'),('ledger_entries','accountKind','text'),('ledger_entries','amountMinor','bigint'),
 ('ledger_entries','currency','text'),('ledger_entries','businessDate','date'),('ledger_entries','originalBatchId','uuid'),
 ('ledger_entries','productVersionId','uuid'),('ledger_entries','projectedAt','timestamp'),
 ('account_states','accountId','uuid'),('account_states','partyId','uuid'),('account_states','productVersionId','uuid'),
 ('account_states','state','text'),('account_states','sourceVersion','bigint'),('account_states','sourceAsOf','timestamp'),
 ('account_states','debitBlocked','boolean'),('account_states','creditBlocked','boolean');
CREATE TABLE analytics.dataset_schemas(dataset text NOT NULL REFERENCES analytics.datasets,version integer NOT NULL CHECK(version>0),
 fields jsonb NOT NULL CHECK(jsonb_typeof(fields)='array'),compatibility text NOT NULL CHECK(compatibility IN ('INITIAL','BACKWARD')),
 digest text NOT NULL,registered_by jsonb,registered_person uuid,registered_at timestamptz NOT NULL DEFAULT clock_timestamp(),PRIMARY KEY(dataset,version));
INSERT INTO analytics.dataset_schemas(dataset,version,fields,compatibility,digest)
SELECT d,1,f,'INITIAL',encode(sha256(convert_to(f::text,'UTF8')),'hex') FROM (VALUES
 ('ledger_entries','[{"name":"batchId","type":"uuid","required":true},{"name":"operationId","type":"uuid","required":true},
  {"name":"postingKind","type":"text","required":true},{"name":"accountId","type":"uuid","required":true},
  {"name":"accountKind","type":"text","required":true},{"name":"amountMinor","type":"bigint","required":true},
  {"name":"currency","type":"text","required":true},{"name":"businessDate","type":"date","required":true},
  {"name":"originalBatchId","type":"uuid","required":false}]'::jsonb),
 ('account_states','[{"name":"accountId","type":"uuid","required":true},{"name":"partyId","type":"uuid","required":true},
  {"name":"productVersionId","type":"uuid","required":true},{"name":"state","type":"text","required":true},
  {"name":"sourceVersion","type":"bigint","required":true},{"name":"sourceAsOf","type":"timestamp","required":true}]'::jsonb)) s(d,f);

-- ===== Ingestion (append-only facts keyed by economic identity) =====
CREATE TABLE analytics.ingestion_runs(id uuid PRIMARY KEY,dataset text NOT NULL,schema_version integer NOT NULL,
 mode text NOT NULL CHECK(mode IN ('INCREMENTAL','BACKFILL')),window_from timestamptz NOT NULL,window_to timestamptz NOT NULL,
 read_count integer NOT NULL,inserted integer NOT NULL,duplicates integer NOT NULL,conflicts integer NOT NULL,rejected integer NOT NULL,
 source_high_water timestamptz,requested_by jsonb,requested_person uuid,started_at timestamptz NOT NULL,finished_at timestamptz NOT NULL,
 FOREIGN KEY(dataset,schema_version) REFERENCES analytics.dataset_schemas,CHECK(window_from<=window_to));
CREATE TABLE analytics.watermarks(dataset text PRIMARY KEY REFERENCES analytics.datasets,high_water timestamptz NOT NULL,
 last_run uuid REFERENCES analytics.ingestion_runs,last_run_at timestamptz,checked_at timestamptz NOT NULL);
CREATE TABLE analytics.facts(seq bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,dataset text NOT NULL,schema_version integer NOT NULL,
 economic_key text NOT NULL,economic_digest text NOT NULL,business_date date,account_id uuid NOT NULL,record jsonb NOT NULL,
 source_ref text NOT NULL,source_recorded_at timestamptz NOT NULL,
 run_id uuid NOT NULL REFERENCES analytics.ingestion_runs DEFERRABLE INITIALLY DEFERRED,ingested_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 UNIQUE(dataset,economic_key),FOREIGN KEY(dataset,schema_version) REFERENCES analytics.dataset_schemas);
CREATE INDEX facts_business ON analytics.facts(dataset,business_date);
CREATE INDEX facts_account ON analytics.facts(dataset,account_id);
-- Same economic identity, different economic content: never overwritten; quarantined for review.
CREATE TABLE analytics.ingestion_conflicts(id uuid PRIMARY KEY,run_id uuid NOT NULL REFERENCES analytics.ingestion_runs DEFERRABLE INITIALLY DEFERRED,
 dataset text NOT NULL,economic_key text NOT NULL,business_date date,existing_seq bigint NOT NULL REFERENCES analytics.facts,
 existing_digest text NOT NULL,incoming_digest text NOT NULL,incoming_record jsonb NOT NULL,source_ref text NOT NULL,
 detected_at timestamptz NOT NULL DEFAULT clock_timestamp(),UNIQUE(dataset,economic_key,incoming_digest));
CREATE TABLE analytics.ingestion_rejects(id uuid PRIMARY KEY,run_id uuid NOT NULL REFERENCES analytics.ingestion_runs DEFERRABLE INITIALLY DEFERRED,
 dataset text NOT NULL,source_ref text NOT NULL,reason text NOT NULL,detected_at timestamptz NOT NULL DEFAULT clock_timestamp(),UNIQUE(dataset,source_ref,reason));

-- ===== Transforms and snapshots =====
CREATE TABLE analytics.transforms(code text NOT NULL,version integer NOT NULL CHECK(version>0),inputs jsonb NOT NULL,dimensions text[] NOT NULL,
 definition text NOT NULL,digest text NOT NULL,PRIMARY KEY(code,version));
INSERT INTO analytics.transforms SELECT c,v,i::jsonb,d,t,encode(sha256(convert_to(c||':'||v||':'||t,'UTF8')),'hex') FROM (VALUES
 ('daily_ledger_movement',1,'[{"dataset":"ledger_entries","minSchema":1},{"dataset":"account_states","minSchema":1}]',ARRAY['businessDate','postingKind','partyRef'],
  'Per business date, posting kind and canonical party (master reference as of the snapshot cut): entry count, credits and debits in minor units. '
  'Internal ledger accounts group as INTERNAL:<kind>; customer accounts without an observed account state group as UNRESOLVED.'),
 ('daily_ledger_movement',2,'[{"dataset":"ledger_entries","minSchema":1},{"dataset":"account_states","minSchema":1}]',ARRAY['businessDate','postingKind','partyRef','productVersionId'],
  'Version 1 plus product version: the ledger_entries schema 2 field when recorded, else the latest observed account state.')) s(c,v,i,d,t);
CREATE TABLE analytics.snapshots(id uuid PRIMARY KEY,transform text NOT NULL,transform_version integer NOT NULL,transform_digest text NOT NULL,
 adapter text NOT NULL REFERENCES analytics.adapters,purpose text NOT NULL REFERENCES analytics.purposes,business_from date NOT NULL,business_to date NOT NULL,
 fact_cut bigint NOT NULL,master_cut timestamptz NOT NULL,inputs jsonb NOT NULL,source_watermarks jsonb NOT NULL,completeness jsonb NOT NULL,
 status text NOT NULL CHECK(status IN ('COMPLETE','INCOMPLETE')),control jsonb NOT NULL,result_digest text NOT NULL,
 supersedes uuid REFERENCES analytics.snapshots,explanation jsonb,requested_by jsonb NOT NULL,requested_person uuid NOT NULL,
 created_at timestamptz NOT NULL DEFAULT clock_timestamp(),FOREIGN KEY(transform,transform_version) REFERENCES analytics.transforms,
 CHECK(business_from<=business_to AND business_to-business_from<=92));
CREATE INDEX snapshots_series ON analytics.snapshots(transform,purpose,business_from,business_to,created_at);
CREATE TABLE analytics.snapshot_rows(snapshot_id uuid NOT NULL REFERENCES analytics.snapshots DEFERRABLE INITIALLY DEFERRED,row_no integer NOT NULL,dimensions jsonb NOT NULL,
 entries bigint NOT NULL,credit_minor numeric NOT NULL,debit_minor numeric NOT NULL,PRIMARY KEY(snapshot_id,row_no));
CREATE TABLE analytics.access_denials(id uuid PRIMARY KEY,purpose text NOT NULL,transform text,transform_version integer,reason text NOT NULL,
 requested_by jsonb NOT NULL,requested_person uuid,denied_at timestamptz NOT NULL DEFAULT clock_timestamp());
CREATE TABLE analytics.reproductions(id uuid PRIMARY KEY,snapshot_id uuid NOT NULL REFERENCES analytics.snapshots,reproduced boolean NOT NULL,
 recomputed_digest text NOT NULL,requested_by jsonb NOT NULL,requested_person uuid NOT NULL,reproduced_at timestamptz NOT NULL DEFAULT clock_timestamp());
DO $$ DECLARE t text;BEGIN
 FOREACH t IN ARRAY ARRAY['adapters','purposes','purpose_roles','datasets','source_fields','dataset_schemas','ingestion_runs','facts','ingestion_conflicts',
  'ingestion_rejects','transforms','snapshots','snapshot_rows','access_denials','reproductions'] LOOP
  EXECUTE format('CREATE TRIGGER immutable_%s BEFORE UPDATE OR DELETE OR TRUNCATE ON analytics.%I FOR EACH STATEMENT EXECUTE FUNCTION data.immutable_history()',t,t);
 END LOOP;
END $$;

-- ===== Helpers =====
CREATE FUNCTION analytics.utc(p_at timestamptz) RETURNS text LANGUAGE sql IMMUTABLE SET search_path=pg_catalog AS
$$ SELECT to_char(p_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"') $$;
CREATE FUNCTION analytics.require(p_actor jsonb,p_person uuid,p_roles text[]) RETURNS void LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog AS $$
BEGIN PERFORM controls.require_staff(p_actor,p_person,p_roles);
EXCEPTION WHEN raise_exception THEN RAISE EXCEPTION 'Analytics access denied: %',SQLERRM USING ERRCODE='42501'; END $$;
CREATE FUNCTION analytics.audit(p_actor jsonb,p_person uuid,p_action text,p_resource text,p_outcome text,p_reason text) RETURNS void
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog AS $$ BEGIN
 PERFORM controls.append_audit(gen_random_uuid(),'analytics',jsonb_build_object('actorIssuer',coalesce(p_actor->>'issuer','urn:piggybank:worker'),
  'actorSubject',coalesce(p_actor->>'subject','analytics-ingestion'),'actorPersonId',p_person,'representedParty',NULL,'action',p_action,'resource',p_resource,
  'outcome',p_outcome,'policyVersion','analytics-v1','reasonCode',p_reason,'correlationId',gen_random_uuid(),'occurredAt',analytics.utc(clock_timestamp())));
END $$;
CREATE FUNCTION analytics.valid_value(p_type text,p_value jsonb) RETURNS boolean LANGUAGE sql IMMUTABLE SET search_path=pg_catalog AS $$
 SELECT CASE p_type
  WHEN 'uuid' THEN jsonb_typeof(p_value)='string' AND p_value#>>'{}' ~ '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$'
  WHEN 'bigint' THEN jsonb_typeof(p_value)='string' AND p_value#>>'{}' ~ '^-?[0-9]{1,18}$'
  WHEN 'date' THEN jsonb_typeof(p_value)='string' AND p_value#>>'{}' ~ '^[0-9]{4}-[0-9]{2}-[0-9]{2}$'
  WHEN 'timestamp' THEN jsonb_typeof(p_value)='string' AND p_value#>>'{}' ~ '^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9:.]+Z$'
  WHEN 'boolean' THEN jsonb_typeof(p_value)='boolean'
  WHEN 'text' THEN jsonb_typeof(p_value)='string' AND length(p_value#>>'{}') BETWEEN 1 AND 200
  ELSE false END $$;
-- Canonical party as of a cut: the latest approved master change recorded at or before it, else the source reference.
CREATE FUNCTION analytics.canonical_as_of(p_ref uuid,p_at timestamptz) RETURNS uuid LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS
$$ SELECT coalesce((SELECT h.new_canonical FROM data.master_history h WHERE h.reference_id=p_ref AND h.created_at<=p_at ORDER BY h.created_at DESC,h.id DESC LIMIT 1),p_ref) $$;
-- WP-28 privacy restriction (ANALYTICS purpose) applied to the party's person at or before the cut.
CREATE FUNCTION analytics.party_restricted(p_party uuid,p_at timestamptz) RETURNS boolean LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS
$$ SELECT EXISTS(SELECT 1 FROM core.parties p JOIN controls.person_links l ON l.issuer=p.issuer AND l.subject=p.subject
  JOIN data.processing_restrictions r ON r.person_id=l.person_id WHERE p.id=p_party AND r.purpose='ANALYTICS' AND r.applied_at<=p_at) $$;

-- ===== Schema registration with backward-compatibility check =====
CREATE FUNCTION analytics.register_schema(p_actor jsonb,p_person uuid,p_dataset text,p_version integer,p_fields jsonb) RETURNS jsonb
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,analytics AS $$
DECLARE latest analytics.dataset_schemas;d analytics.datasets;f jsonb;old jsonb;names text[]:='{}';n text;catalog text;BEGIN
 PERFORM analytics.require(p_actor,p_person,ARRAY['data_steward']);
 SELECT * INTO d FROM analytics.datasets WHERE code=p_dataset;IF NOT FOUND THEN RAISE EXCEPTION 'Dataset unavailable' USING ERRCODE='P0002'; END IF;
 PERFORM pg_advisory_xact_lock(hashtext('analytics.warehouse'));
 SELECT * INTO latest FROM analytics.dataset_schemas WHERE dataset=p_dataset ORDER BY version DESC LIMIT 1;
 IF p_version IS DISTINCT FROM latest.version+1 THEN RAISE EXCEPTION 'INCOMPATIBLE_SCHEMA: version must be %',latest.version+1; END IF;
 IF jsonb_typeof(p_fields)<>'array' OR jsonb_array_length(p_fields) NOT BETWEEN 1 AND 40 THEN RAISE EXCEPTION 'Schema fields required' USING ERRCODE='22023'; END IF;
 FOR f IN SELECT * FROM jsonb_array_elements(p_fields) LOOP
  IF jsonb_typeof(f)<>'object' OR NOT (f ?& ARRAY['name','type','required']) OR (SELECT count(*) FROM jsonb_object_keys(f))<>3
   OR jsonb_typeof(f->'required')<>'boolean' THEN RAISE EXCEPTION 'Invalid field declaration' USING ERRCODE='22023'; END IF;
  n:=f->>'name';
  IF n=ANY(names) THEN RAISE EXCEPTION 'INCOMPATIBLE_SCHEMA: duplicate field %',n; END IF;
  names:=names||n;
  SELECT type INTO catalog FROM analytics.source_fields WHERE dataset=p_dataset AND name=n;
  IF catalog IS NULL THEN RAISE EXCEPTION 'INCOMPATIBLE_SCHEMA: field % is not supplied by source %',n,d.source; END IF;
  IF catalog<>f->>'type' THEN RAISE EXCEPTION 'INCOMPATIBLE_SCHEMA: field % has source type %',n,catalog; END IF;
  SELECT x INTO old FROM jsonb_array_elements(latest.fields) x WHERE x->>'name'=n;
  IF old IS NULL AND (f->>'required')::boolean THEN RAISE EXCEPTION 'INCOMPATIBLE_SCHEMA: new field % must be optional',n; END IF;
  IF old IS NOT NULL AND (old->>'required')<>(f->>'required') THEN RAISE EXCEPTION 'INCOMPATIBLE_SCHEMA: field % changes requiredness',n; END IF;
  old:=NULL;
 END LOOP;
 FOR old IN SELECT * FROM jsonb_array_elements(latest.fields) LOOP
  IF NOT (old->>'name')=ANY(names) THEN RAISE EXCEPTION 'INCOMPATIBLE_SCHEMA: field % removed',old->>'name'; END IF;
 END LOOP;
 INSERT INTO analytics.dataset_schemas(dataset,version,fields,compatibility,digest,registered_by,registered_person)
 VALUES(p_dataset,p_version,p_fields,'BACKWARD',encode(sha256(convert_to(p_fields::text,'UTF8')),'hex'),p_actor,p_person);
 PERFORM analytics.audit(p_actor,p_person,'analytics.schema-registered',p_dataset||':v'||p_version,'RECORDED','BACKWARD_COMPATIBLE');
 RETURN jsonb_build_object('dataset',p_dataset,'version',p_version,'compatibility','BACKWARD','previousVersion',latest.version);
END $$;

-- ===== Ingestion =====
-- Candidate rows carry every source field; the active schema version decides which are recorded.
CREATE FUNCTION analytics.candidates(p_dataset text,p_from timestamptz,p_to timestamptz,p_limit integer)
RETURNS TABLE(source_ref text,account_id uuid,recorded_at timestamptz,candidate jsonb) LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
 SELECT 'transaction_views:'||t.batch_id||':'||t.account_id,t.account_id,t.recorded_at,
  jsonb_strip_nulls(jsonb_build_object('batchId',t.batch_id,'operationId',t.operation_id,'postingKind',t.document->>'postingKind','accountId',t.account_id,
   'accountKind',a.kind,'amountMinor',t.document->'amount'->>'minorUnits','currency',t.document->'amount'->'currency'->>'code',
   'businessDate',t.document->>'businessDate','originalBatchId',t.document->>'originalBatchId','productVersionId',v.product_version_id,
   'projectedAt',analytics.utc(t.recorded_at)))
 FROM data.transaction_views t LEFT JOIN core.ledger_accounts a ON a.id=t.account_id LEFT JOIN data.account_views v ON v.account_id=t.account_id
 WHERE p_dataset='ledger_entries' AND t.recorded_at>=p_from AND t.recorded_at<=p_to
 UNION ALL
 SELECT 'account_views:'||v.account_id||':v'||v.source_version,v.account_id,v.recorded_at,
  jsonb_build_object('accountId',v.account_id,'partyId',v.party_id,'productVersionId',v.product_version_id,'state',v.state,
   'sourceVersion',v.source_version::text,'sourceAsOf',analytics.utc(v.source_as_of),'debitBlocked',v.debit_blocked,'creditBlocked',v.credit_blocked)
 FROM data.account_views v WHERE p_dataset='account_states' AND v.recorded_at>=p_from AND v.recorded_at<=p_to
 ORDER BY 3 LIMIT p_limit $$;
CREATE FUNCTION analytics.ingest(p_dataset text,p_mode text,p_from timestamptz,p_to timestamptz,p_limit integer,p_actor jsonb,p_person uuid) RETURNS jsonb
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,analytics AS $$
DECLARE d analytics.datasets;s analytics.dataset_schemas;c record;rec jsonb;f jsonb;k text;digest text;prior analytics.facts;reason text;
 run uuid:=gen_random_uuid();started timestamptz:=clock_timestamp();n_read integer:=0;n_ins integer:=0;n_dup integer:=0;n_conf integer:=0;n_rej integer:=0;
 high timestamptz;hit integer;BEGIN
 SELECT * INTO d FROM analytics.datasets WHERE code=p_dataset;IF NOT FOUND THEN RAISE EXCEPTION 'Dataset unavailable' USING ERRCODE='P0002'; END IF;
 SELECT * INTO s FROM analytics.dataset_schemas WHERE dataset=p_dataset ORDER BY version DESC LIMIT 1;
 FOR c IN SELECT * FROM analytics.candidates(p_dataset,p_from,p_to,p_limit) LOOP
  n_read:=n_read+1;high:=greatest(high,c.recorded_at);reason:=NULL;
  rec:=(SELECT coalesce(jsonb_object_agg(x->>'name',c.candidate->(x->>'name')),'{}'::jsonb) FROM jsonb_array_elements(s.fields) x WHERE c.candidate ? (x->>'name'));
  FOR f IN SELECT * FROM jsonb_array_elements(s.fields) LOOP
   IF rec ? (f->>'name') THEN
    IF NOT analytics.valid_value(f->>'type',rec->(f->>'name')) THEN reason:='INVALID_'||upper(f->>'name'); EXIT; END IF;
   ELSIF (f->>'required')::boolean THEN reason:='MISSING_'||upper(f->>'name'); EXIT; END IF;
  END LOOP;
  IF reason IS NOT NULL THEN
   INSERT INTO analytics.ingestion_rejects VALUES(gen_random_uuid(),run,p_dataset,c.source_ref,reason) ON CONFLICT DO NOTHING;
   GET DIAGNOSTICS hit=ROW_COUNT;n_rej:=n_rej+hit;CONTINUE;
  END IF;
  k:=(SELECT string_agg(rec->>key,'|' ORDER BY ord) FROM unnest(d.economic_key) WITH ORDINALITY u(key,ord));
  digest:=encode(sha256(convert_to((SELECT string_agg(coalesce(rec->>key,''),'|' ORDER BY ord) FROM unnest(d.economic_fields) WITH ORDINALITY u(key,ord)),'UTF8')),'hex');
  SELECT * INTO prior FROM analytics.facts WHERE dataset=p_dataset AND economic_key=k;
  IF FOUND THEN
   IF prior.economic_digest=digest THEN n_dup:=n_dup+1;
   ELSE
    INSERT INTO analytics.ingestion_conflicts VALUES(gen_random_uuid(),run,p_dataset,k,prior.business_date,prior.seq,prior.economic_digest,digest,rec,c.source_ref)
    ON CONFLICT DO NOTHING;GET DIAGNOSTICS hit=ROW_COUNT;IF hit=0 THEN n_dup:=n_dup+1; ELSE n_conf:=n_conf+1; END IF;
   END IF;
   CONTINUE;
  END IF;
  INSERT INTO analytics.facts(dataset,schema_version,economic_key,economic_digest,business_date,account_id,record,source_ref,source_recorded_at,run_id)
  VALUES(p_dataset,s.version,k,digest,(rec->>'businessDate')::date,c.account_id,rec,c.source_ref,c.recorded_at,run);
  n_ins:=n_ins+1;
 END LOOP;
 -- An incremental pass with nothing new records no run; a backfill is always evidenced.
 IF p_mode='BACKFILL' OR n_ins+n_conf+n_rej>0 THEN
  INSERT INTO analytics.ingestion_runs VALUES(run,p_dataset,s.version,p_mode,p_from,p_to,n_read,n_ins,n_dup,n_conf,n_rej,high,p_actor,p_person,started,clock_timestamp());
 END IF;
 INSERT INTO analytics.watermarks AS w VALUES(p_dataset,CASE WHEN p_mode='INCREMENTAL' THEN coalesce(high,p_from) ELSE '-infinity'::timestamptz END,
   CASE WHEN p_mode='BACKFILL' OR n_ins+n_conf+n_rej>0 THEN run END,CASE WHEN p_mode='BACKFILL' OR n_ins+n_conf+n_rej>0 THEN clock_timestamp() END,clock_timestamp())
 ON CONFLICT(dataset) DO UPDATE SET high_water=CASE WHEN p_mode='INCREMENTAL' THEN greatest(w.high_water,coalesce(high,w.high_water)) ELSE w.high_water END,
  last_run=coalesce(EXCLUDED.last_run,w.last_run),last_run_at=coalesce(EXCLUDED.last_run_at,w.last_run_at),checked_at=EXCLUDED.checked_at;
 RETURN jsonb_build_object('runId',CASE WHEN p_mode='BACKFILL' OR n_ins+n_conf+n_rej>0 THEN run END,'dataset',p_dataset,'schemaVersion',s.version,'mode',p_mode,
  'read',n_read,'inserted',n_ins,'duplicates',n_dup,'conflicts',n_conf,'rejected',n_rej,'sourceHighWater',analytics.utc(high));
END $$;
-- Worker: incremental pass per dataset. Rows newer than the safe lag wait for the next pass; a slow source transaction that
-- commits behind the watermark is caught by snapshot STALE_SOURCE detection and repaired with a backfill.
CREATE FUNCTION analytics.ingest_due(p_safe_lag interval DEFAULT interval '30 seconds',p_limit integer DEFAULT 5000) RETURNS jsonb
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,analytics AS $$
DECLARE d text;result jsonb:='[]'::jsonb;BEGIN
 IF p_safe_lag<interval '0' OR p_limit NOT BETWEEN 1 AND 20000 THEN RAISE EXCEPTION 'Invalid ingestion bounds' USING ERRCODE='22023'; END IF;
 IF NOT pg_try_advisory_xact_lock(hashtext('analytics.warehouse')) THEN RETURN result; END IF;
 FOR d IN SELECT code FROM analytics.datasets ORDER BY code LOOP
  result:=result||analytics.ingest(d,'INCREMENTAL',coalesce((SELECT high_water FROM analytics.watermarks WHERE dataset=d),'-infinity'::timestamptz),
   clock_timestamp()-p_safe_lag,p_limit,NULL,NULL);
 END LOOP;
 RETURN result;
END $$;
CREATE FUNCTION analytics.backfill(p_actor jsonb,p_person uuid,p_dataset text,p_from timestamptz,p_to timestamptz) RETURNS jsonb
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,analytics AS $$
DECLARE result jsonb;BEGIN
 PERFORM analytics.require(p_actor,p_person,ARRAY['data_steward']);
 IF p_from IS NULL OR p_to IS NULL OR p_from>p_to OR p_to-p_from>interval '93 days' THEN RAISE EXCEPTION 'Backfill window must be at most 93 days' USING ERRCODE='22023'; END IF;
 PERFORM pg_advisory_xact_lock(hashtext('analytics.warehouse'));
 result:=analytics.ingest(p_dataset,'BACKFILL',p_from,least(p_to,clock_timestamp()),20000,p_actor,p_person);
 PERFORM analytics.audit(p_actor,p_person,'analytics.backfill',result->>'runId','RECORDED','BACKFILL');
 RETURN result;
END $$;

-- ===== Transform: contributing facts with dimensions and inclusion =====
CREATE FUNCTION analytics.contributions(p_version integer,p_purpose text,p_from date,p_to date,p_cut bigint,p_master_cut timestamptz)
RETURNS TABLE(seq bigint,party uuid,dimensions jsonb,amount bigint,included boolean) LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
 WITH f AS (SELECT f.seq,f.account_id,f.record FROM analytics.facts f WHERE f.dataset='ledger_entries' AND f.business_date BETWEEN p_from AND p_to AND f.seq<=p_cut),
 st AS (SELECT DISTINCT ON (s.account_id) s.account_id,s.record FROM analytics.facts s
  WHERE s.dataset='account_states' AND s.seq<=p_cut AND s.account_id IN (SELECT account_id FROM f) ORDER BY s.account_id,(s.record->>'sourceVersion')::bigint DESC),
 j AS (SELECT f.seq,f.record,st.record AS state,CASE WHEN f.record->>'accountKind'='CUSTOMER' AND st.record IS NOT NULL
   THEN analytics.canonical_as_of((st.record->>'partyId')::uuid,p_master_cut) END AS party FROM f LEFT JOIN st ON st.account_id=f.account_id)
 SELECT j.seq,j.party,
  jsonb_build_object('businessDate',j.record->>'businessDate','postingKind',j.record->>'postingKind','partyRef',
   CASE WHEN j.record->>'accountKind'<>'CUSTOMER' THEN 'INTERNAL:'||(j.record->>'accountKind') WHEN j.party IS NULL THEN 'UNRESOLVED' ELSE j.party::text END)
  ||CASE WHEN p_version>=2 THEN jsonb_build_object('productVersionId',coalesce(j.record->>'productVersionId',j.state->>'productVersionId','UNRESOLVED')) ELSE '{}'::jsonb END,
  (j.record->>'amountMinor')::bigint,
  (SELECT p.mandatory FROM analytics.purposes p WHERE p.code=p_purpose) OR j.record->>'accountKind'<>'CUSTOMER'
   OR (j.state IS NOT NULL AND NOT analytics.party_restricted((j.state->>'partyId')::uuid,p_master_cut))
 FROM j $$;
CREATE FUNCTION analytics.result_digest(p_version integer,p_purpose text,p_from date,p_to date,p_cut bigint,p_master_cut timestamptz) RETURNS text
LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
 SELECT encode(sha256(convert_to(coalesce(string_agg(r.dimensions::text||'|'||r.n||'|'||r.credit||'|'||r.debit,E'\n' ORDER BY r.dimensions::text),''),'UTF8')),'hex')
 FROM (SELECT c.dimensions,count(*) n,sum(greatest(c.amount,0)) credit,sum(greatest(-c.amount,0)) debit
  FROM analytics.contributions(p_version,p_purpose,p_from,p_to,p_cut,p_master_cut) c WHERE c.included GROUP BY c.dimensions) r $$;

-- ===== Snapshots: purpose-checked, lineage-complete, control-totalled =====
CREATE FUNCTION analytics.create_snapshot(p_actor jsonb,p_person uuid,p_transform text,p_version integer,p_purpose text,p_from date,p_to date) RETURNS jsonb
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,analytics AS $$
DECLARE t analytics.transforms;p analytics.purposes;target uuid:=gen_random_uuid();denial text;cut bigint;mcut timestamptz;flags jsonb:='[]'::jsonb;
 inputs jsonb;marks jsonb;control jsonb;prev analytics.snapshots;explanation jsonb;digest text;i jsonb;missing integer;
 src record;inp record;exc record;outp record;e jsonb;BEGIN
 SELECT * INTO p FROM analytics.purposes WHERE code=p_purpose;IF NOT FOUND THEN RAISE EXCEPTION 'Unknown purpose' USING ERRCODE='22023'; END IF;
 SELECT * INTO t FROM analytics.transforms WHERE code=p_transform AND version=p_version;IF NOT FOUND THEN RAISE EXCEPTION 'Transform unavailable' USING ERRCODE='P0002'; END IF;
 IF p_from IS NULL OR p_to IS NULL OR p_from>p_to OR p_to-p_from>92 THEN RAISE EXCEPTION 'Business window must be at most 92 days' USING ERRCODE='22023'; END IF;
 BEGIN PERFORM controls.require_staff(p_actor,p_person,ARRAY(SELECT role FROM analytics.purpose_roles WHERE purpose=p_purpose));
 EXCEPTION WHEN raise_exception THEN denial:='ROLE_NOT_PERMITTED_FOR_PURPOSE'; END;
 IF denial IS NULL AND EXISTS(SELECT 1 FROM jsonb_array_elements(t.inputs) x JOIN analytics.datasets d ON d.code=x->>'dataset' WHERE NOT p_purpose=ANY(d.purposes)) THEN
  denial:='PURPOSE_NOT_PERMITTED_FOR_DATASET';
 END IF;
 IF denial IS NOT NULL THEN
  INSERT INTO analytics.access_denials VALUES(target,p_purpose,p_transform,p_version,denial,p_actor,p_person);
  PERFORM analytics.audit(p_actor,p_person,'analytics.snapshot',target::text,'DENIED',denial);
  RETURN jsonb_build_object('outcome','DENIED','reason',denial,'denialId',target);
 END IF;
 PERFORM pg_advisory_xact_lock(hashtext('analytics.warehouse'));
 cut:=coalesce((SELECT max(seq) FROM analytics.facts),0);mcut:=clock_timestamp();
 -- Missing and stale sources for this window.
 FOR i IN SELECT * FROM jsonb_array_elements(t.inputs) LOOP
  IF NOT EXISTS(SELECT 1 FROM analytics.watermarks WHERE dataset=i->>'dataset' AND last_run IS NOT NULL) THEN
   flags:=flags||jsonb_build_object('code','MISSING_SOURCE','dataset',i->>'dataset');
  END IF;
 END LOOP;
 SELECT count(*) INTO missing FROM data.transaction_views v WHERE v.document->>'businessDate' BETWEEN p_from::text AND p_to::text
  AND NOT EXISTS(SELECT 1 FROM analytics.facts f WHERE f.dataset='ledger_entries' AND f.seq<=cut
   AND f.economic_key=v.operation_id||'|'||(v.document->>'postingKind')||'|'||v.account_id);
 IF missing>0 THEN flags:=flags||jsonb_build_object('code','STALE_SOURCE','dataset','ledger_entries','uningestedRows',missing); END IF;
 SELECT count(DISTINCT f.account_id) INTO missing FROM analytics.facts f WHERE f.dataset='ledger_entries' AND f.business_date BETWEEN p_from AND p_to AND f.seq<=cut
  AND f.record->>'accountKind'='CUSTOMER' AND NOT EXISTS(SELECT 1 FROM analytics.facts s WHERE s.dataset='account_states' AND s.account_id=f.account_id AND s.seq<=cut);
 IF missing>0 THEN flags:=flags||jsonb_build_object('code','MISSING_ACCOUNT_STATE','dataset','account_states','accounts',missing); END IF;
 SELECT count(*) INTO missing FROM analytics.ingestion_conflicts WHERE dataset='ledger_entries' AND business_date BETWEEN p_from AND p_to;
 IF missing>0 THEN flags:=flags||jsonb_build_object('code','ECONOMIC_CONFLICT','dataset','ledger_entries','conflicts',missing); END IF;
 -- Control totals: Core journal (authority) = warehouse input = report output + purpose exclusions.
 SELECT count(*) n,coalesce(sum(greatest(e.delta,0)),0) credit,coalesce(sum(greatest(-e.delta,0)),0) debit INTO src
  FROM core.journal_entries e JOIN core.journal_batches b ON b.id=e.batch_id WHERE b.business_date BETWEEN p_from AND p_to;
 SELECT count(*) n,coalesce(sum(greatest(amount,0)),0) credit,coalesce(sum(greatest(-amount,0)),0) debit INTO inp FROM analytics.contributions(p_version,p_purpose,p_from,p_to,cut,mcut) c0;
 SELECT count(*) n,coalesce(sum(greatest(amount,0)),0) credit,coalesce(sum(greatest(-amount,0)),0) debit,count(DISTINCT party) parties INTO exc FROM analytics.contributions(p_version,p_purpose,p_from,p_to,cut,mcut) c0 WHERE NOT included;
 IF (src.n,src.credit,src.debit)<>(inp.n,inp.credit,inp.debit) THEN
  flags:=flags||jsonb_build_object('code','CONTROL_TOTAL_MISMATCH','missingEntries',src.n-inp.n,'missingCreditMinor',(src.credit-inp.credit)::text,'missingDebitMinor',(src.debit-inp.debit)::text);
 END IF;
 SELECT coalesce(jsonb_agg(jsonb_build_object('referenceId',c.reference_id,'conflictId',c.id,'caseId',c.case_id)),'[]'::jsonb) INTO e
  FROM data.reference_conflicts c WHERE c.state='REVIEW_REQUIRED' AND c.reference_id IN (SELECT party FROM analytics.contributions(p_version,p_purpose,p_from,p_to,cut,mcut) c0 WHERE party IS NOT NULL
   UNION SELECT h.reference_id FROM data.master_history h WHERE h.new_canonical IN (SELECT party FROM analytics.contributions(p_version,p_purpose,p_from,p_to,cut,mcut) c0));
 IF jsonb_array_length(e)>0 THEN flags:=flags||jsonb_build_object('code','UNRESOLVED_MASTER_CONFLICT','references',e,'handling','NO_AUTOMATIC_MERGE'); END IF;
 INSERT INTO analytics.snapshot_rows(snapshot_id,row_no,dimensions,entries,credit_minor,debit_minor)
 SELECT target,row_number() OVER (ORDER BY g.dimensions::text),g.dimensions,g.n,g.credit,g.debit
 FROM (SELECT dimensions,count(*) n,sum(greatest(amount,0)) credit,sum(greatest(-amount,0)) debit FROM analytics.contributions(p_version,p_purpose,p_from,p_to,cut,mcut) c0 WHERE included GROUP BY dimensions) g;
 SELECT coalesce(sum(entries),0) n,coalesce(sum(credit_minor),0) credit,coalesce(sum(debit_minor),0) debit INTO outp FROM analytics.snapshot_rows WHERE snapshot_id=target;
 digest:=analytics.result_digest(p_version,p_purpose,p_from,p_to,cut,mcut);
 control:=jsonb_build_object(
  'core',jsonb_build_object('source','CORE_JOURNAL','entries',src.n,'creditMinor',src.credit::text,'debitMinor',src.debit::text),
  'input',jsonb_build_object('entries',inp.n,'creditMinor',inp.credit::text,'debitMinor',inp.debit::text),
  'excluded',jsonb_build_object('entries',exc.n,'creditMinor',exc.credit::text,'debitMinor',exc.debit::text,'parties',exc.parties,'reason',
   CASE WHEN p.mandatory THEN 'NONE_MANDATORY_PURPOSE' ELSE 'ANALYTICS_RESTRICTION_OR_UNVERIFIED_PARTY' END),
  'output',jsonb_build_object('entries',outp.n,'creditMinor',outp.credit::text,'debitMinor',outp.debit::text),
  'reconciled',(src.n,src.credit,src.debit)=(inp.n,inp.credit,inp.debit) AND (inp.n,inp.credit,inp.debit)=(outp.n+exc.n,outp.credit+exc.credit,outp.debit+exc.debit));
 SELECT coalesce(jsonb_agg(jsonb_build_object('dataset',x.dataset,'schemaVersions',x.versions,'runs',x.runs,'facts',x.n) ORDER BY x.dataset),'[]'::jsonb) INTO inputs FROM (
  SELECT f.dataset,jsonb_agg(DISTINCT f.schema_version) versions,jsonb_agg(DISTINCT f.run_id) runs,count(*) n FROM analytics.facts f
  WHERE f.seq IN (SELECT seq FROM analytics.contributions(p_version,p_purpose,p_from,p_to,cut,mcut) c0) OR (f.dataset='account_states' AND f.seq<=cut AND f.account_id IN (SELECT ff.account_id FROM analytics.facts ff WHERE ff.seq IN (SELECT seq FROM analytics.contributions(p_version,p_purpose,p_from,p_to,cut,mcut) c0)))
  GROUP BY f.dataset) x;
 SELECT coalesce(jsonb_object_agg(d.code,jsonb_build_object('highWater',analytics.utc(w.high_water),'lastRun',w.last_run,'lastRunAt',analytics.utc(w.last_run_at),
   'lagSeconds',CASE WHEN w.checked_at IS NULL THEN NULL ELSE floor(extract(epoch FROM clock_timestamp()-w.checked_at)) END,'staleAfterSeconds',extract(epoch FROM d.stale_after)::bigint)),'{}'::jsonb)
  INTO marks FROM analytics.datasets d LEFT JOIN analytics.watermarks w ON w.dataset=d.code WHERE d.code IN (SELECT x->>'dataset' FROM jsonb_array_elements(t.inputs) x);
 SELECT * INTO prev FROM analytics.snapshots WHERE transform=p_transform AND purpose=p_purpose AND business_from=p_from AND business_to=p_to ORDER BY created_at DESC,id DESC LIMIT 1;
 IF FOUND THEN
  explanation:=jsonb_build_object('previousSnapshot',prev.id,'previousDigest',prev.result_digest,'digestChanged',prev.result_digest<>digest,
   'transformVersionChanged',prev.transform_version<>p_version,
   'entriesDelta',outp.n-(prev.control->'output'->>'entries')::bigint,
   'creditDeltaMinor',(outp.credit-(prev.control->'output'->>'creditMinor')::numeric)::text,'debitDeltaMinor',(outp.debit-(prev.control->'output'->>'debitMinor')::numeric)::text,
   'newFacts',(SELECT count(*) FROM analytics.contributions(p_version,p_purpose,p_from,p_to,cut,mcut) c WHERE c.seq>prev.fact_cut),
   'corrections',(SELECT coalesce(jsonb_agg(DISTINCT jsonb_build_object('batchId',f.record->>'batchId','originalBatchId',f.record->>'originalBatchId','postingKind',f.record->>'postingKind')),'[]'::jsonb)
     FROM analytics.facts f WHERE f.seq IN (SELECT seq FROM analytics.contributions(p_version,p_purpose,p_from,p_to,cut,mcut) c0 WHERE seq>prev.fact_cut) AND f.record ? 'originalBatchId'),
   'masterChanges',(SELECT count(*) FROM data.master_history h WHERE h.created_at>prev.master_cut AND h.created_at<=mcut));
 END IF;
 INSERT INTO analytics.snapshots VALUES(target,p_transform,p_version,t.digest,'LOCAL_WAREHOUSE',p_purpose,p_from,p_to,cut,mcut,inputs,marks,flags,
  CASE WHEN jsonb_array_length(flags)=0 THEN 'COMPLETE' ELSE 'INCOMPLETE' END,control,digest,prev.id,explanation,p_actor,p_person);
 e:=jsonb_build_object('eventId',gen_random_uuid(),'eventType','analytics.snapshot-published','schemaVersion',1,'producer','data','aggregateId',target,'aggregateVersion',1,
  'occurredAt',analytics.utc(clock_timestamp()),'correlationId',target,'causationId',target::text,'data',jsonb_build_object('snapshotId',target,'transform',p_transform,
  'transformVersion',p_version,'purpose',p_purpose,'businessFrom',p_from,'businessTo',p_to,'status',CASE WHEN jsonb_array_length(flags)=0 THEN 'COMPLETE' ELSE 'INCOMPLETE' END,
  'resultDigest',digest,'supersedes',prev.id));
 INSERT INTO data.outbox(event_id,aggregate_id,aggregate_version,document,fingerprint) VALUES((e->>'eventId')::uuid,target,1,e,encode(sha256(convert_to(core.periodic_policy_canonical(e),'UTF8')),'hex'));
 PERFORM analytics.audit(p_actor,p_person,'analytics.snapshot',target::text,'RECORDED',CASE WHEN jsonb_array_length(flags)=0 THEN 'COMPLETE' ELSE 'INCOMPLETE' END);
 RETURN analytics.snapshot_document(target,true)||jsonb_build_object('outcome','CREATED');
END $$;
CREATE FUNCTION analytics.snapshot_document(p_id uuid,p_rows boolean) RETURNS jsonb LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
 SELECT jsonb_build_object('id',s.id,'transform',s.transform,'transformVersion',s.transform_version,'transformDigest',s.transform_digest,
  'adapter',s.adapter,'purpose',s.purpose,'businessFrom',s.business_from,'businessTo',s.business_to,'factCut',s.fact_cut,'masterCut',analytics.utc(s.master_cut),
  'inputs',s.inputs,'sourceWatermarks',s.source_watermarks,'completeness',s.completeness,'status',s.status,'control',s.control,'resultDigest',s.result_digest,
  'supersedes',s.supersedes,'explanation',s.explanation,'requestedBy',s.requested_by->>'subject','createdAt',analytics.utc(s.created_at),
  'supersededBy',(SELECT n.id FROM analytics.snapshots n WHERE n.supersedes=s.id ORDER BY n.created_at LIMIT 1),
  'reproductions',(SELECT coalesce(jsonb_agg(jsonb_build_object('reproduced',r.reproduced,'at',analytics.utc(r.reproduced_at)) ORDER BY r.reproduced_at),'[]'::jsonb) FROM analytics.reproductions r WHERE r.snapshot_id=s.id))
 ||CASE WHEN p_rows THEN jsonb_build_object('rows',(SELECT coalesce(jsonb_agg(jsonb_build_object('rowNo',r.row_no,'dimensions',r.dimensions,'entries',r.entries,
  'creditMinor',r.credit_minor::text,'debitMinor',r.debit_minor::text) ORDER BY r.row_no),'[]'::jsonb) FROM analytics.snapshot_rows r WHERE r.snapshot_id=s.id)) ELSE '{}'::jsonb END
 FROM analytics.snapshots s WHERE s.id=p_id $$;
CREATE FUNCTION analytics.require_snapshot(p_actor jsonb,p_person uuid,p_id uuid) RETURNS analytics.snapshots LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,analytics AS $$
DECLARE s analytics.snapshots;BEGIN
 SELECT * INTO s FROM analytics.snapshots WHERE id=p_id;IF NOT FOUND THEN RAISE EXCEPTION 'Snapshot unavailable' USING ERRCODE='P0002'; END IF;
 PERFORM analytics.require(p_actor,p_person,ARRAY(SELECT role FROM analytics.purpose_roles WHERE purpose=s.purpose));
 RETURN s;
END $$;
CREATE FUNCTION analytics.snapshot(p_actor jsonb,p_person uuid,p_id uuid) RETURNS jsonb LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,analytics AS $$
BEGIN PERFORM analytics.require_snapshot(p_actor,p_person,p_id);RETURN analytics.snapshot_document(p_id,true); END $$;
-- Recompute from the immutable facts at the stored cuts; the digest must match.
CREATE FUNCTION analytics.reproduce(p_actor jsonb,p_person uuid,p_id uuid) RETURNS jsonb LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,analytics AS $$
DECLARE s analytics.snapshots;digest text;BEGIN
 s:=analytics.require_snapshot(p_actor,p_person,p_id);
 digest:=analytics.result_digest(s.transform_version,s.purpose,s.business_from,s.business_to,s.fact_cut,s.master_cut);
 INSERT INTO analytics.reproductions VALUES(gen_random_uuid(),s.id,digest=s.result_digest,digest,p_actor,p_person);
 PERFORM analytics.audit(p_actor,p_person,'analytics.reproduce',s.id::text,CASE WHEN digest=s.result_digest THEN 'RECORDED' ELSE 'MISMATCH' END,'REPRODUCTION');
 RETURN jsonb_build_object('snapshotId',s.id,'reproduced',digest=s.result_digest,'storedDigest',s.result_digest,'recomputedDigest',digest);
END $$;
-- One aggregate row back to its facts, source references, schema versions and ingestion runs.
CREATE FUNCTION analytics.trace(p_actor jsonb,p_person uuid,p_id uuid,p_row integer) RETURNS jsonb LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,analytics AS $$
DECLARE s analytics.snapshots;r analytics.snapshot_rows;BEGIN
 s:=analytics.require_snapshot(p_actor,p_person,p_id);
 SELECT * INTO r FROM analytics.snapshot_rows WHERE snapshot_id=p_id AND row_no=p_row;IF NOT FOUND THEN RAISE EXCEPTION 'Row unavailable' USING ERRCODE='P0002'; END IF;
 RETURN jsonb_build_object('snapshotId',s.id,'rowNo',r.row_no,'dimensions',r.dimensions,'entries',r.entries,'transform',s.transform,'transformVersion',s.transform_version,
  'transformDigest',s.transform_digest,'purpose',s.purpose,'adapter',s.adapter,'factCut',s.fact_cut,'masterCut',analytics.utc(s.master_cut),
  'facts',(SELECT coalesce(jsonb_agg(jsonb_build_object('seq',f.seq,'economicKey',f.economic_key,'sourceRef',f.source_ref,'schemaVersion',f.schema_version,
    'runId',f.run_id,'amountMinor',f.record->>'amountMinor','batchId',f.record->>'batchId','originalBatchId',f.record->>'originalBatchId',
    'sourceRecordedAt',analytics.utc(f.source_recorded_at)) ORDER BY f.seq),'[]'::jsonb)
   FROM analytics.contributions(s.transform_version,s.purpose,s.business_from,s.business_to,s.fact_cut,s.master_cut) c JOIN analytics.facts f ON f.seq=c.seq
   WHERE c.included AND c.dimensions=r.dimensions));
END $$;
CREATE FUNCTION analytics.view(p_actor jsonb,p_person uuid) RETURNS jsonb LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,analytics AS $$
BEGIN
 PERFORM analytics.require(p_actor,p_person,ARRAY(SELECT DISTINCT role FROM analytics.purpose_roles));
 RETURN jsonb_build_object(
  'adapters',(SELECT jsonb_agg(jsonb_build_object('code',code,'kind',kind,'status',status,'description',description) ORDER BY code) FROM analytics.adapters),
  'purposes',(SELECT jsonb_agg(jsonb_build_object('code',p.code,'mandatory',p.mandatory,'description',p.description,
    'permitted',EXISTS(SELECT 1 FROM controls.role_grants g JOIN analytics.purpose_roles r ON r.role=g.role WHERE r.purpose=p.code AND g.person_id=p_person
     AND g.scope IS NULL AND g.revoked_at IS NULL AND g.valid_from<=clock_timestamp() AND g.valid_until>clock_timestamp())) ORDER BY p.code) FROM analytics.purposes p),
  'datasets',(SELECT jsonb_agg(jsonb_build_object('code',d.code,'source',d.source,'economicKey',d.economic_key,'purposes',d.purposes,'description',d.description,
    'staleAfterSeconds',extract(epoch FROM d.stale_after)::bigint,
    'schemas',(SELECT jsonb_agg(jsonb_build_object('version',s.version,'compatibility',s.compatibility,'fields',s.fields,'registeredAt',analytics.utc(s.registered_at)) ORDER BY s.version)
      FROM analytics.dataset_schemas s WHERE s.dataset=d.code),
    'sourceFields',(SELECT jsonb_agg(jsonb_build_object('name',f.name,'type',f.type) ORDER BY f.name) FROM analytics.source_fields f WHERE f.dataset=d.code),
    'facts',(SELECT count(*) FROM analytics.facts f WHERE f.dataset=d.code),
    'conflicts',(SELECT count(*) FROM analytics.ingestion_conflicts c WHERE c.dataset=d.code),
    'rejects',(SELECT count(*) FROM analytics.ingestion_rejects c WHERE c.dataset=d.code),
    'watermark',(SELECT jsonb_build_object('highWater',analytics.utc(w.high_water),'lastRun',w.last_run,'lastRunAt',analytics.utc(w.last_run_at),'checkedAt',analytics.utc(w.checked_at),
      'stale',w.checked_at<clock_timestamp()-d.stale_after) FROM analytics.watermarks w WHERE w.dataset=d.code)) ORDER BY d.code) FROM analytics.datasets d),
  'transforms',(SELECT jsonb_agg(jsonb_build_object('code',code,'version',version,'dimensions',dimensions,'definition',definition,'digest',digest) ORDER BY code,version) FROM analytics.transforms),
  'runs',(SELECT coalesce(jsonb_agg(x ORDER BY x->>'finishedAt' DESC),'[]'::jsonb) FROM (SELECT jsonb_build_object('id',id,'dataset',dataset,'schemaVersion',schema_version,'mode',mode,
    'read',read_count,'inserted',inserted,'duplicates',duplicates,'conflicts',conflicts,'rejected',rejected,'finishedAt',analytics.utc(finished_at)) x
    FROM analytics.ingestion_runs ORDER BY finished_at DESC LIMIT 20) r),
  'snapshots',(SELECT coalesce(jsonb_agg(x ORDER BY x->>'createdAt' DESC),'[]'::jsonb) FROM (SELECT analytics.snapshot_document(id,false) x FROM analytics.snapshots ORDER BY created_at DESC LIMIT 20) s),
  'denials',(SELECT coalesce(jsonb_agg(x ORDER BY x->>'deniedAt' DESC),'[]'::jsonb) FROM (SELECT jsonb_build_object('id',id,'purpose',purpose,'reason',reason,
    'requestedBy',requested_by->>'subject','deniedAt',analytics.utc(denied_at)) x FROM analytics.access_denials ORDER BY denied_at DESC LIMIT 10) d));
END $$;

GRANT EXECUTE ON FUNCTION analytics.register_schema(jsonb,uuid,text,integer,jsonb),analytics.ingest_due(interval,integer),
 analytics.backfill(jsonb,uuid,text,timestamptz,timestamptz),analytics.create_snapshot(jsonb,uuid,text,integer,text,date,date),
 analytics.snapshot(jsonb,uuid,uuid),analytics.reproduce(jsonb,uuid,uuid),analytics.trace(jsonb,uuid,uuid,integer),analytics.view(jsonb,uuid) TO bank_data;
