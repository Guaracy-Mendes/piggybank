-- owners: controls (recovery, access review, security findings), documents/data (privacy requests).
-- WP-28 security, privacy, account recovery and audit lifecycle. Synthetic policy fixtures throughout.

ALTER TABLE controls.cases DROP CONSTRAINT cases_kind_check;
ALTER TABLE controls.cases ADD CONSTRAINT cases_kind_check CHECK(kind IN ('CONTROL','SUPPORT','PAYMENT','DATA','FINANCE','MONITORING','SECURITY'));

-- ===== Account recovery and device/factor changes (FR-IAM-06, SEC-03) =====
CREATE TABLE controls.recovery_requests(
 id uuid PRIMARY KEY,person_id uuid NOT NULL,issuer text NOT NULL,subject text NOT NULL,
 kind text NOT NULL CHECK(kind IN ('LOST_DEVICE','CREDENTIAL_RESET')),channel text NOT NULL CHECK(channel IN ('CUSTOMER_SELF','STAFF_ASSISTED')),
 requested_by uuid NOT NULL,actor_context jsonb NOT NULL,request_key uuid NOT NULL,reason text NOT NULL CHECK(length(reason) BETWEEN 3 AND 500),
 verification text NOT NULL CHECK(verification IN ('AUTHENTICATED_SESSION','VERIFIED_CONTACT','VERIFIED_EVIDENCE_REVIEW')),
 risk jsonb NOT NULL,state text NOT NULL CHECK(state IN ('PENDING_APPROVAL','APPROVED','EXECUTED','REJECTED')),
 decided_by uuid,decision_reason text,decided_at timestamptz,executed_at timestamptz,outcome jsonb,case_id uuid,created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 UNIQUE(requested_by,request_key),CHECK(decided_by IS NULL OR (decided_by<>requested_by AND decided_by<>person_id)),
 CHECK(channel<>'CUSTOMER_SELF' OR (kind='LOST_DEVICE' AND verification='AUTHENTICATED_SESSION' AND requested_by=person_id))
);
CREATE INDEX recovery_person ON controls.recovery_requests(person_id,created_at DESC);
CREATE TABLE controls.recovery_history(request_id uuid NOT NULL REFERENCES controls.recovery_requests(id),state text NOT NULL,actor_context jsonb,recorded_at timestamptz NOT NULL DEFAULT clock_timestamp());
CREATE TRIGGER immutable_recovery_history BEFORE UPDATE OR DELETE OR TRUNCATE ON controls.recovery_history FOR EACH STATEMENT EXECUTE FUNCTION controls.immutable_check_result();
CREATE FUNCTION controls.guard_recovery() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN
 IF TG_OP<>'UPDATE' OR (to_jsonb(NEW)-ARRAY['state','decided_by','decision_reason','decided_at','executed_at','outcome','case_id']) IS DISTINCT FROM (to_jsonb(OLD)-ARRAY['state','decided_by','decision_reason','decided_at','executed_at','outcome','case_id'])
  OR NOT ((OLD.state='PENDING_APPROVAL' AND NEW.state IN ('PENDING_APPROVAL','APPROVED','REJECTED')) OR (OLD.state='APPROVED' AND NEW.state IN ('APPROVED','EXECUTED')) OR (OLD.state=NEW.state AND OLD.state IN ('EXECUTED','REJECTED') AND to_jsonb(NEW)=to_jsonb(OLD)))
 THEN RAISE EXCEPTION 'Recovery identity and history are immutable'; END IF;
 IF NEW.state<>OLD.state THEN INSERT INTO controls.recovery_history VALUES(NEW.id,NEW.state,NULL,clock_timestamp()); END IF;
 RETURN NEW;END $$;
CREATE TRIGGER recovery_guard BEFORE UPDATE OR DELETE ON controls.recovery_requests FOR EACH ROW EXECUTE FUNCTION controls.guard_recovery();

-- Risk signals use durable facts only. Any signal makes the request HIGH risk.
CREATE FUNCTION controls.recovery_risk(p_person uuid) RETURNS jsonb LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,controls AS $$
DECLARE signals text[]:='{}';BEGIN
 IF EXISTS(SELECT 1 FROM core.contact_changes c JOIN core.parties p ON p.id=c.party_id JOIN controls.person_links l ON l.issuer=p.issuer AND l.subject=p.subject
   WHERE l.person_id=p_person AND c.created_at>clock_timestamp()-interval '7 days') THEN signals:=array_append(signals,'CONTACT_CHANGED_WITHIN_7_DAYS'); END IF;
 IF EXISTS(SELECT 1 FROM controls.monitoring_alerts a JOIN controls.cases c ON c.id=a.case_id JOIN core.account_relationships r ON r.account_id=a.account_id
   WHERE r.person_id=p_person AND r.kind='OWNER' AND c.state<>'CLOSED') THEN signals:=array_append(signals,'OPEN_MONITORING_ALERT'); END IF;
 IF EXISTS(SELECT 1 FROM controls.recovery_requests WHERE person_id=p_person AND state='EXECUTED' AND executed_at>clock_timestamp()-interval '30 days') THEN signals:=array_append(signals,'RECOVERY_WITHIN_30_DAYS'); END IF;
 RETURN jsonb_build_object('level',CASE WHEN cardinality(signals)=0 THEN 'LOW' ELSE 'HIGH' END,'signals',to_jsonb(signals),'policy','SYNTHETIC_RECOVERY_POLICY_V1','evaluatedAt',controls.utc_text(clock_timestamp()));
