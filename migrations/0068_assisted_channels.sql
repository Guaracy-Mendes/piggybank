-- owner: controls. WP-34 assisted channels (FR-CH-03, FR-CH-04, AT-22): branch, private banker and call center staff act
-- for a customer only inside an assisted session the customer authorizes in a bank channel. A per-channel policy fixture
-- enables only operations confirmed for that channel (DEC-09 is open, so payments are enabled in no channel). Every
-- decision re-checks the live session, the channel policy and the common account restrictions.

CREATE TABLE controls.channel_policies(
 channel text PRIMARY KEY CHECK(channel IN ('BRANCH','PRIVATE_BANKER','CALL_CENTER')),
 verification_methods text[] NOT NULL CHECK(cardinality(verification_methods)>0),
 operations text[] NOT NULL CHECK(operations<@ARRAY['READ_ACCOUNT','PROTECTIVE_FREEZE','DRAFT_PAYMENT']),
 session_minutes integer NOT NULL CHECK(session_minutes BETWEEN 5 AND 60),
 label text NOT NULL CHECK(label='SYNTHETIC_FIXTURE'),
 note text NOT NULL
);
INSERT INTO controls.channel_policies VALUES
 ('BRANCH',ARRAY['ID_DOCUMENT_SIGHTED'],ARRAY['READ_ACCOUNT','PROTECTIVE_FREEZE'],30,'SYNTHETIC_FIXTURE','Payments not confirmed for this channel (DEC-09)'),
 ('PRIVATE_BANKER',ARRAY['KNOWN_RELATIONSHIP','ID_DOCUMENT_SIGHTED'],ARRAY['READ_ACCOUNT','PROTECTIVE_FREEZE'],60,'SYNTHETIC_FIXTURE','Payments not confirmed for this channel (DEC-09)'),
 ('CALL_CENTER',ARRAY['VERIFIED_CONTACT_CALLBACK'],ARRAY['READ_ACCOUNT','PROTECTIVE_FREEZE'],15,'SYNTHETIC_FIXTURE','Read and protective freeze only; payments not confirmed (DEC-09)');
CREATE TRIGGER immutable_channel_policies BEFORE UPDATE OR DELETE OR TRUNCATE ON controls.channel_policies FOR EACH STATEMENT EXECUTE FUNCTION controls.immutable_check_result();

CREATE TABLE controls.assisted_sessions(
 id uuid PRIMARY KEY,channel text NOT NULL REFERENCES controls.channel_policies(channel),
 staff_person uuid NOT NULL,staff_actor jsonb NOT NULL,customer_person uuid NOT NULL,
 verification text NOT NULL,purpose text NOT NULL CHECK(length(trim(purpose)) BETWEEN 3 AND 300),request_key uuid NOT NULL,
 state text NOT NULL CHECK(state IN ('AWAITING_CUSTOMER','AUTHORIZED','DECLINED','CLOSED')),
 authorization_deadline timestamptz NOT NULL,authorized_at timestamptz,expires_at timestamptz,closed_at timestamptz,closed_by uuid,
 created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 UNIQUE(staff_person,request_key),CHECK(staff_person<>customer_person),
 CHECK((state='AUTHORIZED')=(authorized_at IS NOT NULL AND closed_at IS NULL) OR state IN ('CLOSED','DECLINED'))
);
CREATE INDEX assisted_customer ON controls.assisted_sessions(customer_person,created_at DESC);
CREATE TABLE controls.assisted_actions(
 session_id uuid NOT NULL REFERENCES controls.assisted_sessions(id),operation text NOT NULL,resource_id uuid,outcome text NOT NULL CHECK(outcome IN ('ALLOWED','DENIED')),
 reason text NOT NULL,recorded_at timestamptz NOT NULL DEFAULT clock_timestamp()
);
CREATE TRIGGER immutable_assisted_actions BEFORE UPDATE OR DELETE OR TRUNCATE ON controls.assisted_actions FOR EACH STATEMENT EXECUTE FUNCTION controls.immutable_check_result();
CREATE TABLE controls.assisted_session_history(session_id uuid NOT NULL REFERENCES controls.assisted_sessions(id),state text NOT NULL,actor_person uuid NOT NULL,recorded_at timestamptz NOT NULL DEFAULT clock_timestamp());
CREATE TRIGGER immutable_assisted_history BEFORE UPDATE OR DELETE OR TRUNCATE ON controls.assisted_session_history FOR EACH STATEMENT EXECUTE FUNCTION controls.immutable_check_result();
CREATE FUNCTION controls.guard_assisted_session() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE mutable text[]:=ARRAY['state','authorized_at','expires_at','closed_at','closed_by'];BEGIN
 IF TG_OP<>'UPDATE' OR (to_jsonb(NEW)-mutable) IS DISTINCT FROM (to_jsonb(OLD)-mutable)
  OR NOT ((OLD.state='AWAITING_CUSTOMER' AND NEW.state IN ('AUTHORIZED','DECLINED','CLOSED')) OR (OLD.state='AUTHORIZED' AND NEW.state='CLOSED'))
 THEN RAISE EXCEPTION 'Assisted session identity and history are immutable'; END IF;
 RETURN NEW;END $$;
