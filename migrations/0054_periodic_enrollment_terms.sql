-- Separately approved financial terms and withdrawable requests for future Core activation.
-- Preview policies/reviews stay unchanged. These functions never post or activate money.
-- Runtime roles cannot redefine this native clock. Disposable tests can advance it
-- under the migration owner without editing financial facts or accepting API clocks.
CREATE FUNCTION core.periodic_clock() RETURNS timestamptz LANGUAGE sql VOLATILE SECURITY DEFINER SET search_path=pg_catalog AS $$ SELECT clock_timestamp() $$;
GRANT EXECUTE ON FUNCTION core.periodic_clock() TO bank_core;
ALTER TABLE controls.change_proposals DROP CONSTRAINT change_proposals_kind_check;
ALTER TABLE controls.change_proposals ADD CONSTRAINT change_proposals_kind_check CHECK(kind IN ('PRODUCT','TERMS','CORRECTION','MASTER_REFERENCE','RAIL_POLICY','FINANCE_CHART','FINANCE_REPORT','PERIODIC_POLICY','FINANCE_CORRECTION','PERIODIC_OFFER'));

CREATE FUNCTION documents.periodic_offer_material_v1(p_id uuid,p_policy uuid) RETURNS jsonb
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,documents AS $$
DECLARE policy core.periodic_policies; price text; period_text text; title text;
BEGIN
 SELECT * INTO STRICT policy FROM core.periodic_policies WHERE id=p_policy;
 IF p_id IS NULL OR p_id='00000000-0000-0000-0000-000000000000'::uuid
 OR policy.document->>'id' IS DISTINCT FROM policy.id::text OR policy.document->>'productVersionId' IS DISTINCT FROM policy.product_version_id::text
 OR policy.document->>'activation' IS DISTINCT FROM 'PREVIEW_ONLY' OR policy.document->>'label' IS DISTINCT FROM 'SYNTHETIC'
 OR policy.document->>'code' IS DISTINCT FROM policy.code OR (policy.document->>'version')::integer IS DISTINCT FROM policy.version
 OR (policy.document->>'effectiveFrom')::date IS DISTINCT FROM policy.effective_from OR (policy.document->>'effectiveUntil')::date IS DISTINCT FROM policy.effective_until
 OR policy.document->'calendar'->>'timezone' IS DISTINCT FROM 'UTC'
 OR encode(sha256(convert_to(core.periodic_policy_canonical(policy.document),'UTF8')),'hex') IS DISTINCT FROM policy.digest THEN RAISE EXCEPTION 'Original approved calculation reference required'; END IF;
 CASE policy.document->'calculation'->>'kind'
 WHEN 'MONTHLY_FIXED_FEE' THEN
  title:='Synthetic monthly fee enrollment terms';
  price:='The fixed fee for each complete eligible calendar month is '||(policy.document->'calculation'->'amount'->>'minorUnits')||' TST minor units. TST has scale 2. No proration applies.';
  period_text:='The earliest eligible month starts on the first day of the next UTC calendar month after acceptance, or the calculation reference start if later.';
 WHEN 'DAILY_CREDIT_ACCRUAL' THEN
  title:='Synthetic daily credit enrollment terms';
  price:='The annual credit rate is '||(policy.document->'calculation'->>'annualBasisPoints')||' basis points. Nonnegative closing booked TST minor units multiply that rate and divide by 3650000. ACT/365 fixed applies including leap years, with half-up rounding to one minor unit each day. Holds are separate from booked balance.';
  period_text:='The earliest eligible day is the next UTC calendar day after acceptance, or the calculation reference start if later.';
 ELSE RAISE EXCEPTION 'Supported calculation reference required'; END CASE;
 RETURN jsonb_build_object('formatVersion',1,'source','DOCUMENTS_PERIODIC_ENROLLMENT_TERMS_V1','id',p_id,'kind','SYNTHETIC_PERIODIC_ENROLLMENT_OFFER','productVersionId',policy.product_version_id,'policy',policy.document,'policyDigest',policy.digest,'title',title,'text',
 E'SYNTHETIC ENROLLMENT TERMS. TST has no monetary value. These separately approved terms are distinct from preview disclosures and review acknowledgments.\n\n'||price||E'\n\n'||period_text||' Every eligible period must be wholly within '||to_char(policy.effective_from,'YYYY-MM-DD')||' inclusive to '||to_char(policy.effective_until,'YYYY-MM-DD')||E' exclusive. Acceptance never authorizes retroactive periods.\n\nThe complete frozen calculation reference and UTC calendar are part of these terms. Only explicitly declared business weekdays and holidays apply. The due time is the calendar cutoff on the first declared business day on or after the exclusive period end, searching offsets zero through fourteen calendar days.\n\nBy explicitly accepting this exact offer, the sole current account owner requests enrollment and consents to the described synthetic fee or credit only for eligible periods after separate Core activation. The request remains PENDING_ACTIVATION; financial effects require separate approved activation and native due-period execution. No funds are reserved by acceptance. Current account restrictions, ownership, eligibility and authoritative economic finalization must be rechecked before any future financial effect.\n\nYou may withdraw this pending request before activation. Withdrawal prevents its future activation and cannot be undone by replaying acceptance. A new approved offer and explicit acceptance are required for a new enrollment after withdrawal. Different calculation or terms material requires new approval and acceptance. Reading or acknowledging preview information never constitutes acceptance of these terms.');