END $$;
CREATE FUNCTION controls.recovery_json(r controls.recovery_requests,p_staff boolean) RETURNS jsonb LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
 SELECT jsonb_build_object('id',r.id,'kind',r.kind,'channel',r.channel,'state',r.state,'createdAt',controls.utc_text(r.created_at),'executedAt',controls.utc_text(r.executed_at),
  'outcome',r.outcome)||CASE WHEN p_staff THEN jsonb_build_object('personId',r.person_id,'reason',r.reason,'verification',r.verification,'risk',r.risk,'requestedBy',r.requested_by,
  'decidedBy',r.decided_by,'decisionReason',r.decision_reason,'caseId',r.case_id) ELSE '{}'::jsonb END $$;
-- Customer self-service "lost device": protective, executed at once; HIGH risk still opens a reviewable case.
-- Staff-assisted recovery: ownership verification recorded; a credential reset or any HIGH risk needs an independent security approver.
CREATE FUNCTION controls.request_recovery(p_actor jsonb,p_person uuid,p_key uuid,p_data jsonb) RETURNS jsonb
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,controls AS $$
DECLARE r controls.recovery_requests;target uuid:=gen_random_uuid();customer uuid;link controls.person_links;risk jsonb;self boolean:=p_actor->>'trust'='CUSTOMER';state text;made uuid;
BEGIN
 IF p_key IS NULL OR jsonb_typeof(p_data)<>'object' OR EXISTS(SELECT 1 FROM jsonb_object_keys(p_data) k WHERE k NOT IN ('personId','kind','reason','verification')) THEN RAISE EXCEPTION 'Invalid recovery request' USING ERRCODE='22023'; END IF;
 SELECT * INTO r FROM controls.recovery_requests WHERE requested_by=p_person AND request_key=p_key;
 IF FOUND THEN
  IF r.kind<>p_data->>'kind' OR r.reason<>trim(p_data->>'reason') THEN RAISE EXCEPTION 'Recovery request key conflict'; END IF;
  RETURN controls.recovery_json(r,NOT self);
 END IF;
 IF self THEN
  PERFORM 1 FROM controls.person_links WHERE issuer=p_actor->>'issuer' AND subject=p_actor->>'subject' AND person_id=p_person;
  IF NOT FOUND THEN RAISE EXCEPTION 'Current canonical identity required'; END IF;
  IF p_data->>'kind'<>'LOST_DEVICE' OR p_data ? 'personId' THEN RAISE EXCEPTION 'Customers may only revoke their own devices' USING ERRCODE='22023'; END IF;
  customer:=p_person;
 ELSE
  PERFORM controls.require_staff(p_actor,p_person,ARRAY['account_support']);
  customer:=(p_data->>'personId')::uuid;
  IF customer IS NULL OR customer=p_person THEN RAISE EXCEPTION 'Recovery for another current customer required' USING ERRCODE='22023'; END IF;
  IF p_data->>'verification' NOT IN ('VERIFIED_CONTACT','VERIFIED_EVIDENCE_REVIEW') THEN RAISE EXCEPTION 'Recorded ownership verification required' USING ERRCODE='22023'; END IF;
 END IF;
 SELECT * INTO link FROM controls.person_links WHERE person_id=customer ORDER BY issuer LIMIT 1;
 IF NOT FOUND OR (NOT self AND NOT EXISTS(SELECT 1 FROM core.parties WHERE issuer=link.issuer AND subject=link.subject)) THEN RAISE EXCEPTION 'Customer identity unavailable' USING ERRCODE='P0002'; END IF;
 IF p_data->>'kind' NOT IN ('LOST_DEVICE','CREDENTIAL_RESET') OR length(trim(coalesce(p_data->>'reason',''))) NOT BETWEEN 3 AND 500 THEN RAISE EXCEPTION 'Supported recovery and reason required' USING ERRCODE='22023'; END IF;
 risk:=controls.recovery_risk(customer);
 state:=CASE WHEN self THEN 'APPROVED' WHEN p_data->>'kind'='CREDENTIAL_RESET' OR risk->>'level'='HIGH' THEN 'PENDING_APPROVAL' ELSE 'APPROVED' END;
 INSERT INTO controls.recovery_requests(id,person_id,issuer,subject,kind,channel,requested_by,actor_context,request_key,reason,verification,risk,state)
 VALUES(target,customer,link.issuer,link.subject,p_data->>'kind',CASE WHEN self THEN 'CUSTOMER_SELF' ELSE 'STAFF_ASSISTED' END,p_person,p_actor,p_key,trim(p_data->>'reason'),
  CASE WHEN self THEN 'AUTHENTICATED_SESSION' ELSE p_data->>'verification' END,risk,state) RETURNING * INTO r;
 INSERT INTO controls.recovery_history VALUES(target,state,p_actor,clock_timestamp());
 IF state='PENDING_APPROVAL' OR risk->>'level'='HIGH' THEN
  made:=controls.open_case('SECURITY',target,customer,'security-operations',CASE WHEN state='PENDING_APPROVAL' THEN 'INDEPENDENT_RECOVERY_APPROVAL' ELSE 'REVIEW_SUSPICIOUS_RECOVERY' END,'HIGH',p_actor);
  UPDATE controls.recovery_requests SET case_id=made WHERE id=target RETURNING * INTO r;
 END IF;
 PERFORM controls.append_audit(gen_random_uuid(),'recovery',jsonb_build_object('actorIssuer',p_actor->>'issuer','actorSubject',p_actor->>'subject','actorPersonId',p_person,'representedParty',NULL,
  'action','recovery.requested','resource',target::text,'outcome',state,'policyVersion','SYNTHETIC_RECOVERY_POLICY_V1','reasonCode',risk->>'level','correlationId',target,'occurredAt',controls.utc_text(clock_timestamp())));
 RETURN controls.recovery_json(r,NOT self);
