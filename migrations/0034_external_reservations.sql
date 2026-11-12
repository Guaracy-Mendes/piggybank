-- One gross daily principal budget covers internal bookings and active external reservations.
CREATE TABLE core.payment_limit_usage(
 operation_id uuid PRIMARY KEY,payer_account uuid NOT NULL REFERENCES core.ledger_accounts(id),principal bigint NOT NULL CHECK(principal>0),
 product_version_id uuid NOT NULL REFERENCES core.product_versions(id),business_date date NOT NULL,request_digest text NOT NULL,
 state text NOT NULL CHECK(state IN ('ACTIVE','POSTED','RELEASED'))
);
INSERT INTO core.payment_limit_usage SELECT operation_id,payer_account,principal,product_version_id,business_date,request_digest,'POSTED' FROM core.payment_executions;
CREATE INDEX daily_principal_budget ON core.payment_limit_usage(payer_account,business_date) WHERE state IN ('ACTIVE','POSTED');
CREATE FUNCTION core.guard_payment_budget() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF TG_OP<>'UPDATE' OR NEW.operation_id<>OLD.operation_id OR NEW.payer_account<>OLD.payer_account OR NEW.principal<>OLD.principal
 OR NEW.product_version_id<>OLD.product_version_id OR NEW.business_date<>OLD.business_date OR NEW.request_digest<>OLD.request_digest
 OR OLD.state<>'ACTIVE' OR NEW.state NOT IN ('POSTED','RELEASED') THEN RAISE EXCEPTION 'Immutable payment budget identity or terminal state'; END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER immutable_payment_budget BEFORE UPDATE OR DELETE ON core.payment_limit_usage FOR EACH ROW EXECUTE FUNCTION core.guard_payment_budget();
CREATE TRIGGER immutable_payment_budget_set BEFORE TRUNCATE ON core.payment_limit_usage FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal();
CREATE TABLE core.external_reservations(operation_id uuid PRIMARY KEY REFERENCES core.payment_limit_usage(operation_id),instruction jsonb NOT NULL,request_digest text NOT NULL,authority_until timestamptz NOT NULL,created_at timestamptz NOT NULL DEFAULT now());
CREATE TRIGGER immutable_external_authorizations BEFORE UPDATE OR DELETE OR TRUNCATE ON core.external_reservations FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal();
REVOKE ALL ON core.payment_limit_usage,core.external_reservations FROM bank_core;
GRANT SELECT ON core.payment_limit_usage,core.external_reservations TO bank_core;
INSERT INTO core.ledger_accounts(id,kind,state,debit_blocked) VALUES('4f1a780b-3e31-48ef-a294-000000000003','SETTLEMENT','ACTIVE',false) ON CONFLICT(id) DO NOTHING;
CREATE FUNCTION core.reserve_external_payment(p jsonb,p_digest text,p_valid_until timestamptz) RETURNS void
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,core AS $$
DECLARE op uuid:=(p->>'operationId')::uuid; payer uuid:=(p->'details'->>'payerAccountId')::uuid; product_id uuid:=(p->>'productVersionId')::uuid;
DECLARE amount bigint:=(p->'details'->'amount'->>'minorUnits')::bigint; fee bigint:=(p->'details'->'fee'->>'minorUnits')::bigint;
DECLARE day date:=(p->'details'->>'requestedDate')::date; product jsonb; used numeric; calculated numeric; prior core.external_reservations;
BEGIN
 PERFORM pg_advisory_xact_lock(hashtextextended(op::text,0));
 SELECT * INTO prior FROM core.external_reservations WHERE operation_id=op;
 IF FOUND THEN
  IF prior.request_digest<>p_digest OR EXISTS(SELECT 1 FROM core.payment_limit_usage WHERE operation_id=op AND state='RELEASED') THEN RAISE EXCEPTION 'Reservation identity conflict or released authority'; END IF;
  RETURN;
 END IF;
 PERFORM 1 FROM core.ledger_accounts WHERE id=payer FOR UPDATE;
 IF amount IS NULL OR amount<=0 OR fee IS NULL OR fee<0 OR p_valid_until IS NULL OR p_valid_until<=clock_timestamp()
 OR day IS DISTINCT FROM (clock_timestamp() AT TIME ZONE 'UTC')::date OR (p->'details'->>'rail') IS DISTINCT FROM 'SIMULATED'
 OR (p->>'payeeAccountId') IS DISTINCT FROM '4f1a780b-3e31-48ef-a294-000000000003' OR p->>'railPolicyVersionId' IS NULL
 OR (p->'details'->'amount'->'currency'->>'code') IS DISTINCT FROM 'TST' OR (p->'details'->'fee'->'currency'->>'code') IS DISTINCT FROM 'TST'
 OR (p->'details'->'amount'->'currency'->>'scale') IS DISTINCT FROM '2' OR (p->'details'->'fee'->'currency'->>'scale') IS DISTINCT FROM '2'
 THEN RAISE EXCEPTION 'Invalid current simulated instruction'; END IF;
 IF NOT EXISTS(SELECT 1 FROM core.account_registry WHERE id=payer AND product_version_id=product_id) THEN RAISE EXCEPTION 'Current payer product required'; END IF;
 SELECT document INTO STRICT product FROM core.product_versions WHERE id=product_id AND effective_from<=day AND effective_until>day;
 calculated:=trunc((amount::numeric*(product->>'feeBasisPoints')::numeric+5000)/10000)+(product->'flatFee'->>'minorUnits')::numeric;
 IF fee::numeric<>calculated OR amount::numeric>(product->'paymentLimit'->>'minorUnits')::numeric THEN RAISE EXCEPTION 'Current price or payment limit changed'; END IF;
 SELECT coalesce(sum(principal::numeric),0) INTO used FROM core.payment_limit_usage WHERE payer_account=payer AND business_date=day AND state IN ('ACTIVE','POSTED');
 IF used+amount::numeric>(product->'dailyLimit'->>'minorUnits')::numeric THEN RAISE EXCEPTION 'Daily payment limit exceeded'; END IF;
 PERFORM core.reserve(op,payer,amount+fee);
 INSERT INTO core.payment_limit_usage VALUES(op,payer,amount,product_id,day,p_digest,'ACTIVE');
 INSERT INTO core.external_reservations VALUES(op,p,p_digest,p_valid_until,now());
 INSERT INTO core.obligations VALUES(op,payer,'SIMULATED_RAIL','OPEN');
