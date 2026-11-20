-- owners: data (analytics schema: signals, versioned segments, consent-checked audiences with pseudonymous members,
-- journey-partner registry, exports, partner messages, advisory insights, dismissals and outcome reports). WP-38: one local
-- journey-partner simulator role (SIM_JOURNEY) as the template for selected partners (DEC-05/06/09 pending). Partners
-- receive minimized pseudonymous tokens only and can create advisory insights only: no approval, entitlement or posting
-- path exists. Marketing withdrawal hides optional insights and withholds exports without deleting evidence.

-- ===== Purposes, signals, segments, partners (immutable fixtures) =====
INSERT INTO analytics.purposes VALUES('SERVICE_OUTREACH',false,'Operational service outreach from the bank itself. Needs no marketing consent; respects ANALYTICS restrictions; never exported to partners.');
INSERT INTO analytics.purpose_roles VALUES('SERVICE_OUTREACH','data_steward'),('SERVICE_OUTREACH','data_analyst');
CREATE TABLE analytics.signals(code text PRIMARY KEY,description text NOT NULL,minimization text NOT NULL);
INSERT INTO analytics.signals VALUES
 ('ACTIVE_ACCOUNT','The party has an account whose latest observed state is ACTIVE.','Boolean per party; no account identifier leaves the warehouse.'),
 ('RECENTLY_CREDITED','A customer account of the party received a credit with a business date in the last 30 days.','Boolean per party; no amounts or counterparties.'),
 ('NO_RECENT_ACTIVITY','No ledger entry on the party''s customer accounts with a business date in the last 30 days.','Boolean per party; no balances.');
CREATE TABLE analytics.segments(code text NOT NULL,version integer NOT NULL CHECK(version>0),purpose text NOT NULL REFERENCES analytics.purposes,
 signals text[] NOT NULL,definition text NOT NULL,digest text NOT NULL,PRIMARY KEY(code,version),CHECK(purpose IN ('MARKETING','SERVICE_OUTREACH')));
INSERT INTO analytics.segments SELECT c,v,p,s,d,encode(sha256(convert_to(c||':'||v||':'||p||':'||array_to_string(s,',')||':'||d,'UTF8')),'hex') FROM (VALUES
 ('savings_starter',1,'MARKETING',ARRAY['ACTIVE_ACCOUNT','RECENTLY_CREDITED'],'Parties with an active account that received a credit in the last 30 days and who currently permit marketing.'),
 ('dormant_check_in',1,'SERVICE_OUTREACH',ARRAY['ACTIVE_ACCOUNT','NO_RECENT_ACTIVITY'],'Parties with an active account and no ledger activity in the last 30 days (service outreach, not marketing).')) x(c,v,p,s,d);
CREATE TABLE analytics.insight_rules(code text NOT NULL,version integer NOT NULL,segment text NOT NULL,segment_version integer NOT NULL,purpose text NOT NULL,
 title text NOT NULL,body text NOT NULL,action_route text,digest text NOT NULL,PRIMARY KEY(code,version),FOREIGN KEY(segment,segment_version) REFERENCES analytics.segments);
INSERT INTO analytics.insight_rules SELECT c,v,s,sv,p,t,b,r,encode(sha256(convert_to(c||':'||v||':'||t||':'||b||':'||coalesce(r,''),'UTF8')),'hex') FROM (VALUES
 ('savings_starter_tip',1,'savings_starter',1,'MARKETING','Put part of each deposit aside','A savings vault can hold money you do not plan to spend. Moving money is always your decision.','#/accounts'),
 ('dormant_check_in_notice',1,'dormant_check_in',1,'SERVICE_OUTREACH','No recent activity on your account','Your account has had no activity for 30 days. Check your details are still current.','#/profile')) x(c,v,s,sv,p,t,b,r);
CREATE TABLE analytics.journey_partners(code text PRIMARY KEY,status text NOT NULL CHECK(status IN ('LOCAL_TEMPLATE','PENDING_SELECTION')),
 purposes text[] NOT NULL,recommendation_codes text[] NOT NULL,action_routes text[] NOT NULL,description text NOT NULL);
INSERT INTO analytics.journey_partners VALUES('SIM_JOURNEY','LOCAL_TEMPLATE',ARRAY['MARKETING'],ARRAY['SAVINGS_TIP','BUDGET_REMINDER','PRODUCT_EDUCATION'],
 ARRAY['#/accounts','#/payments','#/profile'],'Local journey-partner simulator role. Receives pseudonymous audience tokens and returns advisory recommendations only.');