END $$;
CREATE FUNCTION controls.decide_recovery(p_actor jsonb,p_person uuid,p_id uuid,p_approve boolean,p_reason text) RETURNS jsonb
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,controls AS $$
DECLARE r controls.recovery_requests;BEGIN
 PERFORM controls.require_staff(p_actor,p_person,ARRAY['security_approver']);
 IF length(trim(coalesce(p_reason,''))) NOT BETWEEN 3 AND 500 THEN RAISE EXCEPTION 'Decision reason required' USING ERRCODE='22023'; END IF;
 SELECT * INTO r FROM controls.recovery_requests WHERE id=p_id FOR UPDATE;IF NOT FOUND THEN RAISE EXCEPTION 'Recovery unavailable' USING ERRCODE='P0002'; END IF;
 IF r.requested_by=p_person OR r.person_id=p_person THEN RAISE EXCEPTION 'Independent approval required'; END IF;
 IF r.state<>'PENDING_APPROVAL' THEN
  IF r.decided_by=p_person THEN RETURN controls.recovery_json(r,true); END IF;
  RAISE EXCEPTION 'Recovery already decided';
 END IF;
 UPDATE controls.recovery_requests SET state=CASE WHEN p_approve THEN 'APPROVED' ELSE 'REJECTED' END,decided_by=p_person,decision_reason=trim(p_reason),decided_at=clock_timestamp() WHERE id=p_id RETURNING * INTO r;
 IF r.case_id IS NOT NULL THEN
  UPDATE controls.cases SET state='CLOSED',next_action=CASE WHEN p_approve THEN 'RECOVERY_APPROVED' ELSE 'RECOVERY_REJECTED' END,owner_person=coalesce(owner_person,p_person),version=version+1,updated_at=clock_timestamp() WHERE id=r.case_id;
  INSERT INTO controls.case_actions VALUES(gen_random_uuid(),r.case_id,p_actor->>'issuer',p_actor->>'subject','recovery.decided',CASE WHEN p_approve THEN 'APPROVED' ELSE 'REJECTED' END,clock_timestamp());
  PERFORM controls.case_changed(r.case_id,p_actor,'case.closed');
 END IF;
 PERFORM controls.append_audit(gen_random_uuid(),'recovery',jsonb_build_object('actorIssuer',p_actor->>'issuer','actorSubject',p_actor->>'subject','actorPersonId',p_person,'representedParty',NULL,
  'action','recovery.decided','resource',p_id::text,'outcome',r.state,'policyVersion','SYNTHETIC_RECOVERY_POLICY_V1','reasonCode','INDEPENDENT_DECISION','correlationId',p_id,'occurredAt',controls.utc_text(clock_timestamp())));
 RETURN controls.recovery_json(r,true);
END $$;
-- Called after issuer sessions are revoked (and, for a reset, the password-update required action is set).
CREATE FUNCTION controls.complete_recovery(p_id uuid,p_outcome jsonb) RETURNS jsonb
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,controls AS $$
DECLARE r controls.recovery_requests;BEGIN
 SELECT * INTO r FROM controls.recovery_requests WHERE id=p_id FOR UPDATE;IF NOT FOUND THEN RAISE EXCEPTION 'Recovery unavailable' USING ERRCODE='P0002'; END IF;
 IF r.state='EXECUTED' THEN RETURN controls.recovery_json(r,true); END IF;
 IF r.state<>'APPROVED' THEN RAISE EXCEPTION 'Only an approved recovery can be executed'; END IF;
 UPDATE controls.recovery_requests SET state='EXECUTED',executed_at=clock_timestamp(),outcome=p_outcome WHERE id=p_id RETURNING * INTO r;
 PERFORM data.queue_notification('RECOVERY:'||r.id,r.person_id,'SECURITY','RECOVERY_COMPLETED',jsonb_build_object('kind',r.kind),r.id);
 PERFORM controls.append_audit(gen_random_uuid(),'recovery',jsonb_build_object('actorIssuer','urn:piggybank:api','actorSubject','recovery-executor','actorPersonId',NULL,'representedParty',NULL,
  'action','recovery.executed','resource',p_id::text,'outcome','EXECUTED','policyVersion','SYNTHETIC_RECOVERY_POLICY_V1','reasonCode',r.kind,'correlationId',p_id,'occurredAt',controls.utc_text(clock_timestamp())));
 RETURN controls.recovery_json(r,true);
END $$;
CREATE FUNCTION controls.recovery_target(p_id uuid) RETURNS jsonb LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS
$$ SELECT jsonb_build_object('id',id,'issuer',issuer,'subject',subject,'kind',kind,'state',state) FROM controls.recovery_requests WHERE id=p_id $$;
CREATE FUNCTION controls.recovery_view(p_actor jsonb,p_person uuid) RETURNS jsonb
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,controls AS $$ BEGIN
 IF p_actor->>'trust'='CUSTOMER' THEN
  PERFORM 1 FROM controls.person_links WHERE issuer=p_actor->>'issuer' AND subject=p_actor->>'subject' AND person_id=p_person;
  IF NOT FOUND THEN RAISE EXCEPTION 'Current canonical identity required'; END IF;
  RETURN coalesce((SELECT jsonb_agg(controls.recovery_json(r,false) ORDER BY r.created_at DESC) FROM (SELECT * FROM controls.recovery_requests WHERE person_id=p_person ORDER BY created_at DESC LIMIT 20) r),'[]'::jsonb);
 END IF;
 PERFORM controls.require_staff(p_actor,p_person,ARRAY['account_support','security_approver','security_admin']);
 RETURN coalesce((SELECT jsonb_agg(controls.recovery_json(r,true) ORDER BY r.created_at DESC) FROM (SELECT * FROM controls.recovery_requests ORDER BY created_at DESC LIMIT 100) r),'[]'::jsonb);