END $$;
CREATE FUNCTION core.release_external_unsubmitted(p_operation uuid,p_digest text) RETURNS void
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,core AS $$
BEGIN
 PERFORM pg_advisory_xact_lock(hashtextextended(p_operation::text,0));
 IF NOT EXISTS(SELECT 1 FROM core.external_reservations WHERE operation_id=p_operation AND request_digest=p_digest) THEN RAISE EXCEPTION 'Bound reservation required'; END IF;
 PERFORM core.release_hold(p_operation,NULL);
 UPDATE core.payment_limit_usage SET state='RELEASED' WHERE operation_id=p_operation AND state='ACTIVE';
 UPDATE core.obligations SET state='RESOLVED' WHERE operation_id=p_operation AND state='OPEN';
END $$;
GRANT EXECUTE ON FUNCTION core.reserve_external_payment(jsonb,text,timestamptz),core.release_external_unsubmitted(uuid,text) TO bank_core;

CREATE OR REPLACE FUNCTION core.execute_internal_payment(p jsonb,p_digest text,p_valid_until timestamptz)
RETURNS TABLE(batch_id uuid,created boolean) LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,core AS $$
DECLARE op uuid:=(p->>'operationId')::uuid; payer uuid:=(p->'details'->>'payerAccountId')::uuid; payee uuid:=(p->>'payeeAccountId')::uuid;
DECLARE product_id uuid:=(p->>'productVersionId')::uuid; amount bigint:=(p->'details'->'amount'->>'minorUnits')::bigint; fee bigint:=(p->'details'->'fee'->>'minorUnits')::bigint;
DECLARE day date:=(p->'details'->>'requestedDate')::date; fees uuid:='4f1a780b-3e31-48ef-a294-000000000004'; product jsonb; existing core.payment_executions;
DECLARE legs jsonb; posted record; used numeric; calculated numeric; debit bigint;
BEGIN
 PERFORM pg_advisory_xact_lock(hashtextextended(op::text,0));
 SELECT * INTO existing FROM core.payment_executions WHERE operation_id=op;
 IF FOUND THEN
  IF existing.request_digest<>p_digest THEN RAISE EXCEPTION 'Payment identity conflict'; END IF;
  RETURN QUERY SELECT existing.batch_id,false; RETURN;
 END IF;
 IF amount IS NULL OR amount<=0 OR fee IS NULL OR fee<0 OR payer=payee OR p_valid_until IS NULL OR p_valid_until<=clock_timestamp()
 OR (p->'details'->>'rail') IS DISTINCT FROM 'INTERNAL' OR day IS DISTINCT FROM (now() AT TIME ZONE 'UTC')::date
 OR (p->'details'->'amount'->'currency'->>'code') IS DISTINCT FROM 'TST' OR (p->'details'->'fee'->'currency'->>'code') IS DISTINCT FROM 'TST'
 OR (p->'details'->'amount'->'currency'->>'scale') IS DISTINCT FROM '2' OR (p->'details'->'fee'->'currency'->>'scale') IS DISTINCT FROM '2'
 THEN RAISE EXCEPTION 'Invalid current internal instruction'; END IF;
 -- Shared account lock order also serializes the payer's daily budget.
 PERFORM 1 FROM core.ledger_accounts WHERE id IN(payer,payee) OR (id=fees AND fee>0) ORDER BY id FOR UPDATE;
 IF p_valid_until<=clock_timestamp() OR day IS DISTINCT FROM (clock_timestamp() AT TIME ZONE 'UTC')::date THEN RAISE EXCEPTION 'Execution authority expired while waiting'; END IF;
 IF NOT EXISTS(SELECT 1 FROM core.account_registry r JOIN core.ledger_accounts a ON a.id=r.id WHERE r.id=payer AND r.product_version_id=product_id AND a.kind='CUSTOMER')
 OR NOT EXISTS(SELECT 1 FROM core.account_registry r JOIN core.ledger_accounts a ON a.id=r.id WHERE r.id=payee AND a.kind='CUSTOMER') THEN RAISE EXCEPTION 'Registered internal customer accounts required'; END IF;
 SELECT document INTO STRICT product FROM core.product_versions WHERE id=product_id AND effective_from<=day AND effective_until>day;
 calculated:=((amount::numeric*(product->>'feeBasisPoints')::numeric+5000)/10000);
 calculated:=trunc(calculated)+(product->'flatFee'->>'minorUnits')::numeric;
 IF fee::numeric<>calculated OR amount::numeric>(product->'paymentLimit'->>'minorUnits')::numeric THEN RAISE EXCEPTION 'Current fee or payment limit changed'; END IF;
 SELECT coalesce(sum(principal::numeric),0) INTO used FROM core.payment_limit_usage WHERE payer_account=payer AND business_date=day AND state IN ('ACTIVE','POSTED');
 IF used+amount::numeric>(product->'dailyLimit'->>'minorUnits')::numeric THEN RAISE EXCEPTION 'Daily payment limit exceeded'; END IF;
 debit:=amount+fee;
 legs:=jsonb_build_array(jsonb_build_object('accountId',payer,'delta',-debit),jsonb_build_object('accountId',payee,'delta',amount));
 IF fee>0 THEN legs:=legs||jsonb_build_array(jsonb_build_object('accountId',fees,'delta',fee)); END IF;
 SELECT * INTO STRICT posted FROM core.post(op,'INTERNAL_PAYMENT',legs);
 INSERT INTO core.payment_executions VALUES(op,payer,payee,amount,fee,product_id,day,p_digest,posted.batch_id,now());
 INSERT INTO core.payment_limit_usage VALUES(op,payer,amount,product_id,day,p_digest,'POSTED');
 RETURN QUERY SELECT posted.batch_id,posted.created;
END $$;
CREATE FUNCTION core.mark_external_submitted(p_operation uuid,p_digest text,p_valid_until timestamptz) RETURNS void
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,core AS $$
BEGIN
 PERFORM pg_advisory_xact_lock(hashtextextended(p_operation::text,0));
 IF NOT EXISTS(SELECT 1 FROM core.external_reservations WHERE operation_id=p_operation AND request_digest=p_digest) THEN RAISE EXCEPTION 'Bound reservation required'; END IF;
 IF p_valid_until IS NULL OR p_valid_until<=clock_timestamp() OR p_valid_until>(SELECT authority_until FROM core.external_reservations WHERE operation_id=p_operation) THEN RAISE EXCEPTION 'Submission authority expired'; END IF;
 PERFORM core.mark_submitted(p_operation);
END $$;
GRANT EXECUTE ON FUNCTION core.mark_external_submitted(uuid,text,timestamptz) TO bank_core;
