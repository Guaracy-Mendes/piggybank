-- Retained customer review evidence only. No enrollment, activation or money authority.
CREATE FUNCTION core.periodic_disclosure_material_v1(p_account uuid,p_policy uuid) RETURNS jsonb
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,core AS $$
DECLARE policy core.periodic_policies; title text; price text; period_text text; days text; holidays text;
 amount bigint; rate integer; material_text text;
BEGIN
 SELECT p.* INTO STRICT policy FROM core.periodic_policies p JOIN core.account_registry r ON r.product_version_id=p.product_version_id WHERE p.id=p_policy AND r.id=p_account;
 IF p_account='00000000-0000-0000-0000-000000000000'::uuid OR policy.document->>'id' IS DISTINCT FROM policy.id::text
 OR policy.document->>'productVersionId' IS DISTINCT FROM policy.product_version_id::text
 OR policy.document->>'code' IS DISTINCT FROM policy.code OR (policy.document->>'version')::integer IS DISTINCT FROM policy.version
 OR (policy.document->>'effectiveFrom')::date IS DISTINCT FROM policy.effective_from OR (policy.document->>'effectiveUntil')::date IS DISTINCT FROM policy.effective_until
 OR policy.document->>'activation' IS DISTINCT FROM 'PREVIEW_ONLY' OR policy.document->>'label' IS DISTINCT FROM 'SYNTHETIC'
 OR policy.document->'calendar'->>'timezone' IS DISTINCT FROM 'UTC'
 OR encode(sha256(convert_to(core.periodic_policy_canonical(policy.document),'UTF8')),'hex') IS DISTINCT FROM policy.digest THEN RAISE EXCEPTION 'Original native preview policy required'; END IF;
 CASE policy.document->'calculation'->>'kind'
 WHEN 'MONTHLY_FIXED_FEE' THEN
  amount:=(policy.document->'calculation'->'amount'->>'minorUnits')::bigint;
  IF amount<=0 THEN RAISE EXCEPTION 'Positive fixed amount required'; END IF;
  title:='Monthly fee preview';
  price:=format('The fixed monthly amount is TST %s.%s (%s minor units) for each complete calendar month. There is no proration and no balance-dependent fee formula.',amount/100,lpad((amount%100)::text,2,'0'),amount);
  period_text:='Each period is one complete calendar month.';
 WHEN 'DAILY_CREDIT_ACCRUAL' THEN
  rate:=(policy.document->'calculation'->>'annualBasisPoints')::integer;
  IF rate NOT BETWEEN 1 AND 10000 THEN RAISE EXCEPTION 'Supported credit rate required'; END IF;
  title:='Daily credit preview';
  price:=format('The annual rate is %s.%s%% (%s basis points). Each calendar day''s nonnegative closing booked balance is multiplied by the annual basis points and divided by 3650000. ACT/365 fixed applies, including leap years. Each day''s result rounds half up to one minor unit. Held funds remain separate from booked balance. This is a calculation preview, not an interest credit.',rate/100,lpad((rate%100)::text,2,'0'),rate);
  period_text:='Each period is one calendar day, including weekends and declared holidays. Only its due date rolls.';
 ELSE RAISE EXCEPTION 'Supported preview calculation required'; END CASE;
 SELECT string_agg((ARRAY['Monday','Tuesday','Wednesday','Thursday','Friday','Saturday','Sunday'])[v::integer],', ' ORDER BY n) INTO days FROM jsonb_array_elements_text(policy.document->'calendar'->'businessWeekdays') WITH ORDINALITY t(v,n);
 SELECT string_agg(v,', ' ORDER BY n) INTO holidays FROM jsonb_array_elements_text(policy.document->'calendar'->'holidays') WITH ORDINALITY t(v,n);
 material_text:=format(E'Synthetic preview only. TST has no monetary value. Reading this disclosure does not enroll your account, accept terms, activate a rule, debit a fee or credit interest.\n\nRule %s version %s covers %s inclusive to %s exclusive.\n\n%s\n\nThe frozen calendar is %s version %s, timezone UTC. Coverage is %s inclusive to %s exclusive. Business weekdays: %s. Explicit holidays: %s. No other holidays are inferred.\n\n%s The due time is %s UTC on the first declared business day on or after the exclusive period end, searching from the period end through fourteen following calendar days. Equality at cutoff is due.\n\nNo customer obligation or financial completion is established. Enrollment, material terms acceptance and financial activation require separate future controls. Changes require a new approved policy version and a new disclosure.',
 policy.code,policy.version,to_char(policy.effective_from,'YYYY-MM-DD'),to_char(policy.effective_until,'YYYY-MM-DD'),price,policy.document->'calendar'->>'code',policy.document->'calendar'->>'version',policy.document->'calendar'->>'coverageFrom',policy.document->'calendar'->>'coverageUntil',days,coalesce(holidays,'None declared'),period_text,policy.document->'calendar'->>'cutoff');
 RETURN jsonb_build_object('formatVersion',1,'source','CORE_APPROVED_PERIODIC_DISCLOSURE_V1','accountId',p_account,'policy',policy.document,'policyDigest',policy.digest,'title',title,'text',material_text);