END $$;

-- ===== Access recertification (FR-CTRL-06, SEC-10) =====
CREATE TABLE controls.access_reviews(id uuid PRIMARY KEY,opened_by uuid NOT NULL,actor_context jsonb NOT NULL,opened_at timestamptz NOT NULL DEFAULT clock_timestamp(),closed_at timestamptz);
CREATE TABLE controls.access_review_items(
 review_id uuid NOT NULL REFERENCES controls.access_reviews(id),grant_id uuid NOT NULL,person_id uuid NOT NULL,role text NOT NULL,valid_until timestamptz NOT NULL,
 decision text CHECK(decision IN ('KEEP','REVOKE')),reason text,decided_by uuid,decided_at timestamptz,PRIMARY KEY(review_id,grant_id),
 CHECK((decision IS NULL)=(decided_by IS NULL)),CHECK(decided_by IS NULL OR decided_by<>person_id)
);
CREATE FUNCTION controls.guard_access_item() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN
 IF TG_OP<>'UPDATE' OR OLD.decision IS NOT NULL OR (to_jsonb(NEW)-ARRAY['decision','reason','decided_by','decided_at']) IS DISTINCT FROM (to_jsonb(OLD)-ARRAY['decision','reason','decided_by','decided_at']) THEN RAISE EXCEPTION 'Access review decisions are final'; END IF;RETURN NEW;END $$;
CREATE TRIGGER access_item_guard BEFORE UPDATE OR DELETE ON controls.access_review_items FOR EACH ROW EXECUTE FUNCTION controls.guard_access_item();
CREATE FUNCTION controls.open_access_review(p_actor jsonb,p_person uuid) RETURNS jsonb
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,controls AS $$
DECLARE target uuid:=gen_random_uuid();BEGIN
 PERFORM controls.require_staff(p_actor,p_person,ARRAY['security_admin']);
 IF EXISTS(SELECT 1 FROM controls.access_reviews WHERE closed_at IS NULL) THEN RAISE EXCEPTION 'An access review is already open'; END IF;
 INSERT INTO controls.access_reviews VALUES(target,p_person,p_actor,clock_timestamp(),NULL);
 INSERT INTO controls.access_review_items(review_id,grant_id,person_id,role,valid_until)
 SELECT target,id,person_id,role,valid_until FROM controls.role_grants WHERE revoked_at IS NULL AND valid_until>clock_timestamp() AND valid_from<=clock_timestamp();
 PERFORM controls.append_audit(gen_random_uuid(),'access-review',jsonb_build_object('actorIssuer',p_actor->>'issuer','actorSubject',p_actor->>'subject','actorPersonId',p_person,'representedParty',NULL,
  'action','access-review.opened','resource',target::text,'outcome','RECORDED','policyVersion','synthetic-v1','reasonCode','RECERTIFICATION','correlationId',target,'occurredAt',controls.utc_text(clock_timestamp())));
 RETURN controls.access_review_view(p_actor,p_person);
END $$;
CREATE FUNCTION controls.decide_access_item(p_actor jsonb,p_person uuid,p_review uuid,p_grant uuid,p_decision text,p_reason text) RETURNS jsonb
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,controls AS $$
DECLARE i controls.access_review_items;BEGIN
 PERFORM controls.require_staff(p_actor,p_person,ARRAY['security_approver']);
 IF p_decision NOT IN ('KEEP','REVOKE') OR length(trim(coalesce(p_reason,''))) NOT BETWEEN 3 AND 500 THEN RAISE EXCEPTION 'Decision and reason required' USING ERRCODE='22023'; END IF;
 SELECT * INTO i FROM controls.access_review_items WHERE review_id=p_review AND grant_id=p_grant FOR UPDATE;
 IF NOT FOUND OR EXISTS(SELECT 1 FROM controls.access_reviews WHERE id=p_review AND closed_at IS NOT NULL) THEN RAISE EXCEPTION 'Open review item unavailable' USING ERRCODE='P0002'; END IF;
 IF i.person_id=p_person THEN RAISE EXCEPTION 'Reviewers cannot recertify their own access'; END IF;
 IF i.decision IS NOT NULL THEN
  IF i.decided_by=p_person AND i.decision=p_decision THEN RETURN controls.access_review_view(p_actor,p_person); END IF;
  RAISE EXCEPTION 'Item already decided';
 END IF;
 UPDATE controls.access_review_items SET decision=p_decision,reason=trim(p_reason),decided_by=p_person,decided_at=clock_timestamp() WHERE review_id=p_review AND grant_id=p_grant;
 IF p_decision='REVOKE' THEN UPDATE controls.role_grants SET revoked_at=clock_timestamp() WHERE id=p_grant AND revoked_at IS NULL; END IF;
 PERFORM controls.append_audit(gen_random_uuid(),'access-review',jsonb_build_object('actorIssuer',p_actor->>'issuer','actorSubject',p_actor->>'subject','actorPersonId',p_person,'representedParty',NULL,
  'action','access-review.'||lower(p_decision),'resource',p_grant::text,'outcome','RECORDED','policyVersion','synthetic-v1','reasonCode','RECERTIFICATION','correlationId',p_review,'occurredAt',controls.utc_text(clock_timestamp())));
 IF NOT EXISTS(SELECT 1 FROM controls.access_review_items WHERE review_id=p_review AND decision IS NULL) THEN UPDATE controls.access_reviews SET closed_at=clock_timestamp() WHERE id=p_review; END IF;
 RETURN controls.access_review_view(p_actor,p_person);