-- ===== Audiences, exports, partner messages, insights, outcomes =====
CREATE TABLE analytics.audiences(id uuid PRIMARY KEY,segment text NOT NULL,segment_version integer NOT NULL,segment_digest text NOT NULL,purpose text NOT NULL,
 fact_cut bigint NOT NULL,activity_from date NOT NULL,candidates integer NOT NULL,members integer NOT NULL,excluded jsonb NOT NULL,member_digest text NOT NULL,
 requested_by jsonb NOT NULL,requested_person uuid NOT NULL,built_at timestamptz NOT NULL DEFAULT clock_timestamp(),FOREIGN KEY(segment,segment_version) REFERENCES analytics.segments);
CREATE TABLE analytics.audience_members(audience_id uuid NOT NULL REFERENCES analytics.audiences DEFERRABLE INITIALLY DEFERRED,party_id uuid NOT NULL,
 member_token text NOT NULL UNIQUE CHECK(member_token ~ '^m_[0-9a-f]{32}$'),PRIMARY KEY(audience_id,party_id));
CREATE TABLE analytics.audience_exports(id uuid PRIMARY KEY,audience_id uuid NOT NULL REFERENCES analytics.audiences,partner text NOT NULL REFERENCES analytics.journey_partners,
 exported integer NOT NULL,withdrawn_excluded integer NOT NULL,restricted_excluded integer NOT NULL,export_digest text NOT NULL,requested_by jsonb NOT NULL,
 requested_person uuid NOT NULL,exported_at timestamptz NOT NULL DEFAULT clock_timestamp());
CREATE TABLE analytics.export_items(export_id uuid NOT NULL REFERENCES analytics.audience_exports DEFERRABLE INITIALLY DEFERRED,member_token text NOT NULL,PRIMARY KEY(export_id,member_token));
CREATE TABLE analytics.export_deliveries(id uuid PRIMARY KEY,export_id uuid NOT NULL REFERENCES analytics.audience_exports,delivered integer NOT NULL,withheld integer NOT NULL,
 delivered_at timestamptz NOT NULL DEFAULT clock_timestamp());
CREATE TABLE analytics.partner_messages(message_id text PRIMARY KEY CHECK(message_id ~ '^[A-Za-z0-9_-]{8,64}$'),partner text NOT NULL REFERENCES analytics.journey_partners,
 digest text NOT NULL,result text NOT NULL,response jsonb NOT NULL,received_at timestamptz NOT NULL DEFAULT clock_timestamp());
CREATE TABLE analytics.insights(id uuid PRIMARY KEY,party_id uuid NOT NULL,source text NOT NULL CHECK(source IN ('BANK_RULE','PARTNER')),code text NOT NULL,version integer NOT NULL,
 purpose text NOT NULL REFERENCES analytics.purposes,title text NOT NULL CHECK(length(title) BETWEEN 3 AND 80),body text NOT NULL CHECK(length(body) BETWEEN 3 AND 400),
 action_route text CHECK(action_route ~ '^#/[a-z/-]{1,40}$'),provenance jsonb NOT NULL,dedupe_key text NOT NULL UNIQUE,created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 CHECK(purpose IN ('MARKETING','SERVICE_OUTREACH')));
CREATE INDEX insights_party ON analytics.insights(party_id,created_at);
CREATE TABLE analytics.insight_dismissals(insight_id uuid PRIMARY KEY REFERENCES analytics.insights,actor jsonb NOT NULL,dismissed_at timestamptz NOT NULL DEFAULT clock_timestamp());
CREATE TABLE analytics.outcome_reports(id uuid PRIMARY KEY,audience_id uuid NOT NULL REFERENCES analytics.audiences,fact_cut bigint NOT NULL,denominators jsonb NOT NULL,
 outcomes jsonb NOT NULL,watermarks jsonb NOT NULL,completeness jsonb NOT NULL,status text NOT NULL CHECK(status IN ('COMPLETE','INCOMPLETE')),digest text NOT NULL,
 requested_by jsonb NOT NULL,requested_person uuid NOT NULL,created_at timestamptz NOT NULL DEFAULT clock_timestamp());
DO $$ DECLARE t text;BEGIN
 FOREACH t IN ARRAY ARRAY['signals','segments','insight_rules','journey_partners','audiences','audience_members','audience_exports','export_items','export_deliveries',
  'partner_messages','insights','insight_dismissals','outcome_reports'] LOOP
  EXECUTE format('CREATE TRIGGER immutable_%s BEFORE UPDATE OR DELETE OR TRUNCATE ON analytics.%I FOR EACH STATEMENT EXECUTE FUNCTION data.immutable_history()',t,t);
 END LOOP;
