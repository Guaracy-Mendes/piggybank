-- Separate immutable policy versions; no posting, account enrollment or charge authority.
ALTER TABLE controls.change_proposals DROP CONSTRAINT change_proposals_kind_check;
ALTER TABLE controls.change_proposals ADD CONSTRAINT change_proposals_kind_check CHECK(kind IN ('PRODUCT','TERMS','CORRECTION','MASTER_REFERENCE','RAIL_POLICY','FINANCE_CHART','FINANCE_REPORT','PERIODIC_POLICY'));
CREATE TABLE core.periodic_policies(
 id uuid PRIMARY KEY,product_version_id uuid NOT NULL REFERENCES core.product_versions(id),
 code text NOT NULL CHECK(code~'^[A-Z][A-Z0-9_]{0,31}$'),version integer NOT NULL CHECK(version>0),
 effective_from date NOT NULL,effective_until date NOT NULL CHECK(effective_until>effective_from AND effective_until-effective_from<=366),
 document jsonb NOT NULL,digest text NOT NULL CHECK(digest~'^[0-9a-f]{64}$'),
 approval_id uuid NOT NULL UNIQUE,maker_person uuid NOT NULL,checker_person uuid NOT NULL,
 published_at timestamptz NOT NULL DEFAULT now(),CHECK(maker_person<>checker_person),
 UNIQUE(product_version_id,code,version),CHECK(document->>'label'='SYNTHETIC'),CHECK(document->>'activation'='PREVIEW_ONLY')
);
REVOKE ALL ON core.periodic_policies FROM bank_core;
GRANT SELECT ON core.periodic_policies TO bank_core;
CREATE TRIGGER immutable_periodic_policies BEFORE UPDATE OR DELETE OR TRUNCATE ON core.periodic_policies FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal();
-- Canonical compact JSON for the bounded integer/ASCII policy protocol; binds native SQL to Controls' digest.
CREATE FUNCTION core.periodic_policy_canonical(p_value jsonb) RETURNS text
LANGUAGE plpgsql IMMUTABLE STRICT SET search_path=pg_catalog,core AS $$
DECLARE result text;
BEGIN
 CASE jsonb_typeof(p_value)
 WHEN 'object' THEN SELECT '{'||coalesce(string_agg(to_jsonb(k)::text||':'||core.periodic_policy_canonical(v),',' ORDER BY k COLLATE "C"),'')||'}' INTO result FROM jsonb_each(p_value) AS t(k,v);
 WHEN 'array' THEN SELECT '['||coalesce(string_agg(core.periodic_policy_canonical(v),',' ORDER BY n),'')||']' INTO result FROM jsonb_array_elements(p_value) WITH ORDINALITY AS t(v,n);
 ELSE result:=p_value::text;
 END CASE;
 RETURN result;
END $$;
CREATE FUNCTION core.publish_periodic_policy(p_document jsonb,p_digest text,p_approval uuid,p_maker uuid,p_checker uuid) RETURNS boolean
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,core AS $$
DECLARE original core.periodic_policies; prior core.periodic_policies; product core.product_versions;
 policy_id uuid:=(p_document->>'id')::uuid; product_id uuid:=(p_document->>'productVersionId')::uuid;
 code_value text:=p_document->>'code'; version_value integer:=(p_document->>'version')::integer;
 from_value date:=(p_document->>'effectiveFrom')::date; until_value date:=(p_document->>'effectiveUntil')::date;