END $$;
GRANT EXECUTE ON FUNCTION core.periodic_disclosure_material_v1(uuid,uuid) TO bank_core;

CREATE TABLE documents.periodic_preview_reviews(
 id uuid PRIMARY KEY,person_id uuid NOT NULL,account_id uuid NOT NULL REFERENCES core.account_registry(id),policy_id uuid NOT NULL REFERENCES core.periodic_policies(id),
 actor_context jsonb NOT NULL,material_digest text NOT NULL CHECK(material_digest~'^[0-9a-f]{64}$'),
 document jsonb NOT NULL,digest text NOT NULL CHECK(digest~'^[0-9a-f]{64}$'),reviewed_at timestamptz NOT NULL,
 UNIQUE(person_id,account_id,policy_id)
);
CREATE TABLE documents.periodic_review_requests(
 issuer text NOT NULL,subject text NOT NULL,client_id text NOT NULL,request_key uuid NOT NULL,
 person_id uuid NOT NULL,account_id uuid NOT NULL,policy_id uuid NOT NULL,expected_digest text NOT NULL,
 review_id uuid NOT NULL REFERENCES documents.periodic_preview_reviews(id),PRIMARY KEY(issuer,subject,client_id,request_key)
);
REVOKE ALL ON documents.periodic_preview_reviews,documents.periodic_review_requests FROM bank_documents;
GRANT SELECT ON documents.periodic_preview_reviews,documents.periodic_review_requests TO bank_documents;
CREATE TRIGGER immutable_periodic_preview_reviews BEFORE UPDATE OR DELETE OR TRUNCATE ON documents.periodic_preview_reviews FOR EACH STATEMENT EXECUTE FUNCTION documents.immutable_record();
CREATE TRIGGER immutable_periodic_review_requests BEFORE UPDATE OR DELETE OR TRUNCATE ON documents.periodic_review_requests FOR EACH STATEMENT EXECUTE FUNCTION documents.immutable_record();