END $$;

-- ===== Helpers =====
-- Marketing permission now: WP-26 preference (default off) unless a WP-28 MARKETING restriction applies.
CREATE FUNCTION analytics.marketing_permitted(p_party uuid) RETURNS boolean LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS
$$ SELECT coalesce(bool_and(coalesce(data.purpose_enabled(l.person_id,'MARKETING'),false)),false) FROM core.parties p
  JOIN controls.person_links l ON l.issuer=p.issuer AND l.subject=p.subject WHERE p.id=p_party $$;
CREATE FUNCTION analytics.restricted(p_party uuid,p_purpose text) RETURNS boolean LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS
$$ SELECT EXISTS(SELECT 1 FROM core.parties p JOIN controls.person_links l ON l.issuer=p.issuer AND l.subject=p.subject
  JOIN data.processing_restrictions r ON r.person_id=l.person_id WHERE p.id=p_party AND r.purpose=p_purpose) $$;
CREATE FUNCTION analytics.require_purpose(p_actor jsonb,p_person uuid,p_purpose text,p_what text) RETURNS boolean
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,analytics AS $$
BEGIN
 PERFORM controls.require_staff(p_actor,p_person,ARRAY(SELECT role FROM analytics.purpose_roles WHERE purpose=p_purpose));
 RETURN true;
EXCEPTION WHEN raise_exception THEN
 INSERT INTO analytics.access_denials VALUES(gen_random_uuid(),p_purpose,p_what,NULL,'ROLE_NOT_PERMITTED_FOR_PURPOSE',p_actor,p_person);
 PERFORM analytics.audit(p_actor,p_person,'analytics.'||split_part(p_what,':',1),p_what,'DENIED','ROLE_NOT_PERMITTED_FOR_PURPOSE');
 RETURN false;
END $$;
-- Signals per party at a fact cut (latest observed account states; business dates from activity_from).
CREATE FUNCTION analytics.party_signals(p_cut bigint,p_from date) RETURNS TABLE(party uuid,active boolean,credited boolean,dormant boolean)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
 WITH st AS (SELECT DISTINCT ON (s.account_id) s.account_id,(s.record->>'partyId')::uuid party,s.record->>'state' state FROM analytics.facts s
   WHERE s.dataset='account_states' AND s.seq<=p_cut ORDER BY s.account_id,(s.record->>'sourceVersion')::bigint DESC),
 act AS (SELECT f.account_id,bool_or((f.record->>'amountMinor')::bigint>0) credited,count(*) n FROM analytics.facts f
   WHERE f.dataset='ledger_entries' AND f.seq<=p_cut AND f.business_date>=p_from AND f.record->>'accountKind'='CUSTOMER' GROUP BY f.account_id)
 SELECT st.party,bool_or(st.state='ACTIVE'),coalesce(bool_or(st.state='ACTIVE' AND act.credited),false),
  bool_and(st.state<>'ACTIVE' OR act.n IS NULL)
 FROM st LEFT JOIN act ON act.account_id=st.account_id GROUP BY st.party $$;

