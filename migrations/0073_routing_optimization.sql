-- owners: simulator (second synthetic rail provider SYNTHETIC_RAIL_B), core (append-only external route log, route function,
-- finality bound to the current route's provider), payments (per-attempt provider work and receipts, governed routing
-- policies with ACTIVE/SHADOW modes, recorded route decisions, provider health and shadow comparison). WP-39 template
-- (user decision 2026-11-18): two interchangeable simulated providers; re-routing only on the current provider's signed
-- non-execution; Core books at most once and only from the current route. Real-baseline benefit stays PENDING.

-- ===== Simulator: provider B (separate state, same protocol) =====
CREATE TABLE simulator.rail_b_instructions(
 id uuid PRIMARY KEY,fingerprint text NOT NULL,instruction jsonb NOT NULL,state text NOT NULL CHECK(state IN ('ACCEPTED','PENDING','SETTLED','REJECTED','CANCELLED')),
 version bigint NOT NULL,evidence_id uuid NOT NULL,created_at timestamptz NOT NULL DEFAULT now()
);
CREATE TABLE simulator.rail_b_events(id uuid PRIMARY KEY,instruction_id uuid NOT NULL REFERENCES simulator.rail_b_instructions(id),version bigint NOT NULL,document jsonb NOT NULL,UNIQUE(instruction_id,version));
CREATE TRIGGER immutable_rail_b_events BEFORE UPDATE OR DELETE OR TRUNCATE ON simulator.rail_b_events FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal();
CREATE TRIGGER immutable_rail_b_identity BEFORE UPDATE OR DELETE ON simulator.rail_b_instructions FOR EACH ROW EXECUTE FUNCTION simulator.guard_rail_identity();
CREATE TRIGGER immutable_rail_b_set BEFORE TRUNCATE ON simulator.rail_b_instructions FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal();
CREATE UNIQUE INDEX rail_b_event_head_scope ON simulator.rail_b_events(id,instruction_id,version);
ALTER TABLE simulator.rail_b_instructions ADD CONSTRAINT rail_b_current_evidence_scope
FOREIGN KEY(evidence_id,id,version) REFERENCES simulator.rail_b_events(id,instruction_id,version) DEFERRABLE INITIALLY DEFERRED;
ALTER TABLE simulator.rail_scenarios DROP CONSTRAINT rail_scenarios_mode_check;
ALTER TABLE simulator.rail_scenarios ADD CONSTRAINT rail_scenarios_mode_check CHECK(mode IN ('ACCEPT','REJECT','REJECT_CAPACITY','LOST_RESPONSE','BEFORE_ACCEPT_TIMEOUT','CONFLICT'));
CREATE TABLE simulator.rail_b_scenarios(instruction_id uuid PRIMARY KEY,mode text NOT NULL CHECK(mode IN ('ACCEPT','REJECT','REJECT_CAPACITY','LOST_RESPONSE','BEFORE_ACCEPT_TIMEOUT','CONFLICT')));

-- ===== Core: append-only route log; non-execution proof required to re-route =====
CREATE TABLE core.external_routes(operation_id uuid NOT NULL REFERENCES core.external_reservations(operation_id),attempt integer NOT NULL CHECK(attempt BETWEEN 1 AND 4),
 provider text NOT NULL CHECK(provider IN ('SYNTHETIC_RAIL','SYNTHETIC_RAIL_B')),nonexecution_evidence uuid UNIQUE,nonexecution_digest text,document jsonb,
 recorded_at timestamptz NOT NULL DEFAULT now(),PRIMARY KEY(operation_id,attempt),UNIQUE(operation_id,provider),CHECK((attempt=1)=(nonexecution_evidence IS NULL)));