END $$;
GRANT EXECUTE ON FUNCTION documents.periodic_offer_material_v1(uuid,uuid) TO bank_documents;

CREATE FUNCTION documents.require_periodic_finance_person(p_person uuid,p_checker boolean) RETURNS void
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,documents AS $$
BEGIN
 PERFORM 1 FROM controls.role_grants WHERE person_id=p_person AND role=CASE WHEN p_checker THEN 'finance_approver' ELSE 'finance_adjuster' END AND scope IS NULL AND revoked_at IS NULL AND valid_from<=clock_timestamp() AND valid_until>clock_timestamp() FOR SHARE;
 IF NOT FOUND THEN RAISE EXCEPTION 'Current Finance role required'; END IF;
 PERFORM 1 FROM controls.case_queue_members WHERE person_id=p_person AND queue='finance-operations' AND capability=CASE WHEN p_checker THEN 'APPROVE' ELSE 'REVIEW' END AND revoked_at IS NULL FOR SHARE;
 IF NOT FOUND THEN RAISE EXCEPTION 'Current Finance queue required'; END IF;
END $$;
CREATE FUNCTION documents.require_periodic_actor(p_actor jsonb,p_person uuid,p_staff boolean) RETURNS void
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,documents AS $$
BEGIN
 IF p_actor->>'trust' IS DISTINCT FROM (CASE WHEN p_staff THEN 'WORKFORCE' ELSE 'CUSTOMER' END) THEN RAISE EXCEPTION 'Current trust context required'; END IF;
 PERFORM 1 FROM controls.person_links WHERE issuer=p_actor->>'issuer' AND subject=p_actor->>'subject' AND person_id=p_person FOR SHARE;
 IF NOT FOUND THEN RAISE EXCEPTION 'Current canonical identity required'; END IF;
 IF coalesce(length(p_actor->>'clientId'),0) NOT BETWEEN 1 AND 256 THEN RAISE EXCEPTION 'Current client required'; END IF;
 IF p_staff THEN PERFORM documents.require_periodic_finance_person(p_person,false); END IF;
END $$;