CREATE FUNCTION documents.periodic_preview_review(p_account uuid,p_policy uuid,p_person uuid) RETURNS jsonb
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,documents AS $$
DECLARE saved documents.periodic_preview_reviews; doc jsonb; material jsonb;
BEGIN
 SELECT * INTO saved FROM documents.periodic_preview_reviews WHERE account_id=p_account AND policy_id=p_policy AND person_id=p_person;
 IF NOT FOUND THEN RETURN NULL; END IF;
 doc:=saved.document;material:=doc->'material';
 IF encode(sha256(convert_to(core.periodic_policy_canonical(doc),'UTF8')),'hex') IS DISTINCT FROM saved.digest
 OR encode(sha256(convert_to(core.periodic_policy_canonical(material),'UTF8')),'hex') IS DISTINCT FROM saved.material_digest
 OR encode(sha256(convert_to(core.periodic_policy_canonical(material->'policy'),'UTF8')),'hex') IS DISTINCT FROM material->>'policyDigest'
 OR doc->>'id' IS DISTINCT FROM saved.id::text OR doc->>'accountId' IS DISTINCT FROM saved.account_id::text OR doc->>'policyId' IS DISTINCT FROM saved.policy_id::text
 OR material->>'accountId' IS DISTINCT FROM saved.account_id::text OR material->'policy'->>'id' IS DISTINCT FROM saved.policy_id::text
 OR doc->>'materialDigest' IS DISTINCT FROM saved.material_digest OR (doc->>'reviewedAt')::timestamptz IS DISTINCT FROM saved.reviewed_at
 OR doc->>'attributionDigest' IS DISTINCT FROM encode(sha256(convert_to(core.periodic_policy_canonical(jsonb_build_object('id',saved.id,'actor',saved.actor_context,'personId',saved.person_id)),'UTF8')),'hex')
 OR doc->>'kind' IS DISTINCT FROM 'PREVIEW_REVIEW_ONLY' OR doc->>'source' IS DISTINCT FROM 'DOCUMENTS_PERIODIC_PREVIEW_REVIEW_V1'
 OR (doc->>'formatVersion')::integer IS DISTINCT FROM 1
 OR doc->>'statement' IS DISTINCT FROM 'I acknowledge reviewing this exact synthetic preview disclosure. This does not enroll my account, accept financial terms, authorize a charge or interest credit, or activate a policy.'
 THEN RAISE EXCEPTION 'Retained preview review integrity conflict'; END IF;
 RETURN jsonb_build_object('review',doc,'digest',saved.digest);
END $$;
GRANT EXECUTE ON FUNCTION documents.periodic_preview_review(uuid,uuid,uuid) TO bank_documents;

CREATE FUNCTION documents.acknowledge_periodic_preview(p_account uuid,p_policy uuid,p_key uuid,p_actor jsonb,p_person uuid,p_digest text,p_acknowledged boolean) RETURNS uuid
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,documents AS $$
DECLARE prior documents.periodic_review_requests; saved documents.periodic_preview_reviews; material jsonb; doc jsonb; event jsonb;
 target uuid:=gen_random_uuid();stamp timestamptz;stamp_text text;expected text;
