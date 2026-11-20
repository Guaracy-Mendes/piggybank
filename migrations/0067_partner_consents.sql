-- owners: identity (partner clients, consents, quotas, callbacks, API versions), core (account delegation).
-- WP-33 consented partner APIs and delegated account authority (R2). Local synthetic only: partner clients are
-- loopback fixtures, and production enablement needs WP-32 plus actual partner contracts (DEC-09).

-- ===== Partner API versions (FR-EXT-02) =====
CREATE TABLE identity.partner_api_versions(
 version text PRIMARY KEY CHECK(version ~ '^[0-9]{4}-[0-9]{2}-[0-9]{2}$'),
 state text NOT NULL CHECK(state IN ('CURRENT','DEPRECATED','SUNSET')),
 sunset_at timestamptz,label text NOT NULL CHECK(label='SYNTHETIC_FIXTURE'),
 CHECK((state='CURRENT')=(sunset_at IS NULL))
);
INSERT INTO identity.partner_api_versions VALUES
 ('2026-11-01','CURRENT',NULL,'SYNTHETIC_FIXTURE'),
 ('2026-08-01','DEPRECATED','2027-02-01T00:00:00Z','SYNTHETIC_FIXTURE'),
 ('2026-05-01','SUNSET','2026-11-01T00:00:00Z','SYNTHETIC_FIXTURE');
CREATE TRIGGER immutable_partner_api_versions BEFORE UPDATE OR DELETE OR TRUNCATE ON identity.partner_api_versions FOR EACH STATEMENT EXECUTE FUNCTION controls.immutable_check_result();
CREATE FUNCTION identity.partner_api_version(p_version text) RETURNS jsonb LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
 SELECT jsonb_build_object('version',v.version,'state',CASE WHEN v.sunset_at IS NOT NULL AND v.sunset_at<=clock_timestamp() THEN 'SUNSET' ELSE v.state END,
  'sunsetAt',controls.utc_text(v.sunset_at),'current',(SELECT version FROM identity.partner_api_versions WHERE state='CURRENT'))
 FROM identity.partner_api_versions v WHERE v.version=p_version $$;

-- ===== Registered partner clients (FR-IAM-05) =====
CREATE TABLE identity.partner_clients(
 id uuid PRIMARY KEY,client_id text NOT NULL UNIQUE CHECK(client_id ~ '^partner-[a-z0-9-]{2,40}$'),
 display_name text NOT NULL CHECK(length(trim(display_name)) BETWEEN 2 AND 80),
 allowed_scopes text[] NOT NULL CHECK(cardinality(allowed_scopes)>0 AND allowed_scopes<@ARRAY['accounts:read','payments:initiate']),
 quota_per_minute integer NOT NULL CHECK(quota_per_minute BETWEEN 1 AND 100000),
 callback_url text CHECK(callback_url ~ '^http://127\.0\.0\.1:[0-9]{2,5}/[A-Za-z0-9/_-]{0,120}$'),
 callback_secret_sealed text CHECK((callback_url IS NULL)=(callback_secret_sealed IS NULL)),
 state text NOT NULL CHECK(state IN ('PENDING_APPROVAL','ACTIVE','SUSPENDED','REJECTED')),
 proposed_by uuid NOT NULL,request_key uuid NOT NULL,decided_by uuid,decision_reason text,version integer NOT NULL DEFAULT 1,
 created_at timestamptz NOT NULL DEFAULT clock_timestamp(),updated_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 UNIQUE(proposed_by,request_key)
);
CREATE TABLE identity.partner_client_history(client_ref uuid NOT NULL REFERENCES identity.partner_clients(id),state text NOT NULL,actor_person uuid,reason text,recorded_at timestamptz NOT NULL DEFAULT clock_timestamp());
CREATE TRIGGER immutable_partner_client_history BEFORE UPDATE OR DELETE OR TRUNCATE ON identity.partner_client_history FOR EACH STATEMENT EXECUTE FUNCTION controls.immutable_check_result();
CREATE FUNCTION identity.guard_partner_client() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN
 IF TG_OP<>'UPDATE' OR (to_jsonb(NEW)-ARRAY['state','decided_by','decision_reason','version','updated_at']) IS DISTINCT FROM (to_jsonb(OLD)-ARRAY['state','decided_by','decision_reason','version','updated_at'])
  OR NOT ((OLD.state='PENDING_APPROVAL' AND NEW.state IN ('ACTIVE','REJECTED')) OR (OLD.state='ACTIVE' AND NEW.state='SUSPENDED') OR (OLD.state='SUSPENDED' AND NEW.state='PENDING_APPROVAL'))
 THEN RAISE EXCEPTION 'Partner client identity and history are immutable'; END IF;
 RETURN NEW;END $$;
CREATE TRIGGER partner_client_guard BEFORE UPDATE OR DELETE ON identity.partner_clients FOR EACH ROW EXECUTE FUNCTION identity.guard_partner_client();
-- A security administrator proposes; a different security administrator or approver decides. Suspension is protective and immediate.
CREATE FUNCTION identity.partner_propose(p_actor jsonb,p_person uuid,p_key uuid,p_data jsonb,p_sealed_secret text) RETURNS jsonb
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,identity AS $$
DECLARE c identity.partner_clients;scopes text[];BEGIN
 PERFORM controls.require_staff(p_actor,p_person,ARRAY['security_admin']);
 IF p_key IS NULL OR jsonb_typeof(p_data)<>'object' OR EXISTS(SELECT 1 FROM jsonb_object_keys(p_data) k WHERE k NOT IN ('clientId','displayName','allowedScopes','quotaPerMinute','callbackUrl'))
   OR jsonb_typeof(p_data->'allowedScopes')<>'array' THEN RAISE EXCEPTION 'Invalid partner registration' USING ERRCODE='22023'; END IF;
 SELECT * INTO c FROM identity.partner_clients WHERE proposed_by=p_person AND request_key=p_key;
 IF FOUND THEN
  IF c.client_id<>p_data->>'clientId' THEN RAISE EXCEPTION 'Partner registration key conflict'; END IF;
  RETURN identity.partner_client_json(c);
 END IF;
 SELECT array_agg(DISTINCT s ORDER BY s) INTO scopes FROM jsonb_array_elements_text(p_data->'allowedScopes') s;
 INSERT INTO identity.partner_clients(id,client_id,display_name,allowed_scopes,quota_per_minute,callback_url,callback_secret_sealed,state,proposed_by,request_key)
  VALUES(gen_random_uuid(),p_data->>'clientId',trim(p_data->>'displayName'),scopes,(p_data->>'quotaPerMinute')::integer,NULLIF(p_data->>'callbackUrl',''),
   CASE WHEN NULLIF(p_data->>'callbackUrl','') IS NULL THEN NULL ELSE p_sealed_secret END,'PENDING_APPROVAL',p_person,p_key) RETURNING * INTO c;
 INSERT INTO identity.partner_client_history VALUES(c.id,c.state,p_person,'PROPOSED',clock_timestamp());
 PERFORM controls.append_audit(gen_random_uuid(),'partners',jsonb_build_object('actorIssuer',p_actor->>'issuer','actorSubject',p_actor->>'subject','actorPersonId',p_person,
  'representedParty',NULL,'action','partner.proposed','resource',c.id::text,'outcome','ALLOWED','policyVersion','PARTNER_POLICY_V1','reasonCode','PROPOSED','correlationId',c.id,
  'occurredAt',controls.utc_text(clock_timestamp())));
 RETURN identity.partner_client_json(c);