CREATE TRIGGER immutable_external_routes BEFORE UPDATE OR DELETE OR TRUNCATE ON core.external_routes FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal();
REVOKE ALL ON core.external_routes FROM bank_core;
GRANT SELECT ON core.external_routes TO bank_core;
-- Signed REJECTED evidence with one of these reasons proves the provider did not execute the instruction.
CREATE FUNCTION core.nonexecution_reasons() RETURNS text[] LANGUAGE sql IMMUTABLE SET search_path=pg_catalog AS
$$ SELECT ARRAY['PROVIDER_CAPACITY_NOT_EXECUTED','PROVIDER_UNAVAILABLE_NOT_EXECUTED'] $$;
CREATE FUNCTION core.route_external(p_operation uuid,p_digest text,p_provider text,p_packet jsonb,p_evidence_digest text)
RETURNS TABLE(provider text,attempt integer,created boolean)
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,core AS $$
DECLARE r core.external_reservations;h core.holds;cur core.external_routes;e jsonb:=p_packet->'body';prior core.external_routes;BEGIN
 PERFORM pg_advisory_xact_lock(hashtextextended(p_operation::text,0));
 IF p_provider NOT IN ('SYNTHETIC_RAIL','SYNTHETIC_RAIL_B') THEN RAISE EXCEPTION 'Unknown rail provider'; END IF;
 SELECT * INTO STRICT r FROM core.external_reservations WHERE operation_id=p_operation AND request_digest=p_digest;
 IF EXISTS(SELECT 1 FROM core.external_finalities WHERE operation_id=p_operation) THEN RAISE EXCEPTION 'Instruction already final'; END IF;
 SELECT * INTO STRICT h FROM core.holds WHERE operation_id=p_operation FOR UPDATE;
 SELECT * INTO cur FROM core.external_routes x WHERE x.operation_id=p_operation ORDER BY x.attempt DESC LIMIT 1;
 IF cur.operation_id IS NULL AND h.submitted THEN
  -- Instructions submitted before WP-39 were routed to the original provider.
  INSERT INTO core.external_routes(operation_id,attempt,provider) VALUES(p_operation,1,'SYNTHETIC_RAIL') RETURNING * INTO cur;
 END IF;
 IF p_packet IS NULL THEN
  IF cur.operation_id IS NOT NULL THEN RETURN QUERY SELECT cur.provider,cur.attempt,false; RETURN; END IF;
  IF h.state<>'ACTIVE' THEN RAISE EXCEPTION 'Active reservation required'; END IF;
  INSERT INTO core.external_routes(operation_id,attempt,provider) VALUES(p_operation,1,p_provider) RETURNING * INTO cur;
  RETURN QUERY SELECT cur.provider,cur.attempt,true; RETURN;
 END IF;
 SELECT * INTO prior FROM core.external_routes x WHERE x.operation_id=p_operation AND x.nonexecution_evidence=(e->>'evidenceId')::uuid;
 IF FOUND THEN
  IF prior.provider<>p_provider OR prior.nonexecution_digest<>p_evidence_digest THEN RAISE EXCEPTION 'Conflicting re-route'; END IF;
  RETURN QUERY SELECT prior.provider,prior.attempt,false; RETURN;
 END IF;
 IF cur.operation_id IS NULL OR (e->>'provider') IS DISTINCT FROM cur.provider OR (e->>'instructionId') IS DISTINCT FROM p_operation::text
  OR (e->>'paymentId') IS DISTINCT FROM p_operation::text OR (e->>'inputDigest') IS DISTINCT FROM p_digest OR (e->>'status') IS DISTINCT FROM 'REJECTED'
  OR NOT coalesce(e->>'reasonCode','')=ANY(core.nonexecution_reasons()) OR e->>'settlementDate' IS NOT NULL OR e->>'evidenceId' IS NULL
  OR (e->'amount') IS DISTINCT FROM (r.instruction->'details'->'amount') THEN
  RAISE EXCEPTION 'Authenticated non-execution by the current provider required';
 END IF;
 IF h.state<>'ACTIVE' OR NOT h.submitted THEN RAISE EXCEPTION 'Original submitted reservation required'; END IF;
 IF EXISTS(SELECT 1 FROM core.external_routes x WHERE x.operation_id=p_operation AND x.provider=p_provider) THEN RAISE EXCEPTION 'Provider already attempted'; END IF;
 INSERT INTO core.external_routes VALUES(p_operation,cur.attempt+1,p_provider,(e->>'evidenceId')::uuid,p_evidence_digest,p_packet,now()) RETURNING * INTO cur;
 RETURN QUERY SELECT cur.provider,cur.attempt,true;
END $$;
GRANT EXECUTE ON FUNCTION core.route_external(uuid,text,text,jsonb,text) TO bank_core;