BEGIN
 PERFORM pg_advisory_xact_lock(hashtextextended(product_id::text||'/'||code_value,176));
 SELECT * INTO original FROM core.periodic_policies WHERE id=policy_id;
 IF FOUND THEN
  IF original.document<>p_document OR original.digest<>p_digest OR original.approval_id<>p_approval OR original.maker_person<>p_maker OR original.checker_person<>p_checker THEN RAISE EXCEPTION 'Periodic policy identity conflict'; END IF;
  RETURN false;
 END IF;
 IF encode(sha256(convert_to(core.periodic_policy_canonical(p_document),'UTF8')),'hex')<>p_digest THEN RAISE EXCEPTION 'Periodic policy digest mismatch'; END IF;
 IF NOT EXISTS(SELECT 1 FROM controls.change_proposals WHERE id=p_approval AND kind='PERIODIC_POLICY' AND resource_id=policy_id AND digest=p_digest AND maker_person=p_maker AND checker_person=p_checker AND state='APPROVED' AND expires_at>now()) THEN RAISE EXCEPTION 'Exact approved periodic policy required'; END IF;
 IF p_maker=p_checker OR NOT EXISTS(SELECT 1 FROM controls.role_grants WHERE person_id=p_maker AND role='finance_adjuster' AND scope IS NULL AND revoked_at IS NULL AND valid_from<=now() AND valid_until>now()) OR NOT EXISTS(SELECT 1 FROM controls.role_grants WHERE person_id=p_checker AND role='finance_approver' AND scope IS NULL AND revoked_at IS NULL AND valid_from<=now() AND valid_until>now()) OR NOT EXISTS(SELECT 1 FROM controls.case_queue_members WHERE person_id=p_maker AND queue='finance-operations' AND capability='REVIEW' AND revoked_at IS NULL) OR NOT EXISTS(SELECT 1 FROM controls.case_queue_members WHERE person_id=p_checker AND queue='finance-operations' AND capability='APPROVE' AND revoked_at IS NULL) THEN RAISE EXCEPTION 'Current independent Finance role and queue required'; END IF;
 SELECT * INTO STRICT product FROM core.product_versions WHERE id=product_id;
 IF from_value<product.effective_from OR until_value>product.effective_until THEN RAISE EXCEPTION 'Periodic policy outside product validity'; END IF;
 SELECT * INTO prior FROM core.periodic_policies WHERE product_version_id=product_id AND code=code_value ORDER BY version DESC LIMIT 1;
 IF FOUND THEN
  IF version_value<>prior.version+1 OR from_value<prior.effective_until OR from_value<(now() AT TIME ZONE 'UTC')::date OR (prior.document->'calculation'->>'kind') IS DISTINCT FROM (p_document->'calculation'->>'kind') THEN RAISE EXCEPTION 'Sequential future nonoverlapping periodic version required'; END IF;
 ELSE
  IF version_value<>1 THEN RAISE EXCEPTION 'First periodic version must be one'; END IF;
 END IF;
 INSERT INTO core.periodic_policies(id,product_version_id,code,version,effective_from,effective_until,document,digest,approval_id,maker_person,checker_person)
 VALUES(policy_id,product_id,code_value,version_value,from_value,until_value,p_document,p_digest,p_approval,p_maker,p_checker);
 RETURN true;
END $$;
GRANT EXECUTE ON FUNCTION core.publish_periodic_policy(jsonb,text,uuid,uuid,uuid) TO bank_core;
CREATE TABLE finance.periodic_policy_requests(
 id uuid PRIMARY KEY,policy_id uuid NOT NULL,issuer text NOT NULL,subject text NOT NULL,client_id text NOT NULL,
 request_key uuid NOT NULL,actor_context jsonb NOT NULL,maker_person uuid NOT NULL,
 policy jsonb NOT NULL,policy_digest text NOT NULL,reason text NOT NULL CHECK(length(reason) BETWEEN 4 AND 240),
 expires_at timestamptz NOT NULL,created_at timestamptz NOT NULL DEFAULT now(),registered_at timestamptz,
 registration_attempts integer NOT NULL DEFAULT 0,registration_reason text NOT NULL DEFAULT 'REGISTRATION_PENDING',
 UNIQUE(issuer,subject,client_id,request_key)
);
CREATE FUNCTION finance.guard_periodic_policy_request() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF TG_OP<>'UPDATE' OR (to_jsonb(NEW)-ARRAY['registered_at','registration_attempts','registration_reason'])<>(to_jsonb(OLD)-ARRAY['registered_at','registration_attempts','registration_reason']) OR (OLD.registered_at IS NOT NULL AND NEW.registered_at IS DISTINCT FROM OLD.registered_at) OR NEW.registration_attempts<OLD.registration_attempts THEN RAISE EXCEPTION 'Immutable periodic policy command'; END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER immutable_periodic_policy_request BEFORE UPDATE OR DELETE ON finance.periodic_policy_requests FOR EACH ROW EXECUTE FUNCTION finance.guard_periodic_policy_request();
CREATE TRIGGER immutable_periodic_policy_request_set BEFORE TRUNCATE ON finance.periodic_policy_requests FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal();
CREATE TABLE finance.periodic_policy_cursor(id boolean PRIMARY KEY DEFAULT true CHECK(id),last_id uuid NOT NULL DEFAULT '00000000-0000-0000-0000-000000000000');
INSERT INTO finance.periodic_policy_cursor DEFAULT VALUES;