END $$;
CREATE FUNCTION identity.partner_decide(p_actor jsonb,p_person uuid,p_id uuid,p_decision text,p_reason text) RETURNS jsonb
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,identity AS $$
DECLARE c identity.partner_clients;target text;BEGIN
 PERFORM controls.require_staff(p_actor,p_person,ARRAY['security_admin','security_approver']);
 IF p_decision NOT IN ('APPROVE','REJECT','SUSPEND','REINSTATE') OR length(trim(coalesce(p_reason,''))) NOT BETWEEN 3 AND 300 THEN RAISE EXCEPTION 'Invalid partner decision' USING ERRCODE='22023'; END IF;
 SELECT * INTO c FROM identity.partner_clients WHERE id=p_id FOR UPDATE;
 IF NOT FOUND THEN RAISE EXCEPTION 'Partner client not found' USING ERRCODE='P0002'; END IF;
 target:=CASE p_decision WHEN 'APPROVE' THEN 'ACTIVE' WHEN 'REJECT' THEN 'REJECTED' WHEN 'SUSPEND' THEN 'SUSPENDED' ELSE 'PENDING_APPROVAL' END;
 IF c.state=target THEN RETURN identity.partner_client_json(c); END IF;
 IF p_decision IN ('APPROVE','REJECT') AND (c.state<>'PENDING_APPROVAL' OR p_person=(SELECT h.actor_person FROM identity.partner_client_history h WHERE h.client_ref=c.id AND h.state='PENDING_APPROVAL' ORDER BY h.recorded_at DESC LIMIT 1))
  THEN RAISE EXCEPTION 'Independent approver required for a pending registration'; END IF;
 IF p_decision='SUSPEND' AND c.state<>'ACTIVE' THEN RAISE EXCEPTION 'Only an active partner can be suspended'; END IF;
 IF p_decision='REINSTATE' AND c.state<>'SUSPENDED' THEN RAISE EXCEPTION 'Only a suspended partner can be reinstated'; END IF;
 -- Reinstatement goes back to approval; whoever requested it cannot also approve it.
 IF p_decision='REINSTATE' THEN
  UPDATE identity.partner_clients SET state=target,decided_by=NULL,decision_reason=trim(p_reason),version=version+1,updated_at=clock_timestamp() WHERE id=p_id RETURNING * INTO c;
 ELSE
  UPDATE identity.partner_clients SET state=target,decided_by=p_person,decision_reason=trim(p_reason),version=version+1,updated_at=clock_timestamp() WHERE id=p_id RETURNING * INTO c;
 END IF;
 INSERT INTO identity.partner_client_history VALUES(c.id,c.state,p_person,trim(p_reason),clock_timestamp());
 PERFORM controls.append_audit(gen_random_uuid(),'partners',jsonb_build_object('actorIssuer',p_actor->>'issuer','actorSubject',p_actor->>'subject','actorPersonId',p_person,
  'representedParty',NULL,'action','partner.'||lower(p_decision),'resource',c.id::text,'outcome','ALLOWED','policyVersion','PARTNER_POLICY_V1','reasonCode',p_decision,'correlationId',c.id,
  'occurredAt',controls.utc_text(clock_timestamp())));
 RETURN identity.partner_client_json(c);
END $$;
CREATE FUNCTION identity.partners_view(p_actor jsonb,p_person uuid) RETURNS jsonb
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,identity AS $$ BEGIN
 PERFORM controls.require_staff(p_actor,p_person,ARRAY['security_admin','security_approver','reviewer','case_approver']);
 RETURN jsonb_build_object(
  'clients',coalesce((SELECT jsonb_agg(identity.partner_client_json(c) ORDER BY c.created_at DESC) FROM identity.partner_clients c),'[]'::jsonb),
  'consents',coalesce((SELECT jsonb_agg(identity.consent_json(k,true) ORDER BY k.created_at DESC) FROM (SELECT * FROM identity.consents ORDER BY created_at DESC LIMIT 100) k),'[]'::jsonb),
  'callbacks',coalesce((SELECT jsonb_agg(jsonb_build_object('id',b.id,'clientId',c.client_id,'consentId',b.consent_id,'eventType',b.event_type,'state',b.state,'attempts',b.attempts,
    'lastStatus',b.last_status,'createdAt',controls.utc_text(b.created_at),'deliveredAt',controls.utc_text(b.delivered_at)) ORDER BY b.created_at DESC)
    FROM (SELECT * FROM identity.partner_callbacks ORDER BY created_at DESC LIMIT 50) b JOIN identity.partner_clients c ON c.id=b.client_ref),'[]'::jsonb));
END $$;
-- The API accepts a partner token only for a registered, active client (azp), never a client named only by the token.
CREATE FUNCTION identity.partner_client_active(p_client text) RETURNS jsonb LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
 SELECT jsonb_build_object('id',c.id,'clientId',c.client_id,'displayName',c.display_name,'allowedScopes',to_jsonb(c.allowed_scopes),'quotaPerMinute',c.quota_per_minute)
 FROM identity.partner_clients c WHERE c.client_id=p_client AND c.state='ACTIVE' $$;