END $$;
CREATE FUNCTION controls.access_review_view(p_actor jsonb,p_person uuid) RETURNS jsonb
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,controls AS $$
DECLARE r controls.access_reviews;BEGIN
 PERFORM controls.require_staff(p_actor,p_person,ARRAY['security_admin','security_approver']);
 SELECT * INTO r FROM controls.access_reviews ORDER BY opened_at DESC LIMIT 1;
 IF NOT FOUND THEN RETURN jsonb_build_object('review',NULL); END IF;
 RETURN jsonb_build_object('review',jsonb_build_object('id',r.id,'openedBy',r.opened_by,'openedAt',controls.utc_text(r.opened_at),'closedAt',controls.utc_text(r.closed_at),
  'items',coalesce((SELECT jsonb_agg(jsonb_build_object('grantId',i.grant_id,'personId',i.person_id,'role',i.role,'validUntil',controls.utc_text(i.valid_until),'decision',i.decision,'reason',i.reason,
    'decidedBy',i.decided_by,'decidedAt',controls.utc_text(i.decided_at),'self',i.person_id=p_person) ORDER BY i.role,i.person_id) FROM controls.access_review_items i WHERE i.review_id=r.id),'[]'::jsonb)));
END $$;

-- ===== Security findings register (SEC-07, SEC-10) =====
CREATE TABLE controls.security_findings(
 id uuid PRIMARY KEY,reference text NOT NULL UNIQUE CHECK(reference ~ '^SF-[0-9]{3,6}$'),title text NOT NULL CHECK(length(title) BETWEEN 3 AND 200),
 severity text NOT NULL CHECK(severity IN ('LOW','MEDIUM','HIGH','CRITICAL')),area text NOT NULL CHECK(length(area) BETWEEN 2 AND 80),description text NOT NULL CHECK(length(description) BETWEEN 3 AND 2000),
 reported_by uuid NOT NULL,reported_at timestamptz NOT NULL DEFAULT clock_timestamp(),status text NOT NULL DEFAULT 'OPEN' CHECK(status IN ('OPEN','RESOLVED','ACCEPTED_RISK')),
 disposition text,evidence_ref text,disposed_by uuid,disposed_at timestamptz,
 CHECK((status='OPEN')=(disposed_by IS NULL)),CHECK(disposed_by IS NULL OR disposed_by<>reported_by),CHECK(NOT (status='ACCEPTED_RISK' AND severity IN ('HIGH','CRITICAL')))
);
CREATE FUNCTION controls.guard_finding() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN
 IF TG_OP<>'UPDATE' OR OLD.status<>'OPEN' OR (to_jsonb(NEW)-ARRAY['status','disposition','evidence_ref','disposed_by','disposed_at']) IS DISTINCT FROM (to_jsonb(OLD)-ARRAY['status','disposition','evidence_ref','disposed_by','disposed_at']) THEN RAISE EXCEPTION 'Finding dispositions are final'; END IF;RETURN NEW;END $$;
CREATE TRIGGER finding_guard BEFORE UPDATE OR DELETE ON controls.security_findings FOR EACH ROW EXECUTE FUNCTION controls.guard_finding();
CREATE FUNCTION controls.report_finding(p_actor jsonb,p_person uuid,p_data jsonb) RETURNS jsonb
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,controls AS $$
DECLARE target uuid:=gen_random_uuid();BEGIN
 PERFORM controls.require_staff(p_actor,p_person,ARRAY['security_admin']);
 IF jsonb_typeof(p_data)<>'object' OR EXISTS(SELECT 1 FROM jsonb_object_keys(p_data) k WHERE k NOT IN ('reference','title','severity','area','description')) THEN RAISE EXCEPTION 'Invalid finding' USING ERRCODE='22023'; END IF;
 INSERT INTO controls.security_findings(id,reference,title,severity,area,description,reported_by) VALUES(target,p_data->>'reference',trim(p_data->>'title'),p_data->>'severity',trim(p_data->>'area'),trim(p_data->>'description'),p_person);
 PERFORM controls.append_audit(gen_random_uuid(),'security-findings',jsonb_build_object('actorIssuer',p_actor->>'issuer','actorSubject',p_actor->>'subject','actorPersonId',p_person,'representedParty',NULL,
  'action','finding.reported','resource',target::text,'outcome',p_data->>'severity','policyVersion','synthetic-v1','reasonCode','SECURITY_FINDING','correlationId',target,'occurredAt',controls.utc_text(clock_timestamp())));
 RETURN controls.findings_view(p_actor,p_person);
