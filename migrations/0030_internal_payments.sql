-- Core owns receiving references, execution limits, fee booking and immutable financial receipts.
CREATE TABLE core.receiving_references(account_id uuid PRIMARY KEY REFERENCES core.account_registry(id),reference text NOT NULL UNIQUE,created_at timestamptz NOT NULL DEFAULT now());
CREATE TABLE core.payment_executions(
 operation_id uuid PRIMARY KEY,payer_account uuid NOT NULL REFERENCES core.ledger_accounts(id),payee_account uuid NOT NULL REFERENCES core.ledger_accounts(id),
 principal bigint NOT NULL CHECK(principal>0),fee bigint NOT NULL CHECK(fee>=0),product_version_id uuid NOT NULL REFERENCES core.product_versions(id),
 business_date date NOT NULL,request_digest text NOT NULL,batch_id uuid NOT NULL UNIQUE REFERENCES core.journal_batches(id),created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX payment_daily_limit ON core.payment_executions(payer_account,business_date);
CREATE TRIGGER immutable_payment_executions BEFORE UPDATE OR DELETE OR TRUNCATE ON core.payment_executions FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal();
REVOKE ALL ON core.receiving_references,core.payment_executions FROM bank_core;
GRANT SELECT ON core.receiving_references,core.payment_executions TO bank_core;
INSERT INTO core.ledger_accounts(id,kind,state,debit_blocked) VALUES('4f1a780b-3e31-48ef-a294-000000000004','CONTROL','ACTIVE',false);
CREATE FUNCTION core.receiving_reference(p_account uuid) RETURNS text LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,core AS $$
DECLARE result text;
BEGIN
 PERFORM 1 FROM core.account_registry r JOIN core.ledger_accounts a ON a.id=r.id WHERE r.id=p_account AND a.state='ACTIVE' AND NOT a.credit_blocked;
 IF NOT FOUND THEN RAISE EXCEPTION 'Receiving account unavailable'; END IF;
 INSERT INTO core.receiving_references VALUES(p_account,'PB'||upper(replace(gen_random_uuid()::text,'-','')),now()) ON CONFLICT(account_id) DO NOTHING;
 SELECT reference INTO STRICT result FROM core.receiving_references WHERE account_id=p_account;
 RETURN result;
END $$;
CREATE FUNCTION core.execute_internal_payment(p jsonb,p_digest text,p_valid_until timestamptz)
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
 PERFORM 1 FROM core.ledger_accounts WHERE id IN(payer,payee,fees) ORDER BY id FOR UPDATE;
 IF p_valid_until<=clock_timestamp() OR day IS DISTINCT FROM (clock_timestamp() AT TIME ZONE 'UTC')::date THEN RAISE EXCEPTION 'Execution authority expired while waiting'; END IF;
 IF NOT EXISTS(SELECT 1 FROM core.account_registry r JOIN core.ledger_accounts a ON a.id=r.id WHERE r.id=payer AND r.product_version_id=product_id AND a.kind='CUSTOMER')
 OR NOT EXISTS(SELECT 1 FROM core.account_registry r JOIN core.ledger_accounts a ON a.id=r.id WHERE r.id=payee AND a.kind='CUSTOMER') THEN RAISE EXCEPTION 'Registered internal customer accounts required'; END IF;
 SELECT document INTO STRICT product FROM core.product_versions WHERE id=product_id AND effective_from<=day AND effective_until>day;
 calculated:=((amount::numeric*(product->>'feeBasisPoints')::numeric+5000)/10000);
 calculated:=trunc(calculated)+(product->'flatFee'->>'minorUnits')::numeric;
 IF fee::numeric<>calculated OR amount::numeric>(product->'paymentLimit'->>'minorUnits')::numeric THEN RAISE EXCEPTION 'Current fee or payment limit changed'; END IF;
 SELECT coalesce(sum(principal::numeric),0) INTO used FROM core.payment_executions WHERE payer_account=payer AND business_date=day;
 IF used+amount::numeric>(product->'dailyLimit'->>'minorUnits')::numeric THEN RAISE EXCEPTION 'Daily payment limit exceeded'; END IF;
 debit:=amount+fee;
 legs:=jsonb_build_array(jsonb_build_object('accountId',payer,'delta',-debit),jsonb_build_object('accountId',payee,'delta',amount));
 IF fee>0 THEN legs:=legs||jsonb_build_array(jsonb_build_object('accountId',fees,'delta',fee)); END IF;
 SELECT * INTO STRICT posted FROM core.post(op,'INTERNAL_PAYMENT',legs);
 INSERT INTO core.payment_executions VALUES(op,payer,payee,amount,fee,product_id,day,p_digest,posted.batch_id,now());
 RETURN QUERY SELECT posted.batch_id,posted.created;
END $$;
GRANT EXECUTE ON FUNCTION core.receiving_reference(uuid),core.execute_internal_payment(jsonb,text,timestamptz) TO bank_core;

CREATE TABLE payments.beneficiaries(
 id uuid PRIMARY KEY,issuer text NOT NULL,subject text NOT NULL,request_key uuid NOT NULL,label text NOT NULL,
 receiving_reference text NOT NULL,payee_account_id uuid NOT NULL,state text NOT NULL CHECK(state IN ('VERIFIED','REVOKED')),version bigint NOT NULL DEFAULT 1,
 fingerprint text NOT NULL,verified_at timestamptz NOT NULL DEFAULT now(),UNIQUE(issuer,subject,request_key)
);
CREATE TABLE payments.quotes(
 id uuid PRIMARY KEY,issuer text NOT NULL,subject text NOT NULL,beneficiary_id uuid NOT NULL REFERENCES payments.beneficiaries(id),beneficiary_version bigint NOT NULL,
 instruction jsonb NOT NULL,details_digest text NOT NULL,total_minor bigint NOT NULL CHECK(total_minor>0),expires_at timestamptz NOT NULL,
 created_at timestamptz NOT NULL DEFAULT now()
);
CREATE TRIGGER immutable_quotes BEFORE UPDATE OR DELETE OR TRUNCATE ON payments.quotes FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal();
CREATE TABLE payments.intents(
 id uuid PRIMARY KEY,issuer text NOT NULL,subject text NOT NULL,client_id text NOT NULL,request_key uuid NOT NULL,quote_id uuid NOT NULL UNIQUE REFERENCES payments.quotes(id),
 actor_context jsonb NOT NULL,challenge_id uuid,state text NOT NULL CHECK(state IN ('DRAFT','ACCEPTED','CONTROLS_PENDING','CONTROL_REVIEW','EXPIRED','DECLINED','BLOCKED','SETTLED')),
 operation_id uuid,batch_id uuid,reason_code text NOT NULL DEFAULT 'ACTION_APPROVAL_REQUIRED',version bigint NOT NULL DEFAULT 1,
 confirmed_at timestamptz,created_at timestamptz NOT NULL DEFAULT now(),updated_at timestamptz NOT NULL DEFAULT now(),UNIQUE(issuer,subject,client_id,request_key),
 CHECK((state='SETTLED')=(batch_id IS NOT NULL))
);