CREATE TABLE documents.periodic_offer_requests(
 id uuid PRIMARY KEY,issuer text NOT NULL,subject text NOT NULL,client_id text NOT NULL,request_key uuid NOT NULL,
 actor_context jsonb NOT NULL,maker_person uuid NOT NULL,policy_id uuid NOT NULL REFERENCES core.periodic_policies(id),material jsonb NOT NULL,digest text NOT NULL,reason text NOT NULL,
 created_at timestamptz NOT NULL,expires_at timestamptz NOT NULL,UNIQUE(issuer,subject,client_id,request_key)
);
CREATE TABLE documents.periodic_offers(
 id uuid PRIMARY KEY REFERENCES documents.periodic_offer_requests(id),policy_id uuid NOT NULL UNIQUE REFERENCES core.periodic_policies(id),product_version_id uuid NOT NULL REFERENCES core.product_versions(id),
 material jsonb NOT NULL,digest text NOT NULL,maker_person uuid NOT NULL,checker_person uuid NOT NULL,published_at timestamptz NOT NULL,publication_digest text NOT NULL,CHECK(maker_person<>checker_person)
);
CREATE TABLE documents.periodic_enrollment_consents(
 id uuid PRIMARY KEY,account_id uuid NOT NULL REFERENCES core.account_registry(id),offer_id uuid NOT NULL REFERENCES documents.periodic_offers(id),person_id uuid NOT NULL,actor_context jsonb NOT NULL,
 document jsonb NOT NULL,digest text NOT NULL,accepted_at timestamptz NOT NULL,UNIQUE(account_id,offer_id)
);
CREATE TABLE documents.periodic_enrollment_withdrawals(
 id uuid PRIMARY KEY,consent_id uuid NOT NULL UNIQUE REFERENCES documents.periodic_enrollment_consents(id),person_id uuid NOT NULL,actor_context jsonb NOT NULL,document jsonb NOT NULL,digest text NOT NULL,withdrawn_at timestamptz NOT NULL
);
CREATE TABLE documents.periodic_enrollment_requests(
 issuer text NOT NULL,subject text NOT NULL,client_id text NOT NULL,request_key uuid NOT NULL,person_id uuid NOT NULL,account_id uuid NOT NULL,offer_id uuid NOT NULL,action text NOT NULL CHECK(action IN ('ENROLL','WITHDRAW')),expected_digest text NOT NULL,consent_id uuid NOT NULL REFERENCES documents.periodic_enrollment_consents(id),receipt_id uuid NOT NULL,
 PRIMARY KEY(issuer,subject,client_id,request_key)
);
REVOKE ALL ON documents.periodic_offer_requests,documents.periodic_offers,documents.periodic_enrollment_consents,documents.periodic_enrollment_withdrawals,documents.periodic_enrollment_requests FROM bank_documents;
GRANT SELECT ON documents.periodic_offer_requests,documents.periodic_offers,documents.periodic_enrollment_consents,documents.periodic_enrollment_withdrawals,documents.periodic_enrollment_requests TO bank_documents;
DO $$ DECLARE t text; BEGIN FOREACH t IN ARRAY ARRAY['periodic_offer_requests','periodic_offers','periodic_enrollment_consents','periodic_enrollment_withdrawals','periodic_enrollment_requests'] LOOP
 EXECUTE format('CREATE TRIGGER immutable_%I BEFORE UPDATE OR DELETE OR TRUNCATE ON documents.%I FOR EACH STATEMENT EXECUTE FUNCTION documents.immutable_record()',t,t);
END LOOP; END $$;

CREATE FUNCTION documents.stage_periodic_offer(p_policy uuid,p_key uuid,p_reason text,p_actor jsonb,p_person uuid) RETURNS uuid
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,documents AS $$
DECLARE saved documents.periodic_offer_requests;target uuid:=gen_random_uuid();material jsonb;stamp timestamptz;
BEGIN
 IF p_key IS NULL OR p_key='00000000-0000-0000-0000-000000000000'::uuid OR length(p_reason) NOT BETWEEN 4 AND 240 OR p_reason<>btrim(p_reason) OR p_reason~'[[:cntrl:]]' THEN RAISE EXCEPTION 'Exact request key and reason required'; END IF;
 PERFORM pg_advisory_xact_lock(hashtextextended(jsonb_build_array(p_actor->>'issuer',p_actor->>'subject',p_actor->>'clientId',p_key)::text,201));
 PERFORM documents.require_periodic_actor(p_actor,p_person,true);
 SELECT * INTO saved FROM documents.periodic_offer_requests WHERE issuer=p_actor->>'issuer' AND subject=p_actor->>'subject' AND client_id=p_actor->>'clientId' AND request_key=p_key;
 IF FOUND THEN
  IF saved.policy_id IS DISTINCT FROM p_policy OR saved.reason IS DISTINCT FROM p_reason OR saved.maker_person IS DISTINCT FROM p_person THEN RAISE EXCEPTION 'Offer request scope conflict'; END IF;
  RETURN saved.id;
 END IF;
 material:=documents.periodic_offer_material_v1(target,p_policy);stamp:=clock_timestamp();
 INSERT INTO documents.periodic_offer_requests VALUES(target,p_actor->>'issuer',p_actor->>'subject',p_actor->>'clientId',p_key,p_actor,p_person,p_policy,material,encode(sha256(convert_to(core.periodic_policy_canonical(material),'UTF8')),'hex'),p_reason,stamp,stamp+interval '24 hours');
 RETURN target;