END $$;
CREATE FUNCTION controls.dispose_finding(p_actor jsonb,p_person uuid,p_id uuid,p_status text,p_disposition text,p_evidence text) RETURNS jsonb
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,controls AS $$
DECLARE f controls.security_findings;BEGIN
 PERFORM controls.require_staff(p_actor,p_person,ARRAY['security_approver']);
 SELECT * INTO f FROM controls.security_findings WHERE id=p_id FOR UPDATE;IF NOT FOUND THEN RAISE EXCEPTION 'Finding unavailable' USING ERRCODE='P0002'; END IF;
 IF f.reported_by=p_person THEN RAISE EXCEPTION 'Independent disposition required'; END IF;
 IF p_status NOT IN ('RESOLVED','ACCEPTED_RISK') OR length(trim(coalesce(p_disposition,''))) NOT BETWEEN 3 AND 2000 THEN RAISE EXCEPTION 'Disposition required' USING ERRCODE='22023'; END IF;
 IF p_status='ACCEPTED_RISK' AND f.severity IN ('HIGH','CRITICAL') THEN RAISE EXCEPTION 'Release-blocking findings must be resolved'; END IF;
 UPDATE controls.security_findings SET status=p_status,disposition=trim(p_disposition),evidence_ref=nullif(trim(coalesce(p_evidence,'')),''),disposed_by=p_person,disposed_at=clock_timestamp() WHERE id=p_id;
 PERFORM controls.append_audit(gen_random_uuid(),'security-findings',jsonb_build_object('actorIssuer',p_actor->>'issuer','actorSubject',p_actor->>'subject','actorPersonId',p_person,'representedParty',NULL,
  'action','finding.disposed','resource',p_id::text,'outcome',p_status,'policyVersion','synthetic-v1','reasonCode','SECURITY_FINDING','correlationId',p_id,'occurredAt',controls.utc_text(clock_timestamp())));
 RETURN controls.findings_view(p_actor,p_person);
END $$;
CREATE FUNCTION controls.findings_view(p_actor jsonb,p_person uuid) RETURNS jsonb
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,controls AS $$ BEGIN
 PERFORM controls.require_staff(p_actor,p_person,ARRAY['security_admin','security_approver']);
 RETURN jsonb_build_object(
  'releaseBlocking',(SELECT count(*) FROM controls.security_findings WHERE status='OPEN' AND severity IN ('HIGH','CRITICAL')),
  'findings',coalesce((SELECT jsonb_agg(jsonb_build_object('id',id,'reference',reference,'title',title,'severity',severity,'area',area,'description',description,'status',status,
    'reportedBy',reported_by,'reportedAt',controls.utc_text(reported_at),'disposition',disposition,'evidenceRef',evidence_ref,'disposedBy',disposed_by,'disposedAt',controls.utc_text(disposed_at)) ORDER BY reference) FROM controls.security_findings),'[]'::jsonb));
END $$;

-- ===== Privacy requests: restriction and personal-data export (SEC-05, AT-16) =====
ALTER TABLE documents.data_requests DROP CONSTRAINT data_requests_kind_check;
ALTER TABLE documents.data_requests ADD CONSTRAINT data_requests_kind_check CHECK(kind IN ('ACCESS','CORRECTION','DELETION','RESTRICTION','EXPORT'));
CREATE TABLE data.processing_restrictions(person_id uuid NOT NULL,purpose text NOT NULL CHECK(purpose IN ('MARKETING','ANALYTICS')),request_id uuid NOT NULL,applied_at timestamptz NOT NULL DEFAULT clock_timestamp(),PRIMARY KEY(person_id,purpose));
CREATE TRIGGER immutable_processing_restrictions BEFORE UPDATE OR DELETE OR TRUNCATE ON data.processing_restrictions FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal();
REVOKE ALL ON data.processing_restrictions FROM bank_data;
-- A restricted purpose is never enabled again by preference alone.
CREATE OR REPLACE FUNCTION data.purpose_enabled(p_person uuid,p_purpose text) RETURNS boolean LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS
$$ SELECT CASE WHEN p.mandatory THEN true WHEN EXISTS(SELECT 1 FROM data.processing_restrictions r WHERE r.person_id=p_person AND r.purpose=p_purpose) THEN false
  ELSE coalesce((SELECT enabled FROM data.notification_preferences WHERE person_id=p_person AND purpose=p_purpose),p.default_enabled) END FROM data.notification_purposes p WHERE p.code=p_purpose $$;