-- ===== Audience build (consent and restriction checked per member) =====
CREATE FUNCTION analytics.build_audience(p_actor jsonb,p_person uuid,p_segment text,p_version integer) RETURNS jsonb
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,analytics AS $$
DECLARE s analytics.segments;r analytics.insight_rules;target uuid:=gen_random_uuid();cut bigint;since date:=(clock_timestamp() AT TIME ZONE 'UTC')::date-30;
 c record;n_cand integer:=0;n_mem integer:=0;no_consent integer:=0;restricted integer:=0;BEGIN
 SELECT * INTO s FROM analytics.segments WHERE code=p_segment AND version=p_version;IF NOT FOUND THEN RAISE EXCEPTION 'Segment unavailable' USING ERRCODE='P0002'; END IF;
 IF NOT analytics.require_purpose(p_actor,p_person,s.purpose,'audience:'||p_segment) THEN RETURN jsonb_build_object('outcome','DENIED','reason','ROLE_NOT_PERMITTED_FOR_PURPOSE'); END IF;
 PERFORM pg_advisory_xact_lock(hashtext('analytics.warehouse'));
 cut:=coalesce((SELECT max(seq) FROM analytics.facts),0);
 SELECT * INTO r FROM analytics.insight_rules WHERE segment=s.code AND segment_version=s.version ORDER BY version DESC LIMIT 1;
 FOR c IN SELECT * FROM analytics.party_signals(cut,since) ORDER BY party LOOP
  IF NOT (c.active AND (CASE WHEN 'RECENTLY_CREDITED'=ANY(s.signals) THEN c.credited ELSE true END) AND (CASE WHEN 'NO_RECENT_ACTIVITY'=ANY(s.signals) THEN c.dormant ELSE true END)) THEN CONTINUE; END IF;
  n_cand:=n_cand+1;
  IF analytics.restricted(c.party,'ANALYTICS') OR (s.purpose='MARKETING' AND analytics.restricted(c.party,'MARKETING')) THEN restricted:=restricted+1;CONTINUE; END IF;
  IF s.purpose='MARKETING' AND NOT analytics.marketing_permitted(c.party) THEN no_consent:=no_consent+1;CONTINUE; END IF;
  INSERT INTO analytics.audience_members VALUES(target,c.party,'m_'||left(encode(sha256(convert_to(target::text||':'||c.party::text,'UTF8')),'hex'),32));
  n_mem:=n_mem+1;
  IF r.code IS NOT NULL THEN
   INSERT INTO analytics.insights(id,party_id,source,code,version,purpose,title,body,action_route,provenance,dedupe_key)
   VALUES(gen_random_uuid(),c.party,'BANK_RULE',r.code,r.version,r.purpose,r.title,r.body,r.action_route,
    jsonb_build_object('rule',r.code,'ruleVersion',r.version,'ruleDigest',r.digest,'segment',s.code,'segmentVersion',s.version,'segmentDigest',s.digest,'audienceId',target,'factCut',cut),
    'BANK_RULE:'||r.code||':'||r.version||':'||c.party) ON CONFLICT(dedupe_key) DO NOTHING;
  END IF;
 END LOOP;
 INSERT INTO analytics.audiences VALUES(target,s.code,s.version,s.digest,s.purpose,cut,since,n_cand,n_mem,
  jsonb_build_object('noMarketingConsent',no_consent,'restricted',restricted),
  encode(sha256(convert_to(coalesce((SELECT string_agg(member_token,',' ORDER BY member_token) FROM analytics.audience_members WHERE audience_id=target),''),'UTF8')),'hex'),p_actor,p_person);
 PERFORM analytics.audit(p_actor,p_person,'analytics.audience',target::text,'RECORDED',s.purpose);
 RETURN analytics.audience_document(target)||jsonb_build_object('outcome','CREATED');
END $$;
CREATE FUNCTION analytics.report_document(p_id uuid) RETURNS jsonb LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
 SELECT jsonb_build_object('id',o.id,'audienceId',o.audience_id,'factCut',o.fact_cut,'denominators',o.denominators,'outcomes',o.outcomes,'watermarks',o.watermarks,
  'completeness',o.completeness,'status',o.status,'digest',o.digest,'requestedBy',o.requested_by->>'subject','createdAt',analytics.utc(o.created_at))
 FROM analytics.outcome_reports o WHERE o.id=p_id $$;
CREATE FUNCTION analytics.audience_document(p_id uuid) RETURNS jsonb LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
 SELECT jsonb_build_object('id',a.id,'segment',a.segment,'segmentVersion',a.segment_version,'segmentDigest',a.segment_digest,'purpose',a.purpose,'factCut',a.fact_cut,
  'activityFrom',a.activity_from,'candidates',a.candidates,'members',a.members,'excluded',a.excluded,'memberDigest',a.member_digest,'builtAt',analytics.utc(a.built_at),
  'requestedBy',a.requested_by->>'subject',
  'exports',(SELECT coalesce(jsonb_agg(jsonb_build_object('id',e.id,'partner',e.partner,'exported',e.exported,'withdrawnExcluded',e.withdrawn_excluded,
    'restrictedExcluded',e.restricted_excluded,'exportedAt',analytics.utc(e.exported_at),
    'deliveries',(SELECT coalesce(jsonb_agg(jsonb_build_object('delivered',d.delivered,'withheld',d.withheld,'at',analytics.utc(d.delivered_at)) ORDER BY d.delivered_at),'[]'::jsonb)
      FROM analytics.export_deliveries d WHERE d.export_id=e.id)) ORDER BY e.exported_at),'[]'::jsonb) FROM analytics.audience_exports e WHERE e.audience_id=a.id),
  'reports',(SELECT coalesce(jsonb_agg(analytics.report_document(o.id) ORDER BY o.created_at),'[]'::jsonb) FROM analytics.outcome_reports o WHERE o.audience_id=a.id))
 FROM analytics.audiences a WHERE a.id=p_id $$;