-- Per-client quota: one statement per request; async commit keeps the hot row lock short (advisory counter).
CREATE TABLE identity.partner_quota(client_id text NOT NULL,window_start timestamptz NOT NULL,requests integer NOT NULL,PRIMARY KEY(client_id,window_start));
CREATE FUNCTION identity.partner_quota_permits(p_client text,p_limit integer) RETURNS boolean LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,identity AS $$
DECLARE taken integer;BEGIN
 PERFORM set_config('synchronous_commit','off',true);
 INSERT INTO identity.partner_quota VALUES(p_client,date_trunc('minute',clock_timestamp()),1)
  ON CONFLICT(client_id,window_start) DO UPDATE SET requests=identity.partner_quota.requests+1 WHERE identity.partner_quota.requests<p_limit RETURNING requests INTO taken;
 DELETE FROM identity.partner_quota WHERE client_id=p_client AND window_start<date_trunc('minute',clock_timestamp())-interval '10 minutes';
 RETURN taken IS NOT NULL;
END $$;

-- ===== Customer consents (FR-IAM-05, FR-EXT-02) =====
CREATE TABLE identity.consents(
 id uuid PRIMARY KEY,client_ref uuid NOT NULL REFERENCES identity.partner_clients(id),client_id text NOT NULL,request_key uuid NOT NULL,
 scopes text[] NOT NULL CHECK(cardinality(scopes)>0 AND scopes<@ARRAY['accounts:read','payments:initiate']),
 duration_days integer NOT NULL CHECK(duration_days BETWEEN 1 AND 90),
 state text NOT NULL CHECK(state IN ('AWAITING_AUTHORIZATION','ACTIVE','REJECTED','REVOKED')),
 person_id uuid,issuer text,subject text,account_ids uuid[],
 authorization_deadline timestamptz NOT NULL,authorized_at timestamptz,expires_at timestamptz,
 revoked_at timestamptz,revoked_by text CHECK(revoked_by IN ('CUSTOMER','STAFF','PARTNER')),revoked_by_person uuid,revoke_reason text,
 version integer NOT NULL DEFAULT 1,created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 UNIQUE(client_ref,request_key),
 CHECK(state='AWAITING_AUTHORIZATION' OR state='REJECTED' OR (person_id IS NOT NULL AND cardinality(account_ids)>0 AND expires_at IS NOT NULL)),
 CHECK((state='REVOKED')=(revoked_at IS NOT NULL))
);
CREATE INDEX consents_person ON identity.consents(person_id,created_at DESC);
CREATE TABLE identity.consent_history(consent_id uuid NOT NULL REFERENCES identity.consents(id),state text NOT NULL,actor_kind text NOT NULL,actor_person uuid,reason text,recorded_at timestamptz NOT NULL DEFAULT clock_timestamp());
CREATE TRIGGER immutable_consent_history BEFORE UPDATE OR DELETE OR TRUNCATE ON identity.consent_history FOR EACH STATEMENT EXECUTE FUNCTION controls.immutable_check_result();
CREATE FUNCTION identity.guard_consent() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE mutable text[]:=ARRAY['state','person_id','issuer','subject','account_ids','authorized_at','expires_at','revoked_at','revoked_by','revoked_by_person','revoke_reason','version'];BEGIN
 IF TG_OP<>'UPDATE' OR (to_jsonb(NEW)-mutable) IS DISTINCT FROM (to_jsonb(OLD)-mutable)
  OR NOT ((OLD.state='AWAITING_AUTHORIZATION' AND NEW.state IN ('ACTIVE','REJECTED')) OR (OLD.state='ACTIVE' AND NEW.state='REVOKED'
   AND NEW.person_id=OLD.person_id AND NEW.account_ids=OLD.account_ids AND NEW.expires_at=OLD.expires_at AND NEW.authorized_at=OLD.authorized_at))
 THEN RAISE EXCEPTION 'Consent identity and history are immutable'; END IF;
 RETURN NEW;END $$;
CREATE TRIGGER consent_guard BEFORE UPDATE OR DELETE ON identity.consents FOR EACH ROW EXECUTE FUNCTION identity.guard_consent();
-- Expiry is derived at read time, so a lapsed grant needs no job to stop working.
CREATE FUNCTION identity.consent_state(k identity.consents) RETURNS text LANGUAGE sql STABLE SET search_path=pg_catalog AS $$
 SELECT CASE WHEN k.state='ACTIVE' AND k.expires_at<=clock_timestamp() THEN 'EXPIRED'
  WHEN k.state='AWAITING_AUTHORIZATION' AND k.authorization_deadline<=clock_timestamp() THEN 'EXPIRED' ELSE k.state END $$;
CREATE FUNCTION identity.consent_json(k identity.consents,p_detail boolean) RETURNS jsonb LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
 SELECT jsonb_build_object('id',k.id,'clientId',k.client_id,'partnerName',(SELECT display_name FROM identity.partner_clients WHERE id=k.client_ref),
  'scopes',to_jsonb(k.scopes),'durationDays',k.duration_days,'state',identity.consent_state(k),'accountIds',coalesce(to_jsonb(k.account_ids),'[]'::jsonb),
  'authorizationDeadline',controls.utc_text(k.authorization_deadline),'authorizedAt',controls.utc_text(k.authorized_at),'expiresAt',controls.utc_text(k.expires_at),
  'revokedAt',controls.utc_text(k.revoked_at),'revokedBy',k.revoked_by,'createdAt',controls.utc_text(k.created_at))
  ||CASE WHEN p_detail THEN jsonb_build_object('personId',k.person_id,'revokeReason',k.revoke_reason,
   'history',coalesce((SELECT jsonb_agg(jsonb_build_object('state',h.state,'actorKind',h.actor_kind,'actorPerson',h.actor_person,'reason',h.reason,'recordedAt',controls.utc_text(h.recorded_at)) ORDER BY h.recorded_at)
    FROM identity.consent_history h WHERE h.consent_id=k.id),'[]'::jsonb)) ELSE '{}'::jsonb END $$;