-- Own data only: profile from the person's applications, verified contact, preferences, consent records, statements and evidence metadata.
CREATE FUNCTION documents.personal_data_export(p_request uuid,p_person uuid) RETURNS jsonb LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,documents AS $$
DECLARE links jsonb;BEGIN
 SELECT coalesce(jsonb_agg(jsonb_build_object('issuer',issuer,'subject',subject)),'[]'::jsonb) INTO links FROM controls.person_links WHERE person_id=p_person;
 RETURN jsonb_build_object('formatVersion',1,'source','DOCUMENTS_PERSONAL_DATA_EXPORT_V1','label','SYNTHETIC_TST','requestId',p_request,'generatedAt',documents.utc_text(clock_timestamp()),
  'applications',coalesce((SELECT jsonb_agg(jsonb_build_object('id',a.id,'state',a.state,'profile',a.profile,'marketing',a.marketing,'consentVersion',a.consent_version,'createdAt',documents.utc_text(a.created_at)) ORDER BY a.created_at)
    FROM journeys.applications a JOIN controls.person_links l ON l.issuer=a.issuer AND l.subject=a.subject WHERE l.person_id=p_person),'[]'::jsonb),
  'contacts',coalesce((SELECT jsonb_agg(jsonb_build_object('email',c.email,'version',c.version,'verifiedAt',documents.utc_text(c.verified_at)))
    FROM core.contacts c JOIN core.parties p ON p.id=c.party_id JOIN controls.person_links l ON l.issuer=p.issuer AND l.subject=p.subject WHERE l.person_id=p_person),'[]'::jsonb),
  'termsAcceptances',coalesce((SELECT jsonb_agg(jsonb_build_object('termsId',x.terms_id,'productVersionId',x.product_version_id,'channel',x.channel,'digest',x.digest,'acceptedAt',documents.utc_text(x.accepted_at)) ORDER BY x.accepted_at)
    FROM documents.acceptances x JOIN controls.person_links l ON l.issuer=x.issuer AND l.subject=x.subject WHERE l.person_id=p_person),'[]'::jsonb),
  'notificationPreferences',coalesce((SELECT jsonb_agg(jsonb_build_object('purpose',purpose,'enabled',enabled,'updatedAt',documents.utc_text(updated_at))) FROM data.notification_preferences WHERE person_id=p_person),'[]'::jsonb),
  'processingRestrictions',coalesce((SELECT jsonb_agg(jsonb_build_object('purpose',purpose,'appliedAt',documents.utc_text(applied_at))) FROM data.processing_restrictions WHERE person_id=p_person),'[]'::jsonb),
  'statements',coalesce((SELECT jsonb_agg(jsonb_build_object('versionId',v.id,'accountId',v.account_id,'periodStart',v.period_start,'version',v.version,'contentDigest',v.content_digest) ORDER BY v.account_id,v.period_start,v.version)
    FROM documents.statement_versions v WHERE p_person=ANY(v.owners) AND NOT EXISTS(SELECT 1 FROM documents.statement_disposals d WHERE d.version_id=v.id)),'[]'::jsonb),
  'identityEvidence',jsonb_build_object('status','RESTRICTED_EVIDENCE','count',(SELECT count(*) FROM controls.evidence_objects e JOIN controls.person_links l ON l.issuer=e.owner_issuer AND l.subject=e.owner_subject WHERE l.person_id=p_person),
    'note','Identity evidence is retained and released only through a supervised support review.'));
END $$;
CREATE OR REPLACE FUNCTION documents.process_data_request(p_actor jsonb,p_person uuid,p_request uuid) RETURNS jsonb
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,documents AS $$
DECLARE r documents.data_requests;v documents.statement_versions;items jsonb:='[]';item jsonb;status text;allowed integer:=0;restricted integer:=0;outcome text;
 export jsonb;policy documents.retention_policies;stamp timestamptz:=clock_timestamp();restricted_purpose text;
BEGIN
 PERFORM documents.require_records_actor(p_actor,p_person,ARRAY['records_officer']);
 PERFORM pg_advisory_xact_lock(hashtextextended('documents.data-request:'||p_request,0));
 SELECT * INTO r FROM documents.data_requests WHERE id=p_request;
 IF NOT FOUND THEN RAISE EXCEPTION 'Data request unavailable' USING ERRCODE='P0002'; END IF;
 IF r.person_id=p_person THEN RAISE EXCEPTION 'Independent processing required'; END IF;
 IF EXISTS(SELECT 1 FROM documents.data_request_outcomes WHERE request_id=r.id) THEN RETURN documents.data_request_json(r,true); END IF;
 IF r.kind IN ('ACCESS','CORRECTION','DELETION') THEN
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
 ELSIF r.kind='RESTRICTION' THEN
  FOREACH restricted_purpose IN ARRAY ARRAY['MARKETING','ANALYTICS'] LOOP
   INSERT INTO data.processing_restrictions VALUES(r.person_id,restricted_purpose,r.id,stamp) ON CONFLICT DO NOTHING;
   items:=items||jsonb_build_object('recordClass',restricted_purpose||'_USE','status','APPLIED');allowed:=allowed+1;
  END LOOP;
  INSERT INTO data.notification_preferences VALUES(r.person_id,'MARKETING',false,p_actor,stamp) ON CONFLICT(person_id,purpose) DO UPDATE SET enabled=false,actor_context=EXCLUDED.actor_context,updated_at=EXCLUDED.updated_at;
  INSERT INTO data.notification_preference_history(person_id,purpose,enabled,actor_context) VALUES(r.person_id,'MARKETING',false,p_actor);
  items:=items||jsonb_build_object('recordClass','SERVICE_AND_SECURITY_NOTICES','status','RESTRICTED_LEGAL_BASIS','note','Security and service notices are required to operate the account.');
  items:=items||jsonb_build_object('recordClass','CORE_JOURNAL','status','RESTRICTED_RETENTION','note','Financial records continue to be processed to provide the account.');
  restricted:=restricted+2;
 ELSE
  items:=jsonb_build_array(jsonb_build_object('recordClass','PERSONAL_DATA','status','INCLUDED'),
   jsonb_build_object('recordClass','IDENTITY_EVIDENCE','status','RESTRICTED_EVIDENCE','note','Released only through a supervised support review.'));
  allowed:=1;restricted:=1;
 END IF;
 outcome:=CASE WHEN restricted=0 THEN 'ALLOWED' WHEN allowed=0 THEN 'RESTRICTED' ELSE 'PARTIALLY_RESTRICTED' END;
 INSERT INTO documents.data_request_outcomes VALUES(r.id,p_person,p_actor,outcome,items,documents.record_digest(jsonb_build_object('requestId',r.id,'outcome',outcome,'items',items)),stamp);
 IF r.kind IN ('ACCESS','EXPORT') THEN
  SELECT * INTO STRICT policy FROM documents.retention_policies WHERE record_class='DATA_EXPORT' AND effective_from<=documents.records_today() ORDER BY version DESC LIMIT 1;
  IF r.kind='ACCESS' THEN
   export:=jsonb_build_object('formatVersion',1,'source','DOCUMENTS_DATA_EXPORT_V1','label','SYNTHETIC_TST','requestId',r.id,'generatedAt',documents.utc_text(stamp),
    'statements',coalesce((SELECT jsonb_agg(x-'status' ORDER BY x->>'accountId',x->>'periodStart',(x->>'version')::int) FROM jsonb_array_elements(items) x WHERE x->>'status'='INCLUDED'),'[]'::jsonb),
    'retrieval','Each listed statement is retrieved separately through an authenticated single-use link.');
  ELSE
   export:=documents.personal_data_export(r.id,r.person_id);
  END IF;
  INSERT INTO documents.data_exports VALUES(r.id,r.person_id,export,documents.record_digest(export),policy.id,documents.records_today()+policy.retain_days,stamp);
 END IF;
 PERFORM documents.audit(p_actor,p_person,'DATA_REQUEST_PROCESSED','COMPLETED',r.kind||'_'||outcome);
 RETURN documents.data_request_json(r,true);