-- Final evidence must come from the current route's provider.
CREATE OR REPLACE FUNCTION core.finalize_external(p_operation uuid,p_digest text,p_packet jsonb,p_evidence_digest text)
RETURNS TABLE(outcome text,batch_id uuid,created boolean)
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,core AS $$
DECLARE r core.external_reservations; h core.holds; previous core.external_finalities; e jsonb:=p_packet->'body';
DECLARE payer uuid; clearing uuid:='4f1a780b-3e31-48ef-a294-000000000003'; fees uuid:='4f1a780b-3e31-48ef-a294-000000000004';
DECLARE principal bigint; fee bigint; total bigint; target uuid:=NULL; final_state text; day date; product uuid; account core.ledger_accounts;
BEGIN
 PERFORM pg_advisory_xact_lock(hashtextextended(p_operation::text,0));
 SELECT * INTO previous FROM core.external_finalities WHERE operation_id=p_operation;
 IF FOUND THEN
  IF previous.request_digest<>p_digest OR previous.evidence_digest<>p_evidence_digest THEN RAISE EXCEPTION 'Conflicting immutable financial finality'; END IF;
  RETURN QUERY SELECT previous.outcome,previous.batch_id,false; RETURN;
 END IF;
 SELECT * INTO STRICT r FROM core.external_reservations WHERE operation_id=p_operation AND request_digest=p_digest;
 payer:=(r.instruction->'details'->>'payerAccountId')::uuid;
 principal:=(r.instruction->'details'->'amount'->>'minorUnits')::bigint;
 fee:=(r.instruction->'details'->'fee'->>'minorUnits')::bigint;total:=principal+fee;
 day:=(r.instruction->'details'->>'requestedDate')::date;product:=(r.instruction->>'productVersionId')::uuid;
 -- WP-39: final evidence is accepted only from the provider of the current route (legacy instructions: SYNTHETIC_RAIL).
 IF (e->>'provider') IS DISTINCT FROM coalesce((SELECT x.provider FROM core.external_routes x WHERE x.operation_id=p_operation ORDER BY x.attempt DESC LIMIT 1),'SYNTHETIC_RAIL') OR (e->>'instructionId') IS DISTINCT FROM p_operation::text
 OR (e->>'paymentId') IS DISTINCT FROM p_operation::text OR (e->>'inputDigest') IS DISTINCT FROM p_digest
 OR (e->'amount') IS DISTINCT FROM (r.instruction->'details'->'amount') OR coalesce((e->>'version')::bigint,0)<=0
 OR e->>'status' IS NULL OR e->>'status' NOT IN ('SETTLED','REJECTED','CANCELLED') OR e->>'evidenceId' IS NULL
 THEN RAISE EXCEPTION 'Exact authenticated final financial evidence required'; END IF;
 PERFORM 1 FROM core.ledger_accounts WHERE id IN(payer,clearing) OR (id=fees AND fee>0) ORDER BY id FOR UPDATE;
 SELECT * INTO STRICT h FROM core.holds WHERE operation_id=p_operation FOR UPDATE;
 IF h.state<>'ACTIVE' OR NOT h.submitted OR h.account_id<>payer OR h.amount<>total THEN RAISE EXCEPTION 'Original submitted reservation required'; END IF;
 IF e->>'status'='SETTLED' THEN
  IF e->>'settlementDate' IS NULL OR (e->>'settlementDate')::date<day OR (e->>'settlementDate')::date>(clock_timestamp() AT TIME ZONE 'UTC')::date THEN RAISE EXCEPTION 'Invalid authenticated settlement date'; END IF;
  -- This exact, previously authorized submitted hold remains payable after a subsequent debit freeze.
  SELECT * INTO STRICT account FROM core.ledger_accounts WHERE id=payer;
  IF account.kind<>'CUSTOMER' OR account.state='CLOSED' OR account.held<total OR account.booked<total THEN RAISE EXCEPTION 'Reserved payer funds unavailable'; END IF;
  IF EXISTS(SELECT 1 FROM core.ledger_accounts WHERE (id=clearing OR (id=fees AND fee>0)) AND (state<>'ACTIVE' OR credit_blocked)) THEN RAISE EXCEPTION 'Settlement control account unavailable'; END IF;
  target:=gen_random_uuid();
  INSERT INTO core.journal_batches(id,operation_id,posting_kind,fingerprint,business_date) VALUES(target,p_operation,'EXTERNAL_PAYMENT',encode(sha256(convert_to(jsonb_build_object('requestDigest',p_digest,'evidenceDigest',p_evidence_digest)::text,'UTF8')),'hex'),(e->>'settlementDate')::date);
  UPDATE core.ledger_accounts SET booked=booked-total,held=held-total,version=version+1 WHERE id=payer;
  UPDATE core.holds SET state='CONSUMED',version=version+1,updated_at=now() WHERE operation_id=p_operation;
  UPDATE core.ledger_accounts SET booked=booked+principal,version=version+1 WHERE id=clearing;
  INSERT INTO core.journal_entries VALUES(target,payer,-total),(target,clearing,principal);
  IF fee>0 THEN
   UPDATE core.ledger_accounts SET booked=booked+fee,version=version+1 WHERE id=fees;
   INSERT INTO core.journal_entries VALUES(target,fees,fee);
  END IF;
  INSERT INTO core.payment_executions VALUES(p_operation,payer,clearing,principal,fee,product,day,p_digest,target,now());
  UPDATE core.payment_limit_usage SET state='POSTED' WHERE operation_id=p_operation AND state='ACTIVE';
  final_state:='SETTLED';
 ELSE
  IF e->>'settlementDate' IS NOT NULL THEN RAISE EXCEPTION 'Nonexecution cannot claim settlement'; END IF;
  PERFORM core.release_hold(p_operation,(e->>'evidenceId')::uuid);
  UPDATE core.payment_limit_usage SET state='RELEASED' WHERE operation_id=p_operation AND state='ACTIVE';
  final_state:=CASE WHEN e->>'status'='REJECTED' THEN 'DECLINED' ELSE 'CANCELLED' END;
 END IF;
 UPDATE core.obligations SET state='RESOLVED' WHERE operation_id=p_operation AND state='OPEN';
 INSERT INTO core.external_finalities(operation_id,request_digest,outcome,evidence_id,evidence_digest,document,batch_id) VALUES(p_operation,p_digest,final_state,(e->>'evidenceId')::uuid,p_evidence_digest,p_packet,target);
 RETURN QUERY SELECT final_state,target,true;
END $$;

-- ===== Payments: per-attempt provider work and receipts =====
ALTER TABLE payments.rail_submissions ADD COLUMN provider text NOT NULL DEFAULT 'SYNTHETIC_RAIL' CHECK(provider IN ('SYNTHETIC_RAIL','SYNTHETIC_RAIL_B')),
 ADD COLUMN route_attempt integer NOT NULL DEFAULT 1 CHECK(route_attempt BETWEEN 1 AND 4);