CREATE TRIGGER assisted_session_guard BEFORE UPDATE OR DELETE ON controls.assisted_sessions FOR EACH ROW EXECUTE FUNCTION controls.guard_assisted_session();

CREATE FUNCTION controls.assisted_state(s controls.assisted_sessions) RETURNS text LANGUAGE sql STABLE SET search_path=pg_catalog AS $$
 SELECT CASE WHEN s.state='AUTHORIZED' AND s.expires_at<=clock_timestamp() THEN 'EXPIRED'
  WHEN s.state='AWAITING_CUSTOMER' AND s.authorization_deadline<=clock_timestamp() THEN 'EXPIRED' ELSE s.state END $$;
CREATE FUNCTION controls.assisted_json(s controls.assisted_sessions,p_staff boolean) RETURNS jsonb LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
 SELECT jsonb_build_object('id',s.id,'channel',s.channel,'verification',s.verification,'purpose',s.purpose,'state',controls.assisted_state(s),
  'operations',(SELECT to_jsonb(operations) FROM controls.channel_policies WHERE channel=s.channel),
  'authorizationDeadline',controls.utc_text(s.authorization_deadline),'authorizedAt',controls.utc_text(s.authorized_at),'expiresAt',controls.utc_text(s.expires_at),
  'closedAt',controls.utc_text(s.closed_at),'createdAt',controls.utc_text(s.created_at))
  ||CASE WHEN p_staff THEN jsonb_build_object('staffPerson',s.staff_person,'customerPerson',s.customer_person,
   'actions',coalesce((SELECT jsonb_agg(jsonb_build_object('operation',a.operation,'resourceId',a.resource_id,'outcome',a.outcome,'reason',a.reason,'recordedAt',controls.utc_text(a.recorded_at)) ORDER BY a.recorded_at)
    FROM controls.assisted_actions a WHERE a.session_id=s.id),'[]'::jsonb)) ELSE '{}'::jsonb END $$;

-- Staff open a session after verifying the customer by a method permitted for the channel; nothing is possible until the
-- customer authorizes it in their own bank channel (identity assurance cannot be bypassed by assistance).
CREATE FUNCTION controls.open_assisted_session(p_actor jsonb,p_person uuid,p_key uuid,p_data jsonb) RETURNS jsonb
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,controls AS $$
DECLARE s controls.assisted_sessions;pol controls.channel_policies;customer uuid;BEGIN
 PERFORM controls.require_staff(p_actor,p_person,ARRAY['assisted_service']);
 IF p_key IS NULL OR jsonb_typeof(p_data)<>'object' OR EXISTS(SELECT 1 FROM jsonb_object_keys(p_data) k WHERE k NOT IN ('channel','customerPerson','verification','purpose'))
 THEN RAISE EXCEPTION 'Invalid assisted session' USING ERRCODE='22023'; END IF;
 SELECT * INTO s FROM controls.assisted_sessions WHERE staff_person=p_person AND request_key=p_key;
 IF FOUND THEN
  IF s.customer_person<>(p_data->>'customerPerson')::uuid OR s.channel<>p_data->>'channel' THEN RAISE EXCEPTION 'Assisted session key conflict'; END IF;
  RETURN controls.assisted_json(s,true);
 END IF;
 SELECT * INTO pol FROM controls.channel_policies WHERE channel=p_data->>'channel';
 IF NOT FOUND THEN RAISE EXCEPTION 'Unknown channel' USING ERRCODE='22023'; END IF;
 IF NOT (p_data->>'verification')=ANY(pol.verification_methods) THEN RAISE EXCEPTION 'Verification method not permitted for this channel' USING ERRCODE='22023'; END IF;
 customer:=(p_data->>'customerPerson')::uuid;
 IF customer=p_person THEN RAISE EXCEPTION 'Staff cannot assist themselves'; END IF;
 IF NOT EXISTS(SELECT 1 FROM core.account_relationships WHERE person_id=customer AND kind='OWNER' AND revoked_at IS NULL) THEN RAISE EXCEPTION 'Customer not found' USING ERRCODE='P0002'; END IF;
 INSERT INTO controls.assisted_sessions(id,channel,staff_person,staff_actor,customer_person,verification,purpose,request_key,state,authorization_deadline)
  VALUES(gen_random_uuid(),pol.channel,p_person,p_actor,customer,p_data->>'verification',trim(p_data->>'purpose'),p_key,'AWAITING_CUSTOMER',clock_timestamp()+interval '10 minutes') RETURNING * INTO s;
 INSERT INTO controls.assisted_session_history VALUES(s.id,s.state,p_person,clock_timestamp());
 PERFORM controls.append_audit(gen_random_uuid(),'assisted',jsonb_build_object('actorIssuer',p_actor->>'issuer','actorSubject',p_actor->>'subject','actorPersonId',p_person,
  'representedParty',customer,'action','assisted.opened','resource',s.id::text,'outcome','ALLOWED','policyVersion','CHANNEL_POLICY_V1','reasonCode',s.channel,'correlationId',s.id,
  'occurredAt',controls.utc_text(clock_timestamp())));
 RETURN controls.assisted_json(s,true);