END $$;
GRANT EXECUTE ON FUNCTION documents.stage_periodic_offer(uuid,uuid,text,jsonb,uuid) TO bank_documents;

CREATE FUNCTION documents.published_periodic_offer(p_id uuid) RETURNS jsonb
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,documents AS $$
DECLARE saved documents.periodic_offers;proof jsonb;
BEGIN
 SELECT * INTO saved FROM documents.periodic_offers WHERE id=p_id;IF NOT FOUND THEN RETURN NULL; END IF;
 proof:=jsonb_build_object('id',saved.id,'digest',saved.digest,'makerPerson',saved.maker_person,'checkerPerson',saved.checker_person,'publishedAt',to_char(saved.published_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'));
 IF saved.material->>'id' IS DISTINCT FROM saved.id::text OR saved.material->>'productVersionId' IS DISTINCT FROM saved.product_version_id::text OR saved.material->'policy'->>'id' IS DISTINCT FROM saved.policy_id::text
 OR encode(sha256(convert_to(core.periodic_policy_canonical(saved.material),'UTF8')),'hex') IS DISTINCT FROM saved.digest
 OR encode(sha256(convert_to(core.periodic_policy_canonical(saved.material->'policy'),'UTF8')),'hex') IS DISTINCT FROM saved.material->>'policyDigest'
 OR encode(sha256(convert_to(core.periodic_policy_canonical(proof),'UTF8')),'hex') IS DISTINCT FROM saved.publication_digest THEN RAISE EXCEPTION 'Published offer integrity conflict'; END IF;
 RETURN jsonb_build_object('material',saved.material,'digest',saved.digest,'publishedAt',proof->>'publishedAt','publicationDigest',saved.publication_digest);
END $$;
GRANT EXECUTE ON FUNCTION documents.published_periodic_offer(uuid) TO bank_documents;

CREATE FUNCTION documents.publish_periodic_offer(p_id uuid,p_actor jsonb,p_person uuid) RETURNS uuid
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,documents AS $$
DECLARE request documents.periodic_offer_requests;approval controls.change_proposals;stamp timestamptz;proof jsonb;material jsonb;
BEGIN
 SELECT * INTO STRICT request FROM documents.periodic_offer_requests WHERE id=p_id;
 PERFORM pg_advisory_xact_lock(hashtextextended(request.policy_id::text,202));
 PERFORM documents.require_periodic_actor(p_actor,p_person,true);
 IF request.maker_person IS DISTINCT FROM p_person THEN RAISE EXCEPTION 'Original canonical maker required'; END IF;
 IF EXISTS(SELECT 1 FROM documents.periodic_offers WHERE id=p_id) THEN PERFORM documents.published_periodic_offer(p_id);RETURN p_id; END IF;
 SELECT * INTO STRICT approval FROM controls.change_proposals WHERE id=p_id FOR SHARE;
 IF approval.kind IS DISTINCT FROM 'PERIODIC_OFFER' OR approval.resource_id IS DISTINCT FROM p_id OR approval.digest IS DISTINCT FROM request.digest OR approval.maker_person IS DISTINCT FROM request.maker_person OR approval.checker_person IS NULL OR approval.checker_person=request.maker_person OR approval.state IS DISTINCT FROM 'APPROVED' OR approval.expires_at<=clock_timestamp() OR request.expires_at<=clock_timestamp() THEN RAISE EXCEPTION 'Exact independent unexpired offer approval required'; END IF;
 PERFORM documents.require_periodic_finance_person(approval.checker_person,true);
 material:=documents.periodic_offer_material_v1(p_id,request.policy_id);
 IF material IS DISTINCT FROM request.material OR encode(sha256(convert_to(core.periodic_policy_canonical(material),'UTF8')),'hex') IS DISTINCT FROM request.digest
 OR (material->'policy'->>'effectiveUntil')::date<=(clock_timestamp() AT TIME ZONE 'UTC')::date THEN RAISE EXCEPTION 'Original unexpired offer material required'; END IF;
 stamp:=clock_timestamp();IF approval.expires_at<=stamp OR request.expires_at<=stamp THEN RAISE EXCEPTION 'Offer approval expired during publication'; END IF;
 proof:=jsonb_build_object('id',p_id,'digest',request.digest,'makerPerson',request.maker_person,'checkerPerson',approval.checker_person,'publishedAt',to_char(stamp AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'));
 INSERT INTO documents.periodic_offers VALUES(p_id,request.policy_id,(material->>'productVersionId')::uuid,material,request.digest,request.maker_person,approval.checker_person,stamp,encode(sha256(convert_to(core.periodic_policy_canonical(proof),'UTF8')),'hex'));
 RETURN p_id;
END $$;
GRANT EXECUTE ON FUNCTION documents.publish_periodic_offer(uuid,jsonb,uuid) TO bank_documents;

CREATE FUNCTION documents.periodic_enrollment(p_account uuid,p_offer uuid,p_person uuid) RETURNS jsonb
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,documents AS $$
DECLARE c documents.periodic_enrollment_consents;w documents.periodic_enrollment_withdrawals;material jsonb;
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
 RETURN jsonb_build_object('consent',c.document,'consentDigest',c.digest,'withdrawal',w.document,'withdrawalDigest',w.digest,'activation',NULL,'status',CASE WHEN w.id IS NULL THEN 'PENDING_ACTIVATION' ELSE 'WITHDRAWN' END,'financialActivation','DISABLED');
END $$;
GRANT EXECUTE ON FUNCTION documents.periodic_enrollment(uuid,uuid,uuid) TO bank_documents;

CREATE FUNCTION documents.request_periodic_enrollment(p_account uuid,p_offer uuid,p_key uuid,p_actor jsonb,p_person uuid,p_action text,p_digest text,p_explicit boolean) RETURNS jsonb
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,documents AS $$
DECLARE prior documents.periodic_enrollment_requests;c documents.periodic_enrollment_consents;w documents.periodic_enrollment_withdrawals;a core.ledger_accounts;r core.account_registry;
 offer jsonb;material jsonb;doc jsonb;event jsonb;stamp timestamptz;stamp_text text;target uuid:=gen_random_uuid();first_period date;period_end date;created boolean:=false;
BEGIN
 IF p_action NOT IN ('ENROLL','WITHDRAW') OR p_action IS NULL OR p_key IS NULL OR p_key='00000000-0000-0000-0000-000000000000'::uuid OR p_digest IS NULL OR p_digest!~'^[0-9a-f]{64}$' OR p_explicit IS DISTINCT FROM true THEN RAISE EXCEPTION 'Explicit exact enrollment command required'; END IF;
 PERFORM pg_advisory_xact_lock(hashtextextended(jsonb_build_array(p_actor->>'issuer',p_actor->>'subject',p_actor->>'clientId',p_key)::text,203));
 PERFORM pg_advisory_xact_lock(hashtextextended(jsonb_build_array(p_account,p_offer)::text,204));
 SELECT * INTO STRICT a FROM core.ledger_accounts WHERE id=p_account FOR UPDATE;
 PERFORM documents.require_periodic_actor(p_actor,p_person,false);
 PERFORM 1 FROM core.account_relationships WHERE account_id=p_account AND person_id=p_person AND kind='OWNER' AND revoked_at IS NULL FOR SHARE;
 IF NOT FOUND THEN RAISE EXCEPTION 'Current account owner required'; END IF;
 SELECT * INTO prior FROM documents.periodic_enrollment_requests WHERE issuer=p_actor->>'issuer' AND subject=p_actor->>'subject' AND client_id=p_actor->>'clientId' AND request_key=p_key;
 IF FOUND THEN
  IF prior.person_id IS DISTINCT FROM p_person OR prior.account_id IS DISTINCT FROM p_account OR prior.offer_id IS DISTINCT FROM p_offer OR prior.action IS DISTINCT FROM p_action OR prior.expected_digest IS DISTINCT FROM p_digest THEN RAISE EXCEPTION 'Enrollment request scope conflict'; END IF;
  doc:=documents.periodic_enrollment(p_account,p_offer,p_person);
  IF doc->'consent'->>'id' IS DISTINCT FROM prior.consent_id::text OR (p_action='WITHDRAW' AND doc->'withdrawal'->>'id' IS DISTINCT FROM prior.receipt_id::text) THEN RAISE EXCEPTION 'Original enrollment receipt required'; END IF;
  RETURN doc;
 END IF;
 SELECT * INTO c FROM documents.periodic_enrollment_consents WHERE account_id=p_account AND offer_id=p_offer;
 IF FOUND AND c.person_id IS DISTINCT FROM p_person THEN RAISE EXCEPTION 'Another canonical owner already requested enrollment'; END IF;
 IF p_action='ENROLL' THEN
  IF c.id IS NOT NULL THEN
   PERFORM documents.periodic_enrollment(p_account,p_offer,p_person);
   IF c.document->'offer'->>'digest' IS DISTINCT FROM p_digest OR EXISTS(SELECT 1 FROM documents.periodic_enrollment_withdrawals WHERE consent_id=c.id) THEN RAISE EXCEPTION 'Existing withdrawn or different consent cannot be replaced'; END IF;
   target:=c.id;
  ELSE
   SELECT * INTO STRICT r FROM core.account_registry WHERE id=p_account;
   offer:=documents.published_periodic_offer(p_offer);IF offer IS NULL THEN RAISE EXCEPTION 'Published offer required'; END IF;material:=offer->'material';
   IF offer->>'digest' IS DISTINCT FROM p_digest OR material->>'productVersionId' IS DISTINCT FROM r.product_version_id::text
   OR a.state IS DISTINCT FROM 'ACTIVE' OR a.kind IS DISTINCT FROM 'CUSTOMER'
   OR (SELECT count(*) FROM core.account_relationships WHERE account_id=p_account AND kind='OWNER' AND revoked_at IS NULL)<>1
   OR (material->'policy'->'calculation'->>'kind'='MONTHLY_FIXED_FEE' AND a.debit_blocked) OR (material->'policy'->'calculation'->>'kind'='DAILY_CREDIT_ACCRUAL' AND a.credit_blocked)
   OR documents.periodic_offer_material_v1(p_offer,(material->'policy'->>'id')::uuid) IS DISTINCT FROM material THEN RAISE EXCEPTION 'Exact current eligible sole-owner offer required'; END IF;
   stamp:=core.periodic_clock();stamp_text:=to_char(stamp AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"');
   IF material->'policy'->'calculation'->>'kind'='MONTHLY_FIXED_FEE' THEN first_period:=greatest((material->'policy'->>'effectiveFrom')::date,(date_trunc('month',stamp AT TIME ZONE 'UTC')+interval '1 month')::date);period_end:=(first_period+interval '1 month')::date;
   ELSE first_period:=greatest((material->'policy'->>'effectiveFrom')::date,(stamp AT TIME ZONE 'UTC')::date+1);period_end:=first_period+1;END IF;
   IF period_end>(material->'policy'->>'effectiveUntil')::date THEN RAISE EXCEPTION USING ERRCODE='P5401',MESSAGE='No complete future eligible period remains'; END IF;
   doc:=jsonb_build_object('formatVersion',1,'source','DOCUMENTS_PERIODIC_ENROLLMENT_CONSENT_V1','kind','PERIODIC_ENROLLMENT_CONSENT','id',target,'accountId',p_account,'offer',offer,'acceptedAt',stamp_text,'earliestPeriodStart',to_char(first_period,'YYYY-MM-DD'),'attributionDigest',encode(sha256(convert_to(core.periodic_policy_canonical(jsonb_build_object('id',target,'personId',p_person,'actor',p_actor)),'UTF8')),'hex'),'statement','I accept these exact synthetic periodic enrollment terms and request enrollment pending separate Core activation. I understand that acceptance itself does not debit fees or credit interest.');
   INSERT INTO documents.periodic_enrollment_consents VALUES(target,p_account,p_offer,p_person,p_actor,doc,encode(sha256(convert_to(core.periodic_policy_canonical(doc),'UTF8')),'hex'),stamp);
   SELECT * INTO STRICT c FROM documents.periodic_enrollment_consents WHERE id=target;
   created:=true;
  END IF;
 ELSE
  IF c.id IS NULL OR c.digest IS DISTINCT FROM p_digest THEN RAISE EXCEPTION 'Original pending consent required for withdrawal'; END IF;
  PERFORM documents.periodic_enrollment(p_account,p_offer,p_person);
  SELECT * INTO w FROM documents.periodic_enrollment_withdrawals WHERE consent_id=c.id;
  IF FOUND THEN target:=w.id;
  ELSE
   stamp:=clock_timestamp();stamp_text:=to_char(stamp AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"');
   doc:=jsonb_build_object('formatVersion',1,'source','DOCUMENTS_PERIODIC_ENROLLMENT_WITHDRAWAL_V1','kind','PERIODIC_ENROLLMENT_WITHDRAWN','id',target,'consentId',c.id,'consentDigest',c.digest,'withdrawnAt',stamp_text,'attributionDigest',encode(sha256(convert_to(core.periodic_policy_canonical(jsonb_build_object('id',target,'personId',p_person,'actor',p_actor)),'UTF8')),'hex'));
   INSERT INTO documents.periodic_enrollment_withdrawals VALUES(target,c.id,p_person,p_actor,doc,encode(sha256(convert_to(core.periodic_policy_canonical(doc),'UTF8')),'hex'),stamp);
   created:=true;
  END IF;
 END IF;
 -- Redacted events share consent aggregate identity, with withdrawal version 2.
 IF created THEN
  event:=jsonb_build_object('eventId',target,'eventType',CASE WHEN p_action='ENROLL' THEN 'periodic.enrollment.requested' ELSE 'periodic.enrollment.withdrawn' END,'schemaVersion',1,'producer','documents','aggregateId',c.id,'aggregateVersion',CASE WHEN p_action='ENROLL' THEN 1 ELSE 2 END,'occurredAt',CASE WHEN p_action='ENROLL' THEN c.document->>'acceptedAt' ELSE doc->>'withdrawnAt' END,'correlationId',p_key,'causationId',p_key::text,'data',jsonb_build_object('consentId',c.id,'accountId',p_account,'offerId',p_offer,'status',CASE WHEN p_action='ENROLL' THEN 'PENDING_ACTIVATION' ELSE 'WITHDRAWN' END));
  INSERT INTO documents.outbox(event_id,aggregate_id,aggregate_version,document,fingerprint) VALUES(target,c.id,CASE WHEN p_action='ENROLL' THEN 1 ELSE 2 END,event,encode(sha256(convert_to(core.periodic_policy_canonical(event),'UTF8')),'hex'));
 END IF;
 INSERT INTO documents.periodic_enrollment_requests VALUES(p_actor->>'issuer',p_actor->>'subject',p_actor->>'clientId',p_key,p_person,p_account,p_offer,p_action,p_digest,c.id,target);
 RETURN documents.periodic_enrollment(p_account,p_offer,p_person);
END $$;
GRANT EXECUTE ON FUNCTION documents.request_periodic_enrollment(uuid,uuid,uuid,jsonb,uuid,text,text,boolean) TO bank_documents;