-- Partner-facing view never includes the customer's identity.
CREATE FUNCTION identity.consent_partner_json(k identity.consents) RETURNS jsonb LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
 SELECT jsonb_build_object('consentId',k.id,'scopes',to_jsonb(k.scopes),'state',identity.consent_state(k),'accountIds',coalesce(to_jsonb(k.account_ids),'[]'::jsonb),
  'authorizationDeadline',controls.utc_text(k.authorization_deadline),'expiresAt',controls.utc_text(k.expires_at),'revokedAt',controls.utc_text(k.revoked_at)) $$;

CREATE FUNCTION identity.partner_client_json(c identity.partner_clients) RETURNS jsonb LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
 SELECT jsonb_build_object('id',c.id,'clientId',c.client_id,'displayName',c.display_name,'allowedScopes',to_jsonb(c.allowed_scopes),'quotaPerMinute',c.quota_per_minute,
  'callbackUrl',c.callback_url,'state',c.state,'proposedBy',c.proposed_by,'decidedBy',c.decided_by,'decisionReason',c.decision_reason,'version',c.version,
  'createdAt',controls.utc_text(c.created_at),'updatedAt',controls.utc_text(c.updated_at),
  'activeConsents',(SELECT count(*) FROM identity.consents k WHERE k.client_ref=c.id AND k.state='ACTIVE' AND k.expires_at>clock_timestamp())) $$;

CREATE TABLE identity.partner_callbacks(
 id uuid PRIMARY KEY,client_ref uuid NOT NULL REFERENCES identity.partner_clients(id),consent_id uuid NOT NULL REFERENCES identity.consents(id),
 event_type text NOT NULL CHECK(event_type IN ('consent.authorized','consent.rejected','consent.revoked')),payload jsonb NOT NULL,
 state text NOT NULL DEFAULT 'PENDING' CHECK(state IN ('PENDING','DELIVERED','FAILED')),attempts integer NOT NULL DEFAULT 0 CHECK(attempts BETWEEN 0 AND 8),
 next_attempt_at timestamptz NOT NULL DEFAULT clock_timestamp(),lease_until timestamptz,last_status integer,delivered_at timestamptz,
 created_at timestamptz NOT NULL DEFAULT clock_timestamp(),UNIQUE(consent_id,event_type)
);
CREATE FUNCTION identity.enqueue_consent_callback(k identity.consents,p_event text) RETURNS void LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,identity AS $$
BEGIN
 IF (SELECT callback_url FROM identity.partner_clients WHERE id=k.client_ref) IS NULL THEN RETURN; END IF;
 INSERT INTO identity.partner_callbacks(id,client_ref,consent_id,event_type,payload)
  VALUES(gen_random_uuid(),k.client_ref,k.id,p_event,jsonb_build_object('eventType',p_event,'consent',identity.consent_partner_json(k),'occurredAt',controls.utc_text(clock_timestamp())))
  ON CONFLICT(consent_id,event_type) DO NOTHING;
END $$;

-- The partner asks; nothing is granted until the customer authorizes in a bank channel within 15 minutes.
CREATE FUNCTION identity.consent_request(p_client text,p_key uuid,p_data jsonb) RETURNS jsonb
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,identity AS $$
DECLARE c identity.partner_clients;k identity.consents;scopes text[];BEGIN
 SELECT * INTO c FROM identity.partner_clients WHERE client_id=p_client AND state='ACTIVE' FOR SHARE;
 IF NOT FOUND THEN RAISE EXCEPTION 'Partner client unavailable' USING ERRCODE='42501'; END IF;
 IF p_key IS NULL OR jsonb_typeof(p_data)<>'object' OR EXISTS(SELECT 1 FROM jsonb_object_keys(p_data) x WHERE x NOT IN ('scopes','durationDays'))
   OR jsonb_typeof(p_data->'scopes')<>'array' OR jsonb_array_length(p_data->'scopes')=0 THEN RAISE EXCEPTION 'Invalid consent request' USING ERRCODE='22023'; END IF;
 SELECT array_agg(DISTINCT s ORDER BY s) INTO scopes FROM jsonb_array_elements_text(p_data->'scopes') s;
 IF NOT scopes<@c.allowed_scopes THEN RAISE EXCEPTION 'Requested scope exceeds the partner registration' USING ERRCODE='42501'; END IF;
 SELECT * INTO k FROM identity.consents WHERE client_ref=c.id AND request_key=p_key;
 IF FOUND THEN
  IF k.scopes<>scopes OR k.duration_days<>(p_data->>'durationDays')::integer THEN RAISE EXCEPTION 'Consent request key conflict'; END IF;
  RETURN identity.consent_partner_json(k);
 END IF;
 INSERT INTO identity.consents(id,client_ref,client_id,request_key,scopes,duration_days,state,authorization_deadline)
  VALUES(gen_random_uuid(),c.id,c.client_id,p_key,scopes,(p_data->>'durationDays')::integer,'AWAITING_AUTHORIZATION',clock_timestamp()+interval '15 minutes') RETURNING * INTO k;
 INSERT INTO identity.consent_history VALUES(k.id,k.state,'PARTNER',NULL,'REQUESTED',clock_timestamp());
 RETURN identity.consent_partner_json(k);
END $$;
-- Customer view of a pending request (any signed-in customer may open the link; it binds to whoever authorizes it).
CREATE FUNCTION identity.consent_for_customer(p_person uuid,p_id uuid) RETURNS jsonb LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,identity AS $$
DECLARE k identity.consents;BEGIN
 SELECT * INTO k FROM identity.consents WHERE id=p_id;
 IF NOT FOUND OR (k.person_id IS NOT NULL AND k.person_id<>p_person) OR (k.person_id IS NULL AND identity.consent_state(k)<>'AWAITING_AUTHORIZATION') THEN
  RAISE EXCEPTION 'Consent not found' USING ERRCODE='P0002'; END IF;
 RETURN identity.consent_json(k,false);