END $$;
CREATE FUNCTION controls.authorize_assisted_session(p_actor jsonb,p_person uuid,p_id uuid,p_approve boolean) RETURNS jsonb
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,controls AS $$
DECLARE s controls.assisted_sessions;pol controls.channel_policies;BEGIN
 IF p_actor->>'trust' IS DISTINCT FROM 'CUSTOMER' THEN RAISE EXCEPTION 'Customer authorization required' USING ERRCODE='42501'; END IF;
 PERFORM 1 FROM controls.person_links WHERE issuer=p_actor->>'issuer' AND subject=p_actor->>'subject' AND person_id=p_person;
 IF NOT FOUND THEN RAISE EXCEPTION 'Current canonical identity required'; END IF;
 SELECT * INTO s FROM controls.assisted_sessions WHERE id=p_id AND customer_person=p_person FOR UPDATE;
 IF NOT FOUND THEN RAISE EXCEPTION 'Assisted session not found' USING ERRCODE='P0002'; END IF;
 IF s.state IN ('AUTHORIZED','DECLINED') THEN RETURN controls.assisted_json(s,false); END IF;
 IF controls.assisted_state(s)<>'AWAITING_CUSTOMER' THEN RAISE EXCEPTION 'Assisted session is no longer open' USING ERRCODE='P0002'; END IF;
 SELECT * INTO STRICT pol FROM controls.channel_policies WHERE channel=s.channel;
 IF p_approve THEN
  UPDATE controls.assisted_sessions SET state='AUTHORIZED',authorized_at=clock_timestamp(),expires_at=clock_timestamp()+make_interval(mins=>pol.session_minutes) WHERE id=p_id RETURNING * INTO s;
 ELSE
  UPDATE controls.assisted_sessions SET state='DECLINED' WHERE id=p_id RETURNING * INTO s;
 END IF;
 INSERT INTO controls.assisted_session_history VALUES(s.id,s.state,p_person,clock_timestamp());
 PERFORM controls.append_audit(gen_random_uuid(),'assisted',jsonb_build_object('actorIssuer',p_actor->>'issuer','actorSubject',p_actor->>'subject','actorPersonId',p_person,
  'representedParty',p_person,'action',CASE WHEN p_approve THEN 'assisted.authorized' ELSE 'assisted.declined' END,'resource',s.id::text,'outcome','ALLOWED','policyVersion','CHANNEL_POLICY_V1',
  'reasonCode',s.channel,'correlationId',s.id,'occurredAt',controls.utc_text(clock_timestamp())));
 RETURN controls.assisted_json(s,false);