-- ===== Export to a journey partner (consent re-checked at export and again at delivery) =====
CREATE FUNCTION analytics.export_audience(p_actor jsonb,p_person uuid,p_audience uuid,p_partner text) RETURNS jsonb
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,analytics AS $$
DECLARE a analytics.audiences;jp analytics.journey_partners;target uuid:=gen_random_uuid();m record;n integer:=0;withdrawn integer:=0;restricted integer:=0;BEGIN
 SELECT * INTO a FROM analytics.audiences WHERE id=p_audience;IF NOT FOUND THEN RAISE EXCEPTION 'Audience unavailable' USING ERRCODE='P0002'; END IF;
 SELECT * INTO jp FROM analytics.journey_partners WHERE code=p_partner;IF NOT FOUND THEN RAISE EXCEPTION 'Partner unavailable' USING ERRCODE='P0002'; END IF;
 IF NOT analytics.require_purpose(p_actor,p_person,a.purpose,'export:'||p_partner) THEN RETURN jsonb_build_object('outcome','DENIED','reason','ROLE_NOT_PERMITTED_FOR_PURPOSE'); END IF;
 IF NOT a.purpose=ANY(jp.purposes) THEN
  INSERT INTO analytics.access_denials VALUES(target,a.purpose,'export:'||p_partner,NULL,'PURPOSE_NOT_PERMITTED_FOR_PARTNER',p_actor,p_person);
  PERFORM analytics.audit(p_actor,p_person,'analytics.export',target::text,'DENIED','PURPOSE_NOT_PERMITTED_FOR_PARTNER');
  RETURN jsonb_build_object('outcome','DENIED','reason','PURPOSE_NOT_PERMITTED_FOR_PARTNER','denialId',target);
 END IF;
 FOR m IN SELECT * FROM analytics.audience_members WHERE audience_id=a.id ORDER BY member_token LOOP
  IF analytics.restricted(m.party_id,'ANALYTICS') OR analytics.restricted(m.party_id,'MARKETING') THEN restricted:=restricted+1;
  ELSIF NOT analytics.marketing_permitted(m.party_id) THEN withdrawn:=withdrawn+1;
  ELSE INSERT INTO analytics.export_items VALUES(target,m.member_token);n:=n+1; END IF;
 END LOOP;
 INSERT INTO analytics.audience_exports VALUES(target,a.id,p_partner,n,withdrawn,restricted,
  encode(sha256(convert_to(coalesce((SELECT string_agg(member_token,',' ORDER BY member_token) FROM analytics.export_items WHERE export_id=target),''),'UTF8')),'hex'),p_actor,p_person);
 PERFORM analytics.audit(p_actor,p_person,'analytics.export',target::text,'RECORDED',p_partner);
 RETURN jsonb_build_object('outcome','CREATED','exportId',target,'audienceId',a.id,'partner',p_partner,'exported',n,'withdrawnExcluded',withdrawn,'restrictedExcluded',restricted);
END $$;
-- Partner channel (signature verified by the caller): pseudonymous tokens only, minus anyone who withdrew since export.
CREATE FUNCTION analytics.partner_export(p_partner text,p_export uuid) RETURNS jsonb
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,analytics AS $$
DECLARE e analytics.audience_exports;a analytics.audiences;tokens jsonb;held integer;BEGIN
 SELECT * INTO e FROM analytics.audience_exports WHERE id=p_export AND partner=p_partner;IF NOT FOUND THEN RAISE EXCEPTION 'Export unavailable' USING ERRCODE='P0002'; END IF;
 SELECT * INTO a FROM analytics.audiences WHERE id=e.audience_id;
 SELECT coalesce(jsonb_agg(i.member_token ORDER BY i.member_token) FILTER (WHERE ok),'[]'::jsonb),count(*) FILTER (WHERE NOT ok) INTO tokens,held
 FROM (SELECT i.member_token,analytics.marketing_permitted(m.party_id) AND NOT analytics.restricted(m.party_id,'ANALYTICS') AND NOT analytics.restricted(m.party_id,'MARKETING') ok
  FROM analytics.export_items i JOIN analytics.audience_members m ON m.audience_id=e.audience_id AND m.member_token=i.member_token WHERE i.export_id=e.id) i;
 INSERT INTO analytics.export_deliveries VALUES(gen_random_uuid(),e.id,jsonb_array_length(tokens),held);
 RETURN jsonb_build_object('exportId',e.id,'segment',a.segment,'segmentVersion',a.segment_version,'purpose',a.purpose,'members',tokens,'withheld',held);