END $$;
CREATE FUNCTION identity.consent_authorize(p_actor jsonb,p_person uuid,p_id uuid,p_approve boolean,p_accounts uuid[]) RETURNS jsonb
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,identity AS $$
DECLARE k identity.consents;accounts uuid[];BEGIN
 IF p_actor->>'trust' IS DISTINCT FROM 'CUSTOMER' THEN RAISE EXCEPTION 'Customer authorization required' USING ERRCODE='42501'; END IF;
 PERFORM 1 FROM controls.person_links WHERE issuer=p_actor->>'issuer' AND subject=p_actor->>'subject' AND person_id=p_person;
 IF NOT FOUND THEN RAISE EXCEPTION 'Current canonical identity required'; END IF;
 SELECT * INTO k FROM identity.consents WHERE id=p_id FOR UPDATE;
 IF NOT FOUND OR (k.person_id IS NOT NULL AND k.person_id<>p_person) THEN RAISE EXCEPTION 'Consent not found' USING ERRCODE='P0002'; END IF;
 IF k.state IN ('ACTIVE','REJECTED') AND k.person_id=p_person THEN RETURN identity.consent_json(k,false); END IF;
 IF identity.consent_state(k)<>'AWAITING_AUTHORIZATION' THEN RAISE EXCEPTION 'Consent request is no longer open' USING ERRCODE='P0002'; END IF;
 IF NOT EXISTS(SELECT 1 FROM identity.partner_clients WHERE id=k.client_ref AND state='ACTIVE') THEN RAISE EXCEPTION 'Partner client unavailable' USING ERRCODE='42501'; END IF;
 IF p_approve THEN
  SELECT array_agg(DISTINCT a ORDER BY a) INTO accounts FROM unnest(p_accounts) a;
  -- Only accounts the customer owns outright; delegated authority cannot be passed on to a partner.
  IF cardinality(accounts) IS NULL OR cardinality(accounts)>10 OR EXISTS(SELECT 1 FROM unnest(accounts) a WHERE NOT EXISTS(
     SELECT 1 FROM core.account_relationships r JOIN core.ledger_accounts l ON l.id=r.account_id WHERE r.account_id=a AND r.person_id=p_person AND r.kind='OWNER' AND r.revoked_at IS NULL AND l.state='ACTIVE'))
  THEN RAISE EXCEPTION 'Choose between one and ten of your own active accounts' USING ERRCODE='22023'; END IF;
  UPDATE identity.consents SET state='ACTIVE',person_id=p_person,issuer=p_actor->>'issuer',subject=p_actor->>'subject',account_ids=accounts,authorized_at=clock_timestamp(),
   expires_at=clock_timestamp()+make_interval(days=>duration_days),version=version+1 WHERE id=p_id RETURNING * INTO k;
  PERFORM identity.enqueue_consent_callback(k,'consent.authorized');
 ELSE
  UPDATE identity.consents SET state='REJECTED',person_id=p_person,issuer=p_actor->>'issuer',subject=p_actor->>'subject',version=version+1 WHERE id=p_id RETURNING * INTO k;
  PERFORM identity.enqueue_consent_callback(k,'consent.rejected');
 END IF;
 INSERT INTO identity.consent_history VALUES(k.id,k.state,'CUSTOMER',p_person,CASE WHEN p_approve THEN 'AUTHORIZED' ELSE 'REJECTED' END,clock_timestamp());
 PERFORM controls.append_audit(gen_random_uuid(),'partners',jsonb_build_object('actorIssuer',p_actor->>'issuer','actorSubject',p_actor->>'subject','actorPersonId',p_person,
  'representedParty',p_person,'action',CASE WHEN p_approve THEN 'consent.authorized' ELSE 'consent.rejected' END,'resource',k.id::text,'outcome','ALLOWED','policyVersion','PARTNER_POLICY_V1',
  'reasonCode',k.client_id,'correlationId',k.id,'occurredAt',controls.utc_text(clock_timestamp())));
 RETURN identity.consent_json(k,false);
END $$;
-- Revocation is prospective: new partner calls fail at once; already-submitted payments keep their own lifecycle.
CREATE FUNCTION identity.consent_revoke(p_actor jsonb,p_person uuid,p_id uuid,p_reason text) RETURNS jsonb
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,identity AS $$
DECLARE k identity.consents;kind text;BEGIN
 IF length(trim(coalesce(p_reason,''))) NOT BETWEEN 3 AND 300 THEN RAISE EXCEPTION 'Revocation reason required' USING ERRCODE='22023'; END IF;
 IF p_actor->>'trust'='CUSTOMER' THEN
  PERFORM 1 FROM controls.person_links WHERE issuer=p_actor->>'issuer' AND subject=p_actor->>'subject' AND person_id=p_person;
  IF NOT FOUND THEN RAISE EXCEPTION 'Current canonical identity required'; END IF;
  kind:='CUSTOMER';
 ELSIF p_actor->>'trust'='WORKFORCE' THEN
  PERFORM controls.require_staff(p_actor,p_person,ARRAY['security_admin']);kind:='STAFF';
 ELSE RAISE EXCEPTION 'Revocation not permitted' USING ERRCODE='42501'; END IF;
 SELECT * INTO k FROM identity.consents WHERE id=p_id FOR UPDATE;
 IF NOT FOUND OR (kind='CUSTOMER' AND k.person_id IS DISTINCT FROM p_person) THEN RAISE EXCEPTION 'Consent not found' USING ERRCODE='P0002'; END IF;
 IF k.state='REVOKED' THEN RETURN identity.consent_json(k,kind='STAFF'); END IF;
 IF k.state<>'ACTIVE' THEN RAISE EXCEPTION 'Only an authorized consent can be revoked'; END IF;
 UPDATE identity.consents SET state='REVOKED',revoked_at=clock_timestamp(),revoked_by=kind,revoked_by_person=p_person,revoke_reason=trim(p_reason),version=version+1 WHERE id=p_id RETURNING * INTO k;
 INSERT INTO identity.consent_history VALUES(k.id,k.state,kind,p_person,trim(p_reason),clock_timestamp());
 PERFORM identity.enqueue_consent_callback(k,'consent.revoked');
 PERFORM controls.append_audit(gen_random_uuid(),'partners',jsonb_build_object('actorIssuer',p_actor->>'issuer','actorSubject',p_actor->>'subject','actorPersonId',p_person,
  'representedParty',k.person_id,'action','consent.revoked','resource',k.id::text,'outcome','ALLOWED','policyVersion','PARTNER_POLICY_V1','reasonCode',kind,'correlationId',k.id,
  'occurredAt',controls.utc_text(clock_timestamp())));
 RETURN identity.consent_json(k,kind='STAFF');