BEGIN
 IF p_key IS NULL OR p_key='00000000-0000-0000-0000-000000000000'::uuid OR p_digest IS NULL OR p_digest!~'^[0-9a-f]{64}$'
 OR p_acknowledged IS DISTINCT FROM true OR p_actor->>'trust' IS DISTINCT FROM 'CUSTOMER'
 OR coalesce(length(p_actor->>'issuer'),0) NOT BETWEEN 1 AND 2048 OR coalesce(length(p_actor->>'subject'),0) NOT BETWEEN 1 AND 512 OR coalesce(length(p_actor->>'clientId'),0) NOT BETWEEN 1 AND 256
 THEN RAISE EXCEPTION 'Explicit customer preview review and exact material digest required'; END IF;
 PERFORM pg_advisory_xact_lock(hashtextextended(jsonb_build_array(p_actor->>'issuer',p_actor->>'subject',p_actor->>'clientId',p_key)::text,193));
 PERFORM pg_advisory_xact_lock(hashtextextended(jsonb_build_array(p_person,p_account,p_policy)::text,194));
 -- Lock and recheck current authority AFTER both potentially blocking request locks.
 PERFORM 1 FROM controls.person_links WHERE issuer=p_actor->>'issuer' AND subject=p_actor->>'subject' AND person_id=p_person FOR SHARE;
 IF NOT FOUND THEN RAISE EXCEPTION 'Current canonical customer identity required'; END IF;
 PERFORM 1 FROM core.account_relationships WHERE account_id=p_account AND person_id=p_person AND kind='OWNER' AND revoked_at IS NULL FOR SHARE;
 IF NOT FOUND THEN RAISE EXCEPTION 'Current native owner required'; END IF;
 SELECT * INTO prior FROM documents.periodic_review_requests WHERE issuer=p_actor->>'issuer' AND subject=p_actor->>'subject' AND client_id=p_actor->>'clientId' AND request_key=p_key;
 IF FOUND THEN
  IF prior.account_id IS DISTINCT FROM p_account OR prior.policy_id IS DISTINCT FROM p_policy OR prior.person_id IS DISTINCT FROM p_person OR prior.expected_digest IS DISTINCT FROM p_digest THEN RAISE EXCEPTION 'Preview review request scope conflict'; END IF;
  doc:=documents.periodic_preview_review(p_account,p_policy,p_person);
  IF doc->'review'->>'id' IS DISTINCT FROM prior.review_id::text OR doc->'review'->>'materialDigest' IS DISTINCT FROM p_digest THEN RAISE EXCEPTION 'Original preview review link required'; END IF;
  RETURN prior.review_id;
 END IF;
 SELECT * INTO saved FROM documents.periodic_preview_reviews WHERE person_id=p_person AND account_id=p_account AND policy_id=p_policy;
 IF FOUND THEN
  IF saved.material_digest IS DISTINCT FROM p_digest THEN RAISE EXCEPTION 'Original reviewed material required'; END IF;
  PERFORM documents.periodic_preview_review(p_account,p_policy,p_person);
  target:=saved.id;
 ELSE
  material:=core.periodic_disclosure_material_v1(p_account,p_policy);
  expected:=encode(sha256(convert_to(core.periodic_policy_canonical(material),'UTF8')),'hex');
  IF expected IS DISTINCT FROM p_digest THEN RAISE EXCEPTION 'Displayed material changed or digest does not match'; END IF;
  stamp:=clock_timestamp();stamp_text:=to_char(stamp AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"');
  doc:=jsonb_build_object('formatVersion',1,'source','DOCUMENTS_PERIODIC_PREVIEW_REVIEW_V1','kind','PREVIEW_REVIEW_ONLY','id',target,'accountId',p_account,'policyId',p_policy,'material',material,'materialDigest',expected,'reviewedAt',stamp_text,'attributionDigest',encode(sha256(convert_to(core.periodic_policy_canonical(jsonb_build_object('id',target,'actor',p_actor,'personId',p_person)),'UTF8')),'hex'),'statement','I acknowledge reviewing this exact synthetic preview disclosure. This does not enroll my account, accept financial terms, authorize a charge or interest credit, or activate a policy.');
  INSERT INTO documents.periodic_preview_reviews(id,person_id,account_id,policy_id,actor_context,material_digest,document,digest,reviewed_at)
  VALUES(target,p_person,p_account,p_policy,p_actor,expected,doc,encode(sha256(convert_to(core.periodic_policy_canonical(doc),'UTF8')),'hex'),stamp);
  event:=jsonb_build_object('eventId',target,'eventType','periodic.preview.reviewed','schemaVersion',1,'producer','documents','aggregateId',target,'aggregateVersion',1,'occurredAt',stamp_text,'correlationId',p_key,'causationId',p_key::text,'data',jsonb_build_object('reviewId',target,'accountId',p_account,'policyId',p_policy,'materialDigest',expected,'kind','PREVIEW_REVIEW_ONLY'));
  INSERT INTO documents.outbox(event_id,aggregate_id,aggregate_version,document,fingerprint) VALUES(target,target,1,event,encode(sha256(convert_to(core.periodic_policy_canonical(event),'UTF8')),'hex'));
 END IF;
 INSERT INTO documents.periodic_review_requests(issuer,subject,client_id,request_key,person_id,account_id,policy_id,expected_digest,review_id)
 VALUES(p_actor->>'issuer',p_actor->>'subject',p_actor->>'clientId',p_key,p_person,p_account,p_policy,p_digest,target);
 RETURN target;
END $$;
GRANT EXECUTE ON FUNCTION documents.acknowledge_periodic_preview(uuid,uuid,uuid,jsonb,uuid,text,boolean) TO bank_documents;