END $$;
-- Partner recommendation (signature and field allowlist verified by the caller and again here): advisory insight only.
CREATE FUNCTION analytics.partner_recommendation(p_partner text,p_message text,p_digest text,p_document jsonb) RETURNS jsonb
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,analytics AS $$
DECLARE jp analytics.journey_partners;prior analytics.partner_messages;m analytics.audience_members;e uuid;result jsonb;insight uuid;BEGIN
 SELECT * INTO jp FROM analytics.journey_partners WHERE code=p_partner;IF NOT FOUND THEN RAISE EXCEPTION 'Partner unavailable' USING ERRCODE='P0002'; END IF;
 PERFORM pg_advisory_xact_lock(hashtext('analytics.partner:'||p_message));
 SELECT * INTO prior FROM analytics.partner_messages WHERE message_id=p_message;
 IF FOUND THEN
  IF prior.digest<>p_digest OR prior.partner<>p_partner THEN RAISE EXCEPTION 'Message identity conflict'; END IF;
  RETURN prior.response;
 END IF;
 IF EXISTS(SELECT 1 FROM jsonb_object_keys(p_document) k WHERE k NOT IN ('messageId','memberToken','recommendationCode','title','body','actionRoute')) THEN
  result:=jsonb_build_object('result','REJECTED','reason','FIELDS_NOT_PERMITTED');
 ELSIF NOT (p_document->>'recommendationCode')=ANY(jp.recommendation_codes) THEN
  result:=jsonb_build_object('result','REJECTED','reason','RECOMMENDATION_NOT_PERMITTED');
 ELSIF p_document ? 'actionRoute' AND NOT (p_document->>'actionRoute')=ANY(jp.action_routes) THEN
  result:=jsonb_build_object('result','REJECTED','reason','ACTION_NOT_PERMITTED');
 ELSIF length(coalesce(p_document->>'title','')) NOT BETWEEN 3 AND 80 OR length(coalesce(p_document->>'body','')) NOT BETWEEN 3 AND 400
  OR (p_document->>'title')||(p_document->>'body') ~ '[<>]' THEN
  result:=jsonb_build_object('result','REJECTED','reason','CONTENT_NOT_PERMITTED');
 ELSE
  SELECT x.export_id INTO e FROM analytics.export_items x JOIN analytics.audience_exports ae ON ae.id=x.export_id
   WHERE x.member_token=p_document->>'memberToken' AND ae.partner=p_partner ORDER BY ae.exported_at DESC LIMIT 1;
  SELECT * INTO m FROM analytics.audience_members WHERE member_token=p_document->>'memberToken';
  IF e IS NULL OR m.party_id IS NULL THEN result:=jsonb_build_object('result','REJECTED','reason','UNKNOWN_MEMBER');
  ELSIF NOT analytics.marketing_permitted(m.party_id) OR analytics.restricted(m.party_id,'MARKETING') OR analytics.restricted(m.party_id,'ANALYTICS') THEN
   result:=jsonb_build_object('result','SUPPRESSED','reason','NO_CURRENT_MARKETING_PERMISSION');
  ELSE
   insight:=gen_random_uuid();
   INSERT INTO analytics.insights(id,party_id,source,code,version,purpose,title,body,action_route,provenance,dedupe_key)
   VALUES(insight,m.party_id,'PARTNER',p_document->>'recommendationCode',1,'MARKETING',p_document->>'title',p_document->>'body',p_document->>'actionRoute',
    jsonb_build_object('partner',p_partner,'exportId',e,'audienceId',m.audience_id,'messageId',p_message),'PARTNER:'||p_partner||':'||p_message);
   result:=jsonb_build_object('result','ACCEPTED','reference',insight);
  END IF;
 END IF;
 result:=result||jsonb_build_object('messageId',p_message);
 INSERT INTO analytics.partner_messages VALUES(p_message,p_partner,p_digest,result->>'result',result);
 RETURN result;
END $$;