-- A re-route advances the attempt by one, changes provider and starts a fresh provider attempt; nothing else may change.
CREATE OR REPLACE FUNCTION payments.guard_rail_work() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE rerouted boolean;BEGIN
 IF TG_OP<>'UPDATE' THEN RAISE EXCEPTION 'Immutable provider work identity'; END IF;
 rerouted:=NEW.route_attempt=OLD.route_attempt+1 AND NEW.provider<>OLD.provider AND NEW.attempted_at IS NULL AND NEW.lease_id IS NULL;
 IF NEW.payment_id<>OLD.payment_id OR NEW.instruction_id<>OLD.instruction_id OR NEW.document<>OLD.document OR NEW.fingerprint<>OLD.fingerprint OR NEW.created_at<>OLD.created_at
 OR (NOT rerouted AND (NEW.route_attempt<>OLD.route_attempt OR NEW.provider<>OLD.provider))
 OR (OLD.attempted_at IS NOT NULL AND NEW.attempted_at IS DISTINCT FROM OLD.attempted_at AND NOT rerouted)
 OR NEW.attempts<OLD.attempts OR NEW.inquiries<OLD.inquiries OR NEW.cancellations<OLD.cancellations OR NEW.lease_fence<OLD.lease_fence THEN RAISE EXCEPTION 'Immutable provider work identity'; END IF;
 RETURN NEW;
END $$;
ALTER TABLE payments.rail_receipts ADD COLUMN route_attempt integer NOT NULL DEFAULT 1,ADD COLUMN provider text NOT NULL DEFAULT 'SYNTHETIC_RAIL';
ALTER TABLE payments.rail_receipts DROP CONSTRAINT rail_receipts_payment_id_provider_version_key;
CREATE UNIQUE INDEX rail_receipts_attempt_version ON payments.rail_receipts(payment_id,route_attempt,provider_version);

-- ===== Routing policies (maker/checker, ACTIVE and SHADOW), decisions, health =====
CREATE TABLE payments.route_policies(version integer PRIMARY KEY CHECK(version>0),document jsonb NOT NULL,digest text NOT NULL,proposed_by jsonb,proposed_person uuid,
 created_at timestamptz NOT NULL DEFAULT clock_timestamp());
INSERT INTO payments.route_policies(version,document,digest) SELECT 1,d,encode(sha256(convert_to(d::text,'UTF8')),'hex') FROM (SELECT
 '{"strategy":"PRIMARY_FALLBACK","providers":["SYNTHETIC_RAIL","SYNTHETIC_RAIL_B"],"fallbackReasons":["PROVIDER_CAPACITY_NOT_EXECUTED","PROVIDER_UNAVAILABLE_NOT_EXECUTED"],"maxAttempts":2,"health":{"windowMinutes":60,"minSamples":5,"maxUnknownRate":0.2,"maxNonexecutionRate":0.5},"feesMinor":{"SYNTHETIC_RAIL":0,"SYNTHETIC_RAIL_B":0},"promotionMinDecisions":3}'::jsonb d) x;
CREATE TABLE payments.route_policy_head(id boolean PRIMARY KEY DEFAULT true CHECK(id),active_version integer NOT NULL REFERENCES payments.route_policies,
 shadow_version integer REFERENCES payments.route_policies,revision bigint NOT NULL,updated_at timestamptz NOT NULL DEFAULT clock_timestamp());
INSERT INTO payments.route_policy_head(active_version,revision) VALUES(1,1);
CREATE TABLE payments.route_policy_changes(id uuid PRIMARY KEY,version integer NOT NULL REFERENCES payments.route_policies,mode text NOT NULL CHECK(mode IN ('ACTIVE','SHADOW','SHADOW_OFF')),
 evidence jsonb NOT NULL,proposed_by jsonb NOT NULL,proposed_person uuid NOT NULL,created_at timestamptz NOT NULL DEFAULT clock_timestamp());
CREATE TABLE payments.route_policy_decisions(change_id uuid PRIMARY KEY REFERENCES payments.route_policy_changes,outcome text NOT NULL CHECK(outcome IN ('APPLIED','REJECTED')),
 head_revision bigint,decided_by jsonb NOT NULL,decided_person uuid NOT NULL,decided_at timestamptz NOT NULL DEFAULT clock_timestamp());
CREATE TABLE payments.route_decisions(payment_id uuid NOT NULL REFERENCES payments.intents(id),attempt integer NOT NULL,provider text NOT NULL,policy_version integer NOT NULL REFERENCES payments.route_policies,
 strategy text NOT NULL,reason jsonb NOT NULL,shadow_version integer REFERENCES payments.route_policies,shadow_provider text,decided_at timestamptz NOT NULL DEFAULT clock_timestamp(),PRIMARY KEY(payment_id,attempt));
DO $$ DECLARE t text;BEGIN
 FOREACH t IN ARRAY ARRAY['route_policies','route_policy_changes','route_policy_decisions','route_decisions'] LOOP
  EXECUTE format('CREATE TRIGGER immutable_%s BEFORE UPDATE OR DELETE OR TRUNCATE ON payments.%I FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal()',t,t);
 END LOOP;