END $$;
-- Staff close their own session; the customer may end it at any time.
CREATE FUNCTION controls.close_assisted_session(p_actor jsonb,p_person uuid,p_id uuid) RETURNS jsonb
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,controls AS $$
DECLARE s controls.assisted_sessions;staff boolean:=p_actor->>'trust'='WORKFORCE';BEGIN
 PERFORM 1 FROM controls.person_links WHERE issuer=p_actor->>'issuer' AND subject=p_actor->>'subject' AND person_id=p_person;
 IF NOT FOUND THEN RAISE EXCEPTION 'Current canonical identity required'; END IF;
 SELECT * INTO s FROM controls.assisted_sessions WHERE id=p_id AND (CASE WHEN staff THEN staff_person ELSE customer_person END)=p_person FOR UPDATE;
 IF NOT FOUND THEN RAISE EXCEPTION 'Assisted session not found' USING ERRCODE='P0002'; END IF;
 IF s.state IN ('CLOSED','DECLINED') THEN RETURN controls.assisted_json(s,staff); END IF;
 UPDATE controls.assisted_sessions SET state='CLOSED',closed_at=clock_timestamp(),closed_by=p_person WHERE id=p_id RETURNING * INTO s;
 INSERT INTO controls.assisted_session_history VALUES(s.id,s.state,p_person,clock_timestamp());
 RETURN controls.assisted_json(s,staff);
END $$;
-- Decision-time check used by Controls entitlements; every decision is recorded against the session.
CREATE FUNCTION controls.assisted_permits(p_session uuid,p_staff uuid,p_customer uuid,p_account uuid,p_operation text) RETURNS boolean
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,controls AS $$
DECLARE s controls.assisted_sessions;ok boolean:=false;why text;BEGIN
 SELECT * INTO s FROM controls.assisted_sessions WHERE id=p_session;
 IF NOT FOUND THEN RETURN false; END IF;
 IF s.staff_person<>p_staff OR s.customer_person<>p_customer THEN why:='SESSION_PARTY_MISMATCH';
 ELSIF controls.assisted_state(s)<>'AUTHORIZED' THEN why:='SESSION_NOT_ACTIVE';
 ELSIF NOT EXISTS(SELECT 1 FROM controls.channel_policies WHERE channel=s.channel AND p_operation=ANY(operations)) THEN why:='OPERATION_NOT_ENABLED_FOR_CHANNEL';
 ELSIF NOT EXISTS(SELECT 1 FROM core.account_relationships WHERE account_id=p_account AND person_id=p_customer AND kind='OWNER' AND revoked_at IS NULL) THEN why:='NOT_CUSTOMER_ACCOUNT';
 ELSE ok:=true;why:='CHANNEL_POLICY';
 END IF;
 INSERT INTO controls.assisted_actions VALUES(s.id,p_operation,p_account,CASE WHEN ok THEN 'ALLOWED' ELSE 'DENIED' END,why,clock_timestamp());
 RETURN ok;
END $$;
CREATE FUNCTION controls.assisted_staff_view(p_actor jsonb,p_person uuid) RETURNS jsonb
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,controls AS $$ BEGIN
 PERFORM controls.require_staff(p_actor,p_person,ARRAY['assisted_service','security_admin']);
 RETURN jsonb_build_object(
  'policies',(SELECT jsonb_agg(jsonb_build_object('channel',channel,'verificationMethods',to_jsonb(verification_methods),'operations',to_jsonb(operations),'sessionMinutes',session_minutes,'note',note) ORDER BY channel) FROM controls.channel_policies),
  'sessions',coalesce((SELECT jsonb_agg(controls.assisted_json(s,true) ORDER BY s.created_at DESC) FROM (SELECT * FROM controls.assisted_sessions WHERE staff_person=p_person OR EXISTS(
    SELECT 1 FROM controls.role_grants g WHERE g.person_id=p_person AND g.role='security_admin' AND g.revoked_at IS NULL AND g.valid_until>clock_timestamp()) ORDER BY created_at DESC LIMIT 50) s),'[]'::jsonb));
END $$;
CREATE FUNCTION controls.assisted_customer_view(p_person uuid) RETURNS jsonb LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
 SELECT coalesce(jsonb_agg(controls.assisted_json(s,false) ORDER BY s.created_at DESC),'[]'::jsonb) FROM (SELECT * FROM controls.assisted_sessions WHERE customer_person=p_person ORDER BY created_at DESC LIMIT 20) s $$;
REVOKE INSERT,UPDATE,DELETE,TRUNCATE ON controls.channel_policies,controls.assisted_sessions,controls.assisted_actions,controls.assisted_session_history FROM bank_controls;
GRANT EXECUTE ON FUNCTION controls.open_assisted_session(jsonb,uuid,uuid,jsonb),controls.authorize_assisted_session(jsonb,uuid,uuid,boolean),controls.close_assisted_session(jsonb,uuid,uuid),
 controls.assisted_permits(uuid,uuid,uuid,uuid,text),controls.assisted_staff_view(jsonb,uuid),controls.assisted_customer_view(uuid) TO bank_controls;