-- ===== Customer insights (purpose re-checked at every read; withdrawal hides, never deletes) =====
CREATE FUNCTION analytics.customer_insights(p_actor jsonb,p_person uuid) RETURNS jsonb
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,analytics AS $$
DECLARE permitted boolean;BEGIN
 PERFORM documents.require_periodic_actor(p_actor,p_person,false);
 permitted:=coalesce(data.purpose_enabled(p_person,'MARKETING'),false);
 RETURN jsonb_build_object('marketingPermitted',permitted,
  'insights',(SELECT coalesce(jsonb_agg(jsonb_build_object('id',i.id,'source',i.source,'purpose',i.purpose,'title',i.title,'body',i.body,'actionRoute',i.action_route,
    'why',CASE WHEN i.source='PARTNER' THEN 'A partner you allowed marketing from suggested this ('||(i.provenance->>'partner')||').'
      WHEN i.purpose='MARKETING' THEN 'You allow product suggestions, and your recent account activity matched rule '||i.code||' v'||i.version||'.'
      ELSE 'Service notice: your account activity matched rule '||i.code||' v'||i.version||'.' END,
    'createdAt',analytics.utc(i.created_at)) ORDER BY i.created_at DESC),'[]'::jsonb)
   FROM analytics.insights i WHERE i.party_id IN (SELECT p.id FROM core.parties p JOIN controls.person_links l ON l.issuer=p.issuer AND l.subject=p.subject WHERE l.person_id=p_person)
    AND NOT EXISTS(SELECT 1 FROM analytics.insight_dismissals d WHERE d.insight_id=i.id)
    AND (i.purpose='SERVICE_OUTREACH' OR permitted)));
END $$;
CREATE FUNCTION analytics.dismiss_insight(p_actor jsonb,p_person uuid,p_insight uuid) RETURNS jsonb
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,analytics AS $$
BEGIN
 PERFORM documents.require_periodic_actor(p_actor,p_person,false);
 IF NOT EXISTS(SELECT 1 FROM analytics.insights i JOIN core.parties p ON p.id=i.party_id JOIN controls.person_links l ON l.issuer=p.issuer AND l.subject=p.subject
  WHERE i.id=p_insight AND l.person_id=p_person) THEN RAISE EXCEPTION 'Insight unavailable' USING ERRCODE='P0002'; END IF;
 INSERT INTO analytics.insight_dismissals VALUES(p_insight,p_actor) ON CONFLICT DO NOTHING;
 RETURN analytics.customer_insights(p_actor,p_person);
END $$;

-- ===== Outcome reports: explicit denominators, watermarks and late/missing data =====
CREATE FUNCTION analytics.outcome_report(p_actor jsonb,p_person uuid,p_audience uuid) RETURNS jsonb
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,analytics AS $$
DECLARE a analytics.audiences;target uuid:=gen_random_uuid();cut bigint;den jsonb;outc jsonb;marks jsonb;flags jsonb:='[]'::jsonb;late integer;BEGIN
 SELECT * INTO a FROM analytics.audiences WHERE id=p_audience;IF NOT FOUND THEN RAISE EXCEPTION 'Audience unavailable' USING ERRCODE='P0002'; END IF;
 IF NOT analytics.require_purpose(p_actor,p_person,a.purpose,'report:'||a.segment) THEN RETURN jsonb_build_object('outcome','DENIED','reason','ROLE_NOT_PERMITTED_FOR_PURPOSE'); END IF;
 PERFORM pg_advisory_xact_lock(hashtext('analytics.warehouse'));
 cut:=coalesce((SELECT max(seq) FROM analytics.facts),0);
 den:=jsonb_build_object('candidates',a.candidates,'members',a.members,'excludedAtBuild',a.excluded,
  'exported',coalesce((SELECT sum(exported) FROM analytics.audience_exports WHERE audience_id=a.id),0),
  'withdrawnSinceBuild',(SELECT count(*) FROM analytics.audience_members m WHERE m.audience_id=a.id AND (a.purpose='MARKETING' AND NOT analytics.marketing_permitted(m.party_id))),
  'basis','members (pseudonymous) of audience '||a.id||' built at fact cut '||a.fact_cut);
 outc:=jsonb_build_object(
  'creditedAfterBuild',(SELECT count(DISTINCT m.party_id) FROM analytics.audience_members m JOIN analytics.facts s ON s.dataset='account_states' AND (s.record->>'partyId')::uuid=m.party_id
    JOIN analytics.facts f ON f.dataset='ledger_entries' AND f.account_id=s.account_id AND f.seq>a.fact_cut AND f.seq<=cut AND (f.record->>'amountMinor')::bigint>0
    WHERE m.audience_id=a.id),
  'insightsCreated',(SELECT count(*) FROM analytics.insights i WHERE i.provenance->>'audienceId'=a.id::text),
  'insightsDismissed',(SELECT count(*) FROM analytics.insights i JOIN analytics.insight_dismissals d ON d.insight_id=i.id WHERE i.provenance->>'audienceId'=a.id::text),
  'partnerRecommendationsAccepted',(SELECT count(*) FROM analytics.insights i WHERE i.source='PARTNER' AND i.provenance->>'audienceId'=a.id::text),
  'measure','distinct members with a customer-account credit ingested after the audience was built');
 SELECT coalesce(jsonb_object_agg(d.code,jsonb_build_object('highWater',analytics.utc(w.high_water),'lastRunAt',analytics.utc(w.last_run_at),'checkedAt',analytics.utc(w.checked_at))),'{}'::jsonb)
  INTO marks FROM analytics.datasets d LEFT JOIN analytics.watermarks w ON w.dataset=d.code;
 IF EXISTS(SELECT 1 FROM analytics.datasets d LEFT JOIN analytics.watermarks w ON w.dataset=d.code WHERE w.last_run IS NULL) THEN
  flags:=flags||jsonb_build_object('code','MISSING_SOURCE');
 END IF;
 IF EXISTS(SELECT 1 FROM analytics.datasets d JOIN analytics.watermarks w ON w.dataset=d.code WHERE w.checked_at<clock_timestamp()-d.stale_after) THEN
  flags:=flags||jsonb_build_object('code','STALE_WATERMARK');
 END IF;
 SELECT count(*) INTO late FROM data.transaction_views v WHERE v.recorded_at>=a.built_at
  AND NOT EXISTS(SELECT 1 FROM analytics.facts f WHERE f.dataset='ledger_entries' AND f.seq<=cut AND f.economic_key=v.operation_id||'|'||(v.document->>'postingKind')||'|'||v.account_id);
 IF late>0 THEN flags:=flags||jsonb_build_object('code','LATE_DATA','uningestedRows',late); END IF;
 INSERT INTO analytics.outcome_reports VALUES(target,a.id,cut,den,outc,marks,flags,CASE WHEN jsonb_array_length(flags)=0 THEN 'COMPLETE' ELSE 'INCOMPLETE' END,
  encode(sha256(convert_to(den::text||outc::text||cut,'UTF8')),'hex'),p_actor,p_person);
 PERFORM analytics.audit(p_actor,p_person,'analytics.outcome-report',target::text,'RECORDED',a.purpose);
 RETURN analytics.report_document(target)||jsonb_build_object('outcome','CREATED');