END $$;
REVOKE ALL ON payments.route_policies,payments.route_policy_head,payments.route_policy_changes,payments.route_policy_decisions FROM bank_payments;
GRANT SELECT ON payments.route_policies,payments.route_policy_head,payments.route_decisions TO bank_payments;
REVOKE UPDATE,DELETE,TRUNCATE ON payments.route_decisions FROM bank_payments;

CREATE FUNCTION payments.route_audit(p_actor jsonb,p_person uuid,p_action text,p_resource text,p_outcome text,p_reason text) RETURNS void
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog AS $$ BEGIN
 PERFORM controls.append_audit(gen_random_uuid(),'routing',jsonb_build_object('actorIssuer',coalesce(p_actor->>'issuer','urn:piggybank:worker'),
  'actorSubject',coalesce(p_actor->>'subject','payment-routing'),'actorPersonId',p_person,'representedParty',NULL,'action',p_action,'resource',p_resource,
  'outcome',p_outcome,'policyVersion','routing-v1','reasonCode',p_reason,'correlationId',gen_random_uuid(),
  'occurredAt',to_char(clock_timestamp() AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"')));
END $$;
-- Measured health per provider over a window, from recorded decisions and their attempt-scoped receipts.
CREATE FUNCTION payments.provider_health(p_window interval) RETURNS jsonb LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
 WITH d AS (SELECT r.payment_id,r.attempt,r.provider,r.decided_at,
   (SELECT min(x.recorded_at) FROM payments.rail_receipts x WHERE x.payment_id=r.payment_id AND x.route_attempt=r.attempt) first_receipt,
   (SELECT x.document->'body'->>'status' FROM payments.rail_receipts x WHERE x.payment_id=r.payment_id AND x.route_attempt=r.attempt ORDER BY x.provider_version DESC LIMIT 1) status,
   (SELECT x.document->'body'->>'reasonCode' FROM payments.rail_receipts x WHERE x.payment_id=r.payment_id AND x.route_attempt=r.attempt ORDER BY x.provider_version DESC LIMIT 1) reason
  FROM payments.route_decisions r WHERE r.decided_at>clock_timestamp()-p_window)
 SELECT coalesce(jsonb_object_agg(p.provider,jsonb_build_object(
   'decisions',(SELECT count(*) FROM d WHERE d.provider=p.provider),
   'settled',(SELECT count(*) FROM d WHERE d.provider=p.provider AND d.status='SETTLED'),
   'nonExecuted',(SELECT count(*) FROM d WHERE d.provider=p.provider AND d.status='REJECTED' AND d.reason=ANY(core.nonexecution_reasons())),
   'rejected',(SELECT count(*) FROM d WHERE d.provider=p.provider AND d.status='REJECTED'),
   'unknown',(SELECT count(*) FROM d WHERE d.provider=p.provider AND d.status IS NULL AND d.decided_at<clock_timestamp()-interval '2 minutes'),
   'avgAckMs',(SELECT round(avg(extract(epoch FROM d.first_receipt-d.decided_at)*1000)) FROM d WHERE d.provider=p.provider AND d.first_receipt IS NOT NULL))),'{}'::jsonb)
 FROM (VALUES('SYNTHETIC_RAIL'),('SYNTHETIC_RAIL_B')) p(provider) $$;
CREATE FUNCTION payments.route_choose(p_version integer,p_excluded text[]) RETURNS jsonb LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE d jsonb;health jsonb;h jsonb;candidate text;best text;best_score numeric;score numeric;n numeric;scores jsonb:='{}'::jsonb;BEGIN
 SELECT document INTO STRICT d FROM payments.route_policies WHERE version=p_version;
 IF d->>'strategy'='PRIMARY_FALLBACK' THEN
  FOR candidate IN SELECT jsonb_array_elements_text(d->'providers') LOOP
   IF NOT candidate=ANY(coalesce(p_excluded,'{}')) THEN RETURN jsonb_build_object('provider',candidate,'rule','FIRST_IN_ORDER'); END IF;
  END LOOP;
  RETURN jsonb_build_object('provider',NULL,'rule','NO_PROVIDER_LEFT');
 END IF;
 health:=payments.provider_health(make_interval(mins=>(d->'health'->>'windowMinutes')::integer));
 FOR candidate IN SELECT jsonb_array_elements_text(d->'providers') LOOP
  CONTINUE WHEN candidate=ANY(coalesce(p_excluded,'{}'));
  h:=health->candidate;n:=greatest((h->>'decisions')::numeric,1);
  IF (h->>'decisions')::integer<(d->'health'->>'minSamples')::integer THEN score:=0;
  ELSIF (h->>'unknown')::numeric/n>(d->'health'->>'maxUnknownRate')::numeric OR (h->>'nonExecuted')::numeric/n>(d->'health'->>'maxNonexecutionRate')::numeric THEN score:=-1;
  ELSE score:=((h->>'settled')::numeric-(h->>'unknown')::numeric-(h->>'nonExecuted')::numeric)/n; END IF;
  scores:=scores||jsonb_build_object(candidate,round(score,4));
  IF best IS NULL OR score>best_score THEN best:=candidate;best_score:=score; END IF;
 END LOOP;
 RETURN jsonb_build_object('provider',best,'rule',CASE WHEN best IS NULL THEN 'NO_PROVIDER_LEFT' WHEN best_score<0 THEN 'NO_HEALTHY_PROVIDER_BEST_AVAILABLE' ELSE 'BEST_MEASURED_HEALTH' END,'scores',scores,'health',health);
END $$;
-- One decision per payment attempt. Attempt 1 follows the active policy (and records the shadow choice); later attempts
-- follow the policy version of attempt 1 so a rollback never changes an in-flight instruction's fallback rules.
CREATE FUNCTION payments.route_decide(p_payment uuid,p_attempt integer,p_excluded text[]) RETURNS jsonb
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE prior payments.route_decisions;head payments.route_policy_head;v integer;choice jsonb;shadow jsonb;d jsonb;BEGIN
 SELECT * INTO prior FROM payments.route_decisions WHERE payment_id=p_payment AND attempt=p_attempt;
 IF FOUND THEN RETURN jsonb_build_object('provider',prior.provider,'policyVersion',prior.policy_version,'attempt',prior.attempt); END IF;
 SELECT * INTO STRICT head FROM payments.route_policy_head;
 v:=CASE WHEN p_attempt=1 THEN head.active_version ELSE (SELECT policy_version FROM payments.route_decisions WHERE payment_id=p_payment AND attempt=1) END;
 v:=coalesce(v,head.active_version);
 SELECT document INTO STRICT d FROM payments.route_policies WHERE version=v;
 IF p_attempt>(d->>'maxAttempts')::integer THEN RETURN jsonb_build_object('provider',NULL,'policyVersion',v,'attempt',p_attempt,'rule','MAX_ATTEMPTS'); END IF;
 choice:=payments.route_choose(v,p_excluded);
 IF choice->>'provider' IS NULL THEN RETURN choice||jsonb_build_object('policyVersion',v,'attempt',p_attempt); END IF;
 IF p_attempt=1 AND head.shadow_version IS NOT NULL THEN shadow:=payments.route_choose(head.shadow_version,p_excluded); END IF;
 INSERT INTO payments.route_decisions VALUES(p_payment,p_attempt,choice->>'provider',v,d->>'strategy',choice||jsonb_build_object('excluded',to_jsonb(coalesce(p_excluded,'{}'))),
  CASE WHEN shadow IS NOT NULL THEN head.shadow_version END,shadow->>'provider');
 RETURN jsonb_build_object('provider',choice->>'provider','policyVersion',v,'attempt',p_attempt);
END $$;
CREATE FUNCTION payments.route_fallback_allowed(p_payment uuid,p_reason text,p_attempt integer) RETURNS boolean LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
 SELECT coalesce((SELECT p_reason=ANY(core.nonexecution_reasons()) AND p.document->'fallbackReasons' ? p_reason AND p_attempt<(p.document->>'maxAttempts')::integer
   FROM payments.route_decisions r JOIN payments.route_policies p ON p.version=r.policy_version WHERE r.payment_id=p_payment AND r.attempt=1),
  -- Legacy instructions without a recorded decision follow the active policy.
  (SELECT p_reason=ANY(core.nonexecution_reasons()) AND p.document->'fallbackReasons' ? p_reason AND p_attempt<(p.document->>'maxAttempts')::integer
   FROM payments.route_policy_head h JOIN payments.route_policies p ON p.version=h.active_version)) $$;
-- Policy documents: only known keys and providers; fallback only on Core-recognised non-execution reasons.
CREATE FUNCTION payments.valid_route_policy(p_document jsonb) RETURNS boolean LANGUAGE sql IMMUTABLE SET search_path=pg_catalog AS $$
 SELECT jsonb_typeof(p_document)='object'
  AND NOT EXISTS(SELECT 1 FROM jsonb_object_keys(p_document) k WHERE k NOT IN ('strategy','providers','fallbackReasons','maxAttempts','health','feesMinor','promotionMinDecisions'))
  AND p_document->>'strategy' IN ('PRIMARY_FALLBACK','HEALTH_WEIGHTED')
  AND jsonb_typeof(p_document->'providers')='array' AND jsonb_array_length(p_document->'providers') BETWEEN 1 AND 2
  AND NOT EXISTS(SELECT 1 FROM jsonb_array_elements_text(p_document->'providers') x WHERE x NOT IN ('SYNTHETIC_RAIL','SYNTHETIC_RAIL_B'))
  AND (SELECT count(DISTINCT x) FROM jsonb_array_elements_text(p_document->'providers') x)=jsonb_array_length(p_document->'providers')
  AND jsonb_typeof(p_document->'fallbackReasons')='array'
  AND NOT EXISTS(SELECT 1 FROM jsonb_array_elements_text(p_document->'fallbackReasons') x WHERE NOT x=ANY(core.nonexecution_reasons()))
  AND jsonb_typeof(p_document->'maxAttempts')='number' AND (p_document->>'maxAttempts')::integer BETWEEN 1 AND jsonb_array_length(p_document->'providers')
  AND jsonb_typeof(p_document->'health')='object' AND NOT EXISTS(SELECT 1 FROM jsonb_object_keys(p_document->'health') k WHERE k NOT IN ('windowMinutes','minSamples','maxUnknownRate','maxNonexecutionRate'))
  AND (p_document->'health'->>'windowMinutes')::integer BETWEEN 5 AND 1440 AND (p_document->'health'->>'minSamples')::integer BETWEEN 1 AND 1000
  AND (p_document->'health'->>'maxUnknownRate')::numeric BETWEEN 0 AND 1 AND (p_document->'health'->>'maxNonexecutionRate')::numeric BETWEEN 0 AND 1
  AND jsonb_typeof(p_document->'feesMinor')='object' AND NOT EXISTS(SELECT 1 FROM jsonb_each(p_document->'feesMinor') f WHERE f.key NOT IN ('SYNTHETIC_RAIL','SYNTHETIC_RAIL_B') OR jsonb_typeof(f.value)<>'number' OR (f.value#>>'{}')::numeric<0)
  AND (p_document->>'promotionMinDecisions')::integer BETWEEN 1 AND 10000 $$;
-- Shadow comparison for one version: agreement with the active choices and measured outcomes/fees/latency per provider.
CREATE FUNCTION payments.route_shadow_report(p_version integer) RETURNS jsonb LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
 SELECT jsonb_build_object('version',p_version,'decisions',count(*),'agreements',count(*) FILTER (WHERE r.provider=r.shadow_provider),
  'agreementRate',CASE WHEN count(*)=0 THEN NULL ELSE round(count(*) FILTER (WHERE r.provider=r.shadow_provider)::numeric/count(*),4) END,
  'activeChoices',(SELECT coalesce(jsonb_object_agg(provider,n),'{}'::jsonb) FROM (SELECT provider,count(*) n FROM payments.route_decisions WHERE shadow_version=p_version AND attempt=1 GROUP BY 1) a),
  'shadowChoices',(SELECT coalesce(jsonb_object_agg(shadow_provider,n),'{}'::jsonb) FROM (SELECT shadow_provider,count(*) n FROM payments.route_decisions WHERE shadow_version=p_version AND attempt=1 GROUP BY 1) a),
  'feesMinor',(SELECT document->'feesMinor' FROM payments.route_policies WHERE version=p_version),
  'providerHealth',payments.provider_health(interval '7 days'),
  'baseline','LOCAL_SIMULATOR_ONLY: real-provider benefit requires production evidence (DEC-03/04/07/11)')
 FROM payments.route_decisions r WHERE r.shadow_version=p_version AND r.attempt=1 $$;
CREATE FUNCTION payments.propose_route_change(p_actor jsonb,p_person uuid,p_document jsonb,p_version integer,p_mode text) RETURNS jsonb
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE v integer:=p_version;target uuid:=gen_random_uuid();BEGIN
 PERFORM controls.require_staff(p_actor,p_person,ARRAY['payment_operator']);
 IF p_mode NOT IN ('ACTIVE','SHADOW','SHADOW_OFF') THEN RAISE EXCEPTION 'Unknown routing mode' USING ERRCODE='22023'; END IF;
 PERFORM pg_advisory_xact_lock(hashtext('payments.route_policy'));
 IF p_document IS NOT NULL THEN
  IF p_version IS NOT NULL OR NOT payments.valid_route_policy(p_document) THEN RAISE EXCEPTION 'Invalid routing policy document' USING ERRCODE='22023'; END IF;
  v:=(SELECT max(version)+1 FROM payments.route_policies);
  INSERT INTO payments.route_policies VALUES(v,p_document,encode(sha256(convert_to(p_document::text,'UTF8')),'hex'),p_actor,p_person);
 ELSIF NOT EXISTS(SELECT 1 FROM payments.route_policies WHERE version=v) THEN RAISE EXCEPTION 'Routing policy unavailable' USING ERRCODE='P0002';
 END IF;
 INSERT INTO payments.route_policy_changes VALUES(target,v,p_mode,payments.route_shadow_report(v),p_actor,p_person);
 PERFORM payments.route_audit(p_actor,p_person,'routing.change-proposed',target::text,'RECORDED',p_mode);
 RETURN jsonb_build_object('changeId',target,'version',v,'mode',p_mode);
END $$;
-- Independent approval; activation needs prior shadow evidence unless the version was active before (rollback).
CREATE FUNCTION payments.decide_route_change(p_actor jsonb,p_person uuid,p_change uuid,p_approve boolean) RETURNS jsonb
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE c payments.route_policy_changes;h payments.route_policy_head;d jsonb;shadow jsonb;was_active boolean;BEGIN
 PERFORM controls.require_staff(p_actor,p_person,ARRAY['payment_approver']);
 PERFORM pg_advisory_xact_lock(hashtext('payments.route_policy'));
 SELECT * INTO c FROM payments.route_policy_changes WHERE id=p_change;IF NOT FOUND THEN RAISE EXCEPTION 'Change unavailable' USING ERRCODE='P0002'; END IF;
 IF EXISTS(SELECT 1 FROM payments.route_policy_decisions WHERE change_id=p_change) THEN RAISE EXCEPTION 'Change already decided'; END IF;
 IF c.proposed_person=p_person THEN RAISE EXCEPTION 'Independent approver required'; END IF;
 SELECT * INTO STRICT h FROM payments.route_policy_head FOR UPDATE;
 IF NOT p_approve THEN
  INSERT INTO payments.route_policy_decisions VALUES(p_change,'REJECTED',NULL,p_actor,p_person);
  PERFORM payments.route_audit(p_actor,p_person,'routing.change-decided',p_change::text,'REJECTED','APPROVER_REJECTED');
  RETURN jsonb_build_object('changeId',p_change,'outcome','REJECTED');
 END IF;
 IF c.mode='ACTIVE' THEN
  SELECT document INTO STRICT d FROM payments.route_policies WHERE version=c.version;
  was_active:=c.version=1 OR EXISTS(SELECT 1 FROM payments.route_policy_changes x JOIN payments.route_policy_decisions y ON y.change_id=x.id
   WHERE x.version=c.version AND x.mode='ACTIVE' AND y.outcome='APPLIED');
  shadow:=payments.route_shadow_report(c.version);
  IF NOT was_active AND (shadow->>'decisions')::integer<(d->>'promotionMinDecisions')::integer THEN
   RAISE EXCEPTION 'Shadow evaluation required: % of % decisions',shadow->>'decisions',d->>'promotionMinDecisions';
  END IF;
  UPDATE payments.route_policy_head SET active_version=c.version,shadow_version=CASE WHEN shadow_version=c.version THEN NULL ELSE shadow_version END,revision=revision+1,updated_at=clock_timestamp();
 ELSIF c.mode='SHADOW' THEN
  IF c.version=h.active_version THEN RAISE EXCEPTION 'The active version cannot also be the shadow'; END IF;
  UPDATE payments.route_policy_head SET shadow_version=c.version,revision=revision+1,updated_at=clock_timestamp();
 ELSE
  UPDATE payments.route_policy_head SET shadow_version=NULL,revision=revision+1,updated_at=clock_timestamp();
 END IF;
 INSERT INTO payments.route_policy_decisions VALUES(p_change,'APPLIED',(SELECT revision FROM payments.route_policy_head),p_actor,p_person);
 PERFORM payments.route_audit(p_actor,p_person,'routing.change-decided',p_change::text,'APPLIED',c.mode);
 RETURN jsonb_build_object('changeId',p_change,'outcome','APPLIED','head',(SELECT to_jsonb(x) FROM payments.route_policy_head x));
END $$;
CREATE FUNCTION payments.routing_view(p_actor jsonb,p_person uuid) RETURNS jsonb LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE h payments.route_policy_head;BEGIN
 PERFORM controls.require_staff(p_actor,p_person,ARRAY['payment_operator','payment_approver','payment_inquiry']);
 SELECT * INTO STRICT h FROM payments.route_policy_head;
 RETURN jsonb_build_object('head',jsonb_build_object('activeVersion',h.active_version,'shadowVersion',h.shadow_version,'revision',h.revision),
  'policies',(SELECT jsonb_agg(jsonb_build_object('version',version,'document',document,'digest',digest,'proposedBy',proposed_by->>'subject') ORDER BY version DESC) FROM payments.route_policies),
  'changes',(SELECT coalesce(jsonb_agg(jsonb_build_object('id',c.id,'version',c.version,'mode',c.mode,'proposedBy',c.proposed_by->>'subject','proposedPerson',c.proposed_person,
    'outcome',d.outcome,'decidedBy',d.decided_by->>'subject','evidence',c.evidence,'createdAt',c.created_at) ORDER BY c.created_at DESC),'[]'::jsonb)
    FROM payments.route_policy_changes c LEFT JOIN payments.route_policy_decisions d ON d.change_id=c.id),
  'health',payments.provider_health(interval '60 minutes'),
  'shadow',CASE WHEN h.shadow_version IS NULL THEN NULL ELSE payments.route_shadow_report(h.shadow_version) END,
  'decisions',(SELECT coalesce(jsonb_agg(x ORDER BY x->>'decidedAt' DESC),'[]'::jsonb) FROM (SELECT jsonb_build_object('paymentId',payment_id,'attempt',attempt,'provider',provider,
    'policyVersion',policy_version,'strategy',strategy,'rule',reason->>'rule','shadowVersion',shadow_version,'shadowProvider',shadow_provider,'decidedAt',decided_at) x
    FROM payments.route_decisions ORDER BY decided_at DESC LIMIT 20) r));
END $$;
GRANT EXECUTE ON FUNCTION payments.route_decide(uuid,integer,text[]),payments.route_fallback_allowed(uuid,text,integer),payments.propose_route_change(jsonb,uuid,jsonb,integer,text),
 payments.decide_route_change(jsonb,uuid,uuid,boolean),payments.routing_view(jsonb,uuid),payments.provider_health(interval) TO bank_payments;