END $$;
CREATE OR REPLACE FUNCTION documents.submit_data_request(p_actor jsonb,p_person uuid,p_key uuid,p_kind text,p_detail text) RETURNS jsonb
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,documents AS $$
DECLARE r documents.data_requests;target uuid:=gen_random_uuid();BEGIN
 PERFORM documents.require_periodic_actor(p_actor,p_person,false);
 IF p_key IS NULL OR p_kind NOT IN ('ACCESS','CORRECTION','DELETION','RESTRICTION','EXPORT') OR length(coalesce(p_detail,''))>500 THEN RAISE EXCEPTION 'Invalid data request' USING ERRCODE='22023'; END IF;
 SELECT * INTO r FROM documents.data_requests WHERE issuer=p_actor->>'issuer' AND subject=p_actor->>'subject' AND client_id=p_actor->>'clientId' AND request_key=p_key;
 IF FOUND THEN
  IF r.kind<>p_kind OR r.detail<>coalesce(p_detail,'') THEN RAISE EXCEPTION 'Data request key conflict'; END IF;
  RETURN documents.data_request_json(r,false);
 END IF;
 INSERT INTO documents.data_requests VALUES(target,p_actor->>'issuer',p_actor->>'subject',p_actor->>'clientId',p_key,p_person,p_kind,coalesce(p_detail,''),'AUTHENTICATED_CUSTOMER_SESSION',p_actor,clock_timestamp()) RETURNING * INTO r;
 PERFORM documents.audit(p_actor,p_person,'DATA_REQUEST','CREATED',p_kind);
 RETURN documents.data_request_json(r,false);
END $$;

-- ===== Security notification template =====
ALTER TABLE data.notifications DROP CONSTRAINT notifications_template_check;
ALTER TABLE data.notifications ADD CONSTRAINT notifications_template_check CHECK(template IN ('STATEMENT_READY','DATA_REQUEST_COMPLETED','PAYMENT_SENT','DEVELOPER_TOKEN_CREATED','RECOVERY_COMPLETED'));
CREATE OR REPLACE FUNCTION data.render_notification(p_template text,p_params jsonb) RETURNS TABLE(subject text,body text) LANGUAGE sql IMMUTABLE SET search_path=pg_catalog AS $$
 SELECT CASE p_template WHEN 'STATEMENT_READY' THEN 'Your statement is ready' WHEN 'DATA_REQUEST_COMPLETED' THEN 'Your data request was processed'
   WHEN 'PAYMENT_SENT' THEN 'A payment left your account' WHEN 'RECOVERY_COMPLETED' THEN 'Your devices were signed out' ELSE 'A developer access token was created' END,
  CASE p_template WHEN 'STATEMENT_READY' THEN 'Your '||(p_params->>'month')||' statement for the account ending '||(p_params->>'accountEnding')||' is available. Sign in to Piggybank and open Statements & documents.'
   WHEN 'DATA_REQUEST_COMPLETED' THEN 'We finished processing your '||lower(p_params->>'kind')||' request. Sign in to Piggybank to see the outcome.'
   WHEN 'PAYMENT_SENT' THEN 'A payment from the account ending '||(p_params->>'accountEnding')||' was posted. Sign in to Piggybank to see the details. If you did not make it, contact support.'
   WHEN 'RECOVERY_COMPLETED' THEN CASE WHEN p_params->>'kind'='CREDENTIAL_RESET' THEN 'Every session was signed out and you must set a new password at your next sign-in. If you did not ask for this, contact support.'
     ELSE 'Every session on every device was signed out. Sign in again on the devices you use. If you did not ask for this, contact support.' END
   ELSE 'A read-only developer access token was created for your profile. If this was not you, sign in and revoke it.' END $$;

DO $$ DECLARE t text;BEGIN
 FOREACH t IN ARRAY ARRAY['recovery_requests','recovery_history','access_reviews','access_review_items','security_findings'] LOOP EXECUTE format('REVOKE ALL ON controls.%I FROM bank_controls',t); END LOOP;
END $$;
GRANT EXECUTE ON FUNCTION controls.request_recovery(jsonb,uuid,uuid,jsonb),controls.decide_recovery(jsonb,uuid,uuid,boolean,text),controls.complete_recovery(uuid,jsonb),
 controls.recovery_target(uuid),controls.recovery_view(jsonb,uuid),controls.open_access_review(jsonb,uuid),controls.decide_access_item(jsonb,uuid,uuid,uuid,text,text),
 controls.access_review_view(jsonb,uuid),controls.report_finding(jsonb,uuid,jsonb),controls.dispose_finding(jsonb,uuid,uuid,text,text,text),controls.findings_view(jsonb,uuid) TO bank_controls;