END $$;
CREATE FUNCTION analytics.audience_view(p_actor jsonb,p_person uuid) RETURNS jsonb LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,analytics AS $$
BEGIN
 PERFORM analytics.require(p_actor,p_person,ARRAY(SELECT DISTINCT role FROM analytics.purpose_roles WHERE purpose IN ('MARKETING','SERVICE_OUTREACH')));
 RETURN jsonb_build_object(
  'signals',(SELECT jsonb_agg(jsonb_build_object('code',code,'description',description,'minimization',minimization) ORDER BY code) FROM analytics.signals),
  'segments',(SELECT jsonb_agg(jsonb_build_object('code',code,'version',version,'purpose',purpose,'signals',signals,'definition',definition,'digest',digest) ORDER BY code,version) FROM analytics.segments),
  'partners',(SELECT jsonb_agg(jsonb_build_object('code',code,'status',status,'purposes',purposes,'recommendationCodes',recommendation_codes,'description',description) ORDER BY code) FROM analytics.journey_partners),
  'audiences',(SELECT coalesce(jsonb_agg(x ORDER BY x->>'builtAt' DESC),'[]'::jsonb) FROM (SELECT analytics.audience_document(id) x FROM analytics.audiences ORDER BY built_at DESC LIMIT 10) s),
  'partnerMessages',(SELECT coalesce(jsonb_agg(x ORDER BY x->>'receivedAt' DESC),'[]'::jsonb) FROM (SELECT jsonb_build_object('messageId',message_id,'partner',partner,'result',result,
    'reason',response->>'reason','receivedAt',analytics.utc(received_at)) x FROM analytics.partner_messages ORDER BY received_at DESC LIMIT 10) m));
END $$;

GRANT EXECUTE ON FUNCTION analytics.build_audience(jsonb,uuid,text,integer),analytics.export_audience(jsonb,uuid,uuid,text),analytics.partner_export(text,uuid),
 analytics.partner_recommendation(text,text,text,jsonb),analytics.customer_insights(jsonb,uuid),analytics.dismiss_insight(jsonb,uuid,uuid),
 analytics.outcome_report(jsonb,uuid,uuid),analytics.audience_view(jsonb,uuid) TO bank_data;