END $$;
-- A partner may withdraw its own consent (e.g. the customer left the partner's service).
CREATE FUNCTION identity.consent_partner_withdraw(p_client text,p_id uuid) RETURNS jsonb
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,identity AS $$
DECLARE k identity.consents;BEGIN
 SELECT * INTO k FROM identity.consents WHERE id=p_id AND client_id=p_client FOR UPDATE;
 IF NOT FOUND THEN RAISE EXCEPTION 'Consent not found' USING ERRCODE='P0002'; END IF;
 IF k.state='ACTIVE' THEN
  UPDATE identity.consents SET state='REVOKED',revoked_at=clock_timestamp(),revoked_by='PARTNER',revoke_reason='Withdrawn by partner',version=version+1 WHERE id=p_id RETURNING * INTO k;
  INSERT INTO identity.consent_history VALUES(k.id,k.state,'PARTNER',NULL,'Withdrawn by partner',clock_timestamp());
 END IF;
 RETURN identity.consent_partner_json(k);
END $$;
CREATE FUNCTION identity.consent_status_for_partner(p_client text,p_id uuid) RETURNS jsonb LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
 SELECT identity.consent_partner_json(k) FROM identity.consents k WHERE k.id=p_id AND k.client_id=p_client $$;
-- Server-side binding used to build a partner actor: present only while the grant and client are both active.
CREATE FUNCTION identity.consent_binding(p_client text,p_id uuid) RETURNS jsonb LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
 SELECT jsonb_build_object('consentId',k.id,'personId',k.person_id,'issuer',k.issuer,'subject',k.subject,'scopes',to_jsonb(k.scopes),'accountIds',to_jsonb(k.account_ids),'expiresAt',controls.utc_text(k.expires_at))
 FROM identity.consents k JOIN identity.partner_clients c ON c.id=k.client_ref
 WHERE k.id=p_id AND k.client_id=p_client AND k.state='ACTIVE' AND k.expires_at>clock_timestamp() AND c.state='ACTIVE' $$;
-- Re-checked by Controls at decision time, so revocation and suspension apply to the very next call.
CREATE FUNCTION identity.consent_permits(p_consent uuid,p_client text,p_person uuid,p_account uuid,p_scope text) RETURNS boolean LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
 SELECT EXISTS(SELECT 1 FROM identity.consents k JOIN identity.partner_clients c ON c.id=k.client_ref
  WHERE k.id=p_consent AND k.client_id=p_client AND k.person_id=p_person AND k.state='ACTIVE' AND k.expires_at>clock_timestamp() AND c.state='ACTIVE'
   AND p_scope=ANY(k.scopes) AND (p_account IS NULL OR p_account=ANY(k.account_ids))) $$;
CREATE FUNCTION identity.customer_consents(p_person uuid) RETURNS jsonb LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
 SELECT coalesce(jsonb_agg(identity.consent_json(k,false) ORDER BY k.created_at DESC),'[]'::jsonb) FROM identity.consents k WHERE k.person_id=p_person $$;

-- Callback delivery claims (worker) and outcomes. Five-attempt exponential backoff, then FAILED for staff attention.
CREATE FUNCTION identity.claim_partner_callback() RETURNS jsonb LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,identity AS $$
DECLARE b identity.partner_callbacks;c identity.partner_clients;BEGIN
 SELECT * INTO b FROM identity.partner_callbacks WHERE state='PENDING' AND next_attempt_at<=clock_timestamp() AND (lease_until IS NULL OR lease_until<clock_timestamp())
  ORDER BY next_attempt_at,id LIMIT 1 FOR UPDATE SKIP LOCKED;
 IF NOT FOUND THEN RETURN NULL; END IF;
 UPDATE identity.partner_callbacks SET lease_until=clock_timestamp()+interval '30 seconds',attempts=attempts+1 WHERE id=b.id RETURNING * INTO b;
 SELECT * INTO STRICT c FROM identity.partner_clients WHERE id=b.client_ref;
 RETURN jsonb_build_object('id',b.id,'attempt',b.attempts,'url',c.callback_url,'sealedSecret',c.callback_secret_sealed,'clientId',c.client_id,'payload',b.payload);
END $$;
CREATE FUNCTION identity.record_partner_callback(p_id uuid,p_status integer) RETURNS void LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,identity AS $$
DECLARE b identity.partner_callbacks;BEGIN
 SELECT * INTO STRICT b FROM identity.partner_callbacks WHERE id=p_id FOR UPDATE;
 IF b.state<>'PENDING' THEN RETURN; END IF;
 IF p_status BETWEEN 200 AND 299 THEN
  UPDATE identity.partner_callbacks SET state='DELIVERED',delivered_at=clock_timestamp(),last_status=p_status,lease_until=NULL WHERE id=p_id;
 ELSIF b.attempts>=5 THEN
  UPDATE identity.partner_callbacks SET state='FAILED',last_status=p_status,lease_until=NULL WHERE id=p_id;
 ELSE
  UPDATE identity.partner_callbacks SET last_status=p_status,lease_until=NULL,next_attempt_at=clock_timestamp()+make_interval(secs=>power(2,b.attempts)::double precision) WHERE id=p_id;
 END IF;
END $$;

REVOKE INSERT,UPDATE,DELETE,TRUNCATE ON identity.partner_api_versions,identity.partner_clients,identity.partner_client_history,identity.partner_quota,
 identity.consents,identity.consent_history,identity.partner_callbacks FROM bank_identity;
GRANT EXECUTE ON FUNCTION identity.partner_api_version(text),identity.partner_propose(jsonb,uuid,uuid,jsonb,text),identity.partner_decide(jsonb,uuid,uuid,text,text),
 identity.partners_view(jsonb,uuid),identity.partner_client_active(text),identity.partner_quota_permits(text,integer),identity.consent_request(text,uuid,jsonb),
 identity.consent_for_customer(uuid,uuid),identity.consent_authorize(jsonb,uuid,uuid,boolean,uuid[]),identity.consent_revoke(jsonb,uuid,uuid,text),
 identity.consent_partner_withdraw(text,uuid),identity.consent_status_for_partner(text,uuid),identity.consent_binding(text,uuid),identity.customer_consents(uuid),
 identity.claim_partner_callback(),identity.record_partner_callback(uuid,integer) TO bank_identity;
GRANT EXECUTE ON FUNCTION identity.consent_permits(uuid,text,uuid,uuid,text) TO bank_controls;
GRANT USAGE ON SCHEMA identity TO bank_controls;

-- ===== Account delegation with bounded authority (FR-ACC-08) =====
-- A delegate shares a short-lived reference; the owner grants VIEW, or PAY up to a per-payment limit, for a fixed term.
CREATE TABLE core.delegate_references(code text PRIMARY KEY CHECK(code ~ '^DLG-[A-Z2-9]{8}$'),person_id uuid NOT NULL,created_at timestamptz NOT NULL DEFAULT clock_timestamp(),expires_at timestamptz NOT NULL);
CREATE INDEX delegate_references_person ON core.delegate_references(person_id,expires_at DESC);
CREATE TABLE core.account_delegations(
 id uuid PRIMARY KEY,account_id uuid NOT NULL REFERENCES core.account_registry(id),owner_person uuid NOT NULL,delegate_person uuid NOT NULL,
 authority text NOT NULL CHECK(authority IN ('VIEW','PAY')),payment_limit_minor bigint CHECK(payment_limit_minor BETWEEN 1 AND 10000000),
 expires_at timestamptz NOT NULL,request_key uuid NOT NULL,revoked_at timestamptz,revoked_by uuid,revoke_reason text,created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 UNIQUE(owner_person,request_key),CHECK(owner_person<>delegate_person),CHECK((authority='PAY')=(payment_limit_minor IS NOT NULL)),
 CHECK(expires_at>created_at AND expires_at<=created_at+interval '366 days'),CHECK((revoked_at IS NULL)=(revoked_by IS NULL))
);
CREATE UNIQUE INDEX account_delegations_open ON core.account_delegations(account_id,delegate_person) WHERE revoked_at IS NULL;
CREATE INDEX account_delegations_delegate ON core.account_delegations(delegate_person) WHERE revoked_at IS NULL;
CREATE TABLE core.delegation_history(delegation_id uuid NOT NULL REFERENCES core.account_delegations(id),event text NOT NULL,actor_person uuid NOT NULL,reason text,recorded_at timestamptz NOT NULL DEFAULT clock_timestamp());
CREATE TRIGGER immutable_delegation_history BEFORE UPDATE OR DELETE OR TRUNCATE ON core.delegation_history FOR EACH STATEMENT EXECUTE FUNCTION controls.immutable_check_result();
CREATE FUNCTION core.guard_delegation() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN
 IF TG_OP<>'UPDATE' OR OLD.revoked_at IS NOT NULL OR (to_jsonb(NEW)-ARRAY['revoked_at','revoked_by','revoke_reason']) IS DISTINCT FROM (to_jsonb(OLD)-ARRAY['revoked_at','revoked_by','revoke_reason'])
 THEN RAISE EXCEPTION 'Delegation terms and history are immutable'; END IF;
 RETURN NEW;END $$;
CREATE TRIGGER delegation_guard BEFORE UPDATE OR DELETE ON core.account_delegations FOR EACH ROW EXECUTE FUNCTION core.guard_delegation();
CREATE FUNCTION core.delegation_json(d core.account_delegations) RETURNS jsonb LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
 SELECT jsonb_build_object('id',d.id,'accountId',d.account_id,'ownerPersonId',d.owner_person,'delegatePersonId',d.delegate_person,'authority',d.authority,
  'paymentLimit',CASE WHEN d.payment_limit_minor IS NULL THEN NULL ELSE jsonb_build_object('minorUnits',d.payment_limit_minor::text,'currency',jsonb_build_object('code','TST','scale',2)) END,
  'state',CASE WHEN d.revoked_at IS NOT NULL THEN 'REVOKED' WHEN d.expires_at<=clock_timestamp() THEN 'EXPIRED' ELSE 'ACTIVE' END,
  'expiresAt',controls.utc_text(d.expires_at),'revokedAt',controls.utc_text(d.revoked_at),'createdAt',controls.utc_text(d.created_at)) $$;
CREATE FUNCTION core.delegate_reference(p_person uuid) RETURNS jsonb LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,core AS $$
DECLARE r core.delegate_references;alphabet text:='ABCDEFGHJKLMNPQRSTUVWXYZ23456789';code text;BEGIN
 SELECT * INTO r FROM core.delegate_references WHERE person_id=p_person AND expires_at>clock_timestamp()+interval '1 hour' ORDER BY expires_at DESC LIMIT 1;
 IF NOT FOUND THEN
  code:='DLG-'||(SELECT string_agg(substr(alphabet,1+get_byte(b,i)%32,1),'' ORDER BY i) FROM (SELECT uuid_send(gen_random_uuid()) b) x,unnest(ARRAY[0,1,2,3,4,5,10,11]) i);
  INSERT INTO core.delegate_references VALUES(code,p_person,clock_timestamp(),clock_timestamp()+interval '24 hours') RETURNING * INTO r;
 END IF;
 RETURN jsonb_build_object('reference',r.code,'expiresAt',controls.utc_text(r.expires_at));
END $$;
CREATE FUNCTION core.grant_delegation(p_owner uuid,p_key uuid,p_data jsonb) RETURNS jsonb LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,core AS $$
DECLARE d core.account_delegations;delegate uuid;limit_minor bigint;BEGIN
 IF p_key IS NULL OR jsonb_typeof(p_data)<>'object' OR EXISTS(SELECT 1 FROM jsonb_object_keys(p_data) k WHERE k NOT IN ('accountId','reference','authority','paymentLimitMinor','durationDays'))
  OR p_data->>'authority' NOT IN ('VIEW','PAY') OR coalesce((p_data->>'durationDays')::integer,0) NOT BETWEEN 1 AND 365 THEN RAISE EXCEPTION 'Invalid delegation' USING ERRCODE='22023'; END IF;
 SELECT * INTO d FROM core.account_delegations WHERE owner_person=p_owner AND request_key=p_key;
 IF FOUND THEN
  IF d.account_id<>(p_data->>'accountId')::uuid OR d.authority<>p_data->>'authority' THEN RAISE EXCEPTION 'Delegation key conflict'; END IF;
  RETURN core.delegation_json(d);
 END IF;
 PERFORM 1 FROM core.account_relationships WHERE account_id=(p_data->>'accountId')::uuid AND person_id=p_owner AND kind='OWNER' AND revoked_at IS NULL FOR SHARE;
 IF NOT FOUND THEN RAISE EXCEPTION 'Account not found' USING ERRCODE='P0002'; END IF;
 SELECT person_id INTO delegate FROM core.delegate_references WHERE code=upper(trim(p_data->>'reference')) AND expires_at>clock_timestamp();
 IF delegate IS NULL THEN RAISE EXCEPTION 'Delegate reference not recognised or expired' USING ERRCODE='22023'; END IF;
 IF delegate=p_owner OR EXISTS(SELECT 1 FROM core.account_relationships WHERE account_id=(p_data->>'accountId')::uuid AND person_id=delegate AND revoked_at IS NULL)
  THEN RAISE EXCEPTION 'That person already has a relationship with this account' USING ERRCODE='22023'; END IF;
 IF p_data->>'authority'='PAY' THEN
  limit_minor:=(p_data->>'paymentLimitMinor')::bigint;
  IF limit_minor IS NULL OR limit_minor NOT BETWEEN 1 AND 10000000 THEN RAISE EXCEPTION 'A payment limit between 0.01 and 100000.00 TST is required' USING ERRCODE='22023'; END IF;
 ELSIF p_data ? 'paymentLimitMinor' THEN RAISE EXCEPTION 'View-only delegation has no payment limit' USING ERRCODE='22023'; END IF;
 BEGIN
  INSERT INTO core.account_delegations(id,account_id,owner_person,delegate_person,authority,payment_limit_minor,expires_at,request_key)
   VALUES(gen_random_uuid(),(p_data->>'accountId')::uuid,p_owner,delegate,p_data->>'authority',limit_minor,clock_timestamp()+make_interval(days=>(p_data->>'durationDays')::integer),p_key) RETURNING * INTO d;
 EXCEPTION WHEN unique_violation THEN RAISE EXCEPTION 'This person already holds an open delegation on the account; revoke it first' USING ERRCODE='22023';
 END;
 INSERT INTO core.delegation_history VALUES(d.id,'GRANTED',p_owner,NULL,clock_timestamp());
 RETURN core.delegation_json(d);
END $$;
-- The owner revokes, or the delegate renounces. Past actions keep their recorded delegate actor.
CREATE FUNCTION core.revoke_delegation(p_person uuid,p_id uuid,p_reason text) RETURNS jsonb LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,core AS $$
DECLARE d core.account_delegations;BEGIN
 IF length(trim(coalesce(p_reason,''))) NOT BETWEEN 3 AND 300 THEN RAISE EXCEPTION 'Revocation reason required' USING ERRCODE='22023'; END IF;
 SELECT * INTO d FROM core.account_delegations WHERE id=p_id FOR UPDATE;
 IF NOT FOUND OR p_person NOT IN (d.owner_person,d.delegate_person) THEN RAISE EXCEPTION 'Delegation not found' USING ERRCODE='P0002'; END IF;
 IF d.revoked_at IS NOT NULL THEN RETURN core.delegation_json(d); END IF;
 UPDATE core.account_delegations SET revoked_at=clock_timestamp(),revoked_by=p_person,revoke_reason=trim(p_reason) WHERE id=p_id RETURNING * INTO d;
 INSERT INTO core.delegation_history VALUES(d.id,CASE WHEN p_person=d.owner_person THEN 'REVOKED' ELSE 'RENOUNCED' END,p_person,trim(p_reason),clock_timestamp());
 RETURN core.delegation_json(d);
END $$;
CREATE FUNCTION core.delegations_for(p_person uuid) RETURNS jsonb LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
 SELECT jsonb_build_object(
  'granted',coalesce((SELECT jsonb_agg(core.delegation_json(d) ORDER BY d.created_at DESC) FROM core.account_delegations d WHERE d.owner_person=p_person),'[]'::jsonb),
  'received',coalesce((SELECT jsonb_agg(core.delegation_json(d) ORDER BY d.created_at DESC) FROM core.account_delegations d WHERE d.delegate_person=p_person),'[]'::jsonb)) $$;
REVOKE INSERT,UPDATE,DELETE,TRUNCATE ON core.delegate_references,core.account_delegations,core.delegation_history FROM bank_core;
GRANT SELECT ON core.delegate_references,core.account_delegations,core.delegation_history TO bank_core;
GRANT EXECUTE ON FUNCTION core.delegate_reference(uuid),core.grant_delegation(uuid,uuid,jsonb),core.revoke_delegation(uuid,uuid,text),core.delegations_for(uuid) TO bank_core;

-- ===== Partner-initiated payments (AT-21) =====
-- A partner may draft a payment, but only the customer can approve it. On confirmation the customer's actor is
-- recorded once as the authorizing actor; execution, reconciliation and recovery then act for that customer, so
-- revoking the partner's consent cannot strand an approved payment. actor_context keeps the partner as initiator.
ALTER TABLE payments.intents ADD COLUMN authorized_actor jsonb;
CREATE OR REPLACE FUNCTION payments.guard_intent_identity() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF TG_OP<>'UPDATE' OR NEW.id<>OLD.id OR NEW.issuer<>OLD.issuer OR NEW.subject<>OLD.subject OR NEW.client_id<>OLD.client_id
 OR NEW.request_key<>OLD.request_key OR NEW.quote_id<>OLD.quote_id OR NEW.actor_context<>OLD.actor_context OR NEW.created_at<>OLD.created_at
 OR (OLD.challenge_id IS NOT NULL AND NEW.challenge_id IS DISTINCT FROM OLD.challenge_id)
 OR (OLD.operation_id IS NOT NULL AND NEW.operation_id IS DISTINCT FROM OLD.operation_id)
 OR (OLD.batch_id IS NOT NULL AND NEW.batch_id IS DISTINCT FROM OLD.batch_id)
 OR (OLD.authorized_actor IS NOT NULL AND NEW.authorized_actor IS DISTINCT FROM OLD.authorized_actor)
 OR (OLD.authorized_actor IS NULL AND NEW.authorized_actor IS NOT NULL AND (OLD.state<>'DRAFT' OR NEW.authorized_actor->>'trust'<>'CUSTOMER'
     OR NEW.authorized_actor->>'issuer'<>OLD.issuer OR NEW.authorized_actor->>'subject'<>OLD.subject))
 THEN RAISE EXCEPTION 'Immutable payment identity'; END IF;
 RETURN NEW;
END $$;
