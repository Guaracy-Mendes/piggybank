-- owners: payments (ATM channel supplement, cards, messages, authorizations, settlement reconciliation), core (ATM
-- settlement account and withdrawal posting). WP-35 local ATM channel against a deterministic network simulator.
-- Synthetic only: no real network, cash logistics or card issuing; actual network certification is required (DEC-04/09).

-- ===== Channel supplement (immutable fixture) =====
CREATE TABLE payments.atm_operations(
 operation text PRIMARY KEY CHECK(operation IN ('BALANCE_INQUIRY','CASH_WITHDRAWAL','DEPOSIT','TRANSFER','PIN_CHANGE','MINI_STATEMENT')),
 enabled boolean NOT NULL,decline_reason text,label text NOT NULL CHECK(label='SYNTHETIC_FIXTURE'),
 CHECK(enabled=(decline_reason IS NULL))
);
INSERT INTO payments.atm_operations VALUES
 ('BALANCE_INQUIRY',true,NULL,'SYNTHETIC_FIXTURE'),('CASH_WITHDRAWAL',true,NULL,'SYNTHETIC_FIXTURE'),
 ('DEPOSIT',false,'OPERATION_NOT_SUPPORTED','SYNTHETIC_FIXTURE'),('TRANSFER',false,'OPERATION_NOT_SUPPORTED','SYNTHETIC_FIXTURE'),
 ('PIN_CHANGE',false,'OPERATION_NOT_SUPPORTED','SYNTHETIC_FIXTURE'),('MINI_STATEMENT',false,'OPERATION_NOT_SUPPORTED','SYNTHETIC_FIXTURE');
CREATE TABLE payments.atm_limits(id integer PRIMARY KEY CHECK(id=1),per_withdrawal_minor bigint NOT NULL,daily_minor bigint NOT NULL,authorization_seconds integer NOT NULL,
 label text NOT NULL CHECK(label='SYNTHETIC_FIXTURE'));
INSERT INTO payments.atm_limits VALUES(1,50000,100000,120,'SYNTHETIC_FIXTURE');
CREATE TRIGGER immutable_atm_operations BEFORE UPDATE OR DELETE OR TRUNCATE ON payments.atm_operations FOR EACH STATEMENT EXECUTE FUNCTION controls.immutable_check_result();
CREATE TRIGGER immutable_atm_limits BEFORE UPDATE OR DELETE OR TRUNCATE ON payments.atm_limits FOR EACH STATEMENT EXECUTE FUNCTION controls.immutable_check_result();

-- ===== Synthetic ATM cards (tokens linked to one owned account; card issuing is out of scope) =====
CREATE TABLE payments.atm_cards(
 id uuid PRIMARY KEY,token text NOT NULL UNIQUE CHECK(token ~ '^ATM-[0-9a-f]{16}$'),account_id uuid NOT NULL,person_id uuid NOT NULL,issuer text NOT NULL,subject text NOT NULL,
 state text NOT NULL CHECK(state IN ('ACTIVE','BLOCKED')),request_key uuid NOT NULL,created_at timestamptz NOT NULL DEFAULT clock_timestamp(),blocked_at timestamptz,
 UNIQUE(person_id,request_key),CHECK((state='BLOCKED')=(blocked_at IS NOT NULL))
);
CREATE FUNCTION payments.guard_atm_card() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN
 IF TG_OP<>'UPDATE' OR OLD.state='BLOCKED' OR NEW.state<>'BLOCKED' OR (to_jsonb(NEW)-ARRAY['state','blocked_at']) IS DISTINCT FROM (to_jsonb(OLD)-ARRAY['state','blocked_at'])
 THEN RAISE EXCEPTION 'ATM card identity is immutable; a card can only be blocked'; END IF;
 RETURN NEW;END $$;
CREATE TRIGGER atm_card_guard BEFORE UPDATE OR DELETE ON payments.atm_cards FOR EACH ROW EXECUTE FUNCTION payments.guard_atm_card();

-- ===== Network messages: one stored response per message id (duplicates replay it; reuse with other content conflicts) =====
CREATE TABLE payments.atm_messages(
 message_id text PRIMARY KEY CHECK(message_id ~ '^[A-Za-z0-9-]{8,64}$'),kind text NOT NULL,fingerprint text NOT NULL CHECK(length(fingerprint)=64),
 document jsonb NOT NULL,response jsonb,received_at timestamptz NOT NULL DEFAULT clock_timestamp(),responded_at timestamptz
);
CREATE FUNCTION payments.guard_atm_message() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN
 IF TG_OP<>'UPDATE' OR OLD.response IS NOT NULL OR (to_jsonb(NEW)-ARRAY['response','responded_at']) IS DISTINCT FROM (to_jsonb(OLD)-ARRAY['response','responded_at'])
 THEN RAISE EXCEPTION 'ATM messages and responses are immutable'; END IF;
 RETURN NEW;END $$;
CREATE TRIGGER atm_message_guard BEFORE UPDATE OR DELETE ON payments.atm_messages FOR EACH ROW EXECUTE FUNCTION payments.guard_atm_message();

CREATE TABLE payments.atm_authorizations(
 id uuid PRIMARY KEY,message_id text NOT NULL UNIQUE REFERENCES payments.atm_messages(message_id),card_id uuid NOT NULL REFERENCES payments.atm_cards(id),
 account_id uuid NOT NULL,terminal_id text NOT NULL CHECK(terminal_id ~ '^SIM-ATM-[0-9]{3}$'),amount_minor bigint NOT NULL CHECK(amount_minor>0),
 state text NOT NULL CHECK(state IN ('PENDING','APPROVED','DECLINED','COMPLETED','REVERSED','EXPIRED')),decline_reason text,
 expires_at timestamptz NOT NULL,completion_batch uuid,late_reversal_at timestamptz,business_date date NOT NULL DEFAULT (clock_timestamp() AT TIME ZONE 'UTC')::date,
 created_at timestamptz NOT NULL DEFAULT clock_timestamp(),updated_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 CHECK((state='DECLINED')=(decline_reason IS NOT NULL)),CHECK((state='COMPLETED')=(completion_batch IS NOT NULL))
);
CREATE INDEX atm_auth_card_day ON payments.atm_authorizations(card_id,business_date);
CREATE INDEX atm_auth_due ON payments.atm_authorizations(expires_at) WHERE state='APPROVED';
CREATE FUNCTION payments.guard_atm_authorization() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE mutable text[]:=ARRAY['state','decline_reason','completion_batch','late_reversal_at','updated_at'];BEGIN
 IF TG_OP<>'UPDATE' OR (to_jsonb(NEW)-mutable) IS DISTINCT FROM (to_jsonb(OLD)-mutable)
  OR (OLD.decline_reason IS NOT NULL AND NEW.decline_reason IS DISTINCT FROM OLD.decline_reason)
  OR NOT ((OLD.state='PENDING' AND NEW.state IN ('APPROVED','DECLINED')) OR (OLD.state='APPROVED' AND NEW.state IN ('COMPLETED','REVERSED','EXPIRED')) OR (OLD.state='COMPLETED' AND NEW.state='COMPLETED' AND OLD.late_reversal_at IS NULL AND NEW.late_reversal_at IS NOT NULL))
 THEN RAISE EXCEPTION 'ATM authorization identity and lifecycle are immutable'; END IF;
 RETURN NEW;END $$;
CREATE TRIGGER atm_authorization_guard BEFORE UPDATE OR DELETE ON payments.atm_authorizations FOR EACH ROW EXECUTE FUNCTION payments.guard_atm_authorization();

-- ===== Settlement files and reconciliation =====
CREATE TABLE payments.atm_settlement_files(
 file_id text PRIMARY KEY CHECK(file_id ~ '^[A-Za-z0-9-]{8,64}$'),business_date date NOT NULL,fingerprint text NOT NULL,document jsonb NOT NULL,
 result jsonb NOT NULL,received_at timestamptz NOT NULL DEFAULT clock_timestamp()
);
CREATE TRIGGER immutable_atm_settlement_files BEFORE UPDATE OR DELETE OR TRUNCATE ON payments.atm_settlement_files FOR EACH STATEMENT EXECUTE FUNCTION controls.immutable_check_result();
-- Compares the network's completed withdrawals with the bank's for the day. Every difference is listed, none is netted.
CREATE FUNCTION payments.atm_reconcile(p_file text,p_date date,p_fingerprint text,p_document jsonb) RETURNS jsonb
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,payments AS $$
DECLARE f payments.atm_settlement_files;items jsonb;result jsonb;BEGIN
 SELECT * INTO f FROM payments.atm_settlement_files WHERE file_id=p_file;
 IF FOUND THEN
  IF f.fingerprint<>p_fingerprint THEN RAISE EXCEPTION 'Settlement file identity conflict'; END IF;
  RETURN f.result;
 END IF;
 IF jsonb_typeof(p_document->'completions')<>'array' THEN RAISE EXCEPTION 'Invalid settlement file' USING ERRCODE='22023'; END IF;
 WITH network AS (SELECT (c->>'authorizationId')::uuid AS id,(c->>'amountMinor')::bigint AS amount FROM jsonb_array_elements(p_document->'completions') c),
  bank AS (SELECT id,amount_minor AS amount,late_reversal_at FROM payments.atm_authorizations WHERE state='COMPLETED' AND business_date=p_date),
  joined AS (SELECT coalesce(n.id,b.id) AS id,n.amount AS network_amount,b.amount AS bank_amount,b.late_reversal_at,
   CASE WHEN b.id IS NULL THEN 'MISSING_AT_BANK' WHEN n.id IS NULL THEN 'MISSING_AT_NETWORK' WHEN n.amount<>b.amount THEN 'AMOUNT_MISMATCH'
    WHEN b.late_reversal_at IS NOT NULL THEN 'LATE_REVERSAL_REVIEW' ELSE 'MATCHED' END AS outcome FROM network n FULL JOIN bank b ON b.id=n.id)
 SELECT coalesce(jsonb_agg(jsonb_build_object('authorizationId',id,'networkAmountMinor',network_amount,'bankAmountMinor',bank_amount,'outcome',outcome) ORDER BY outcome,id),'[]'::jsonb) INTO items FROM joined;
 result:=jsonb_build_object('fileId',p_file,'businessDate',p_date,'items',items,
  'matched',(SELECT count(*) FROM jsonb_array_elements(items) i WHERE i->>'outcome'='MATCHED'),
  'differences',(SELECT count(*) FROM jsonb_array_elements(items) i WHERE i->>'outcome'<>'MATCHED'));
 INSERT INTO payments.atm_settlement_files VALUES(p_file,p_date,p_fingerprint,p_document,result,clock_timestamp());
 RETURN result;
END $$;

-- ===== Core: ATM settlement account and the withdrawal posting =====
INSERT INTO core.ledger_accounts(id,kind,state,debit_blocked) VALUES('4f1a780b-3e31-48ef-a294-00000000a7e0','SETTLEMENT','ACTIVE',false) ON CONFLICT DO NOTHING;
-- Consumes the authorization's hold exactly once (UNIQUE operation/kind); the customer leg re-checks restrictions and funds.
CREATE FUNCTION core.atm_withdrawal(p_authorization uuid,p_account uuid,p_amount bigint) RETURNS TABLE(batch_id uuid,created boolean)
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,core AS $$
BEGIN
 IF p_amount IS NULL OR p_amount<=0 OR NOT EXISTS(SELECT 1 FROM core.ledger_accounts WHERE id=p_account AND kind='CUSTOMER') THEN RAISE EXCEPTION 'Invalid ATM withdrawal'; END IF;
 RETURN QUERY SELECT * FROM core.post(p_authorization,'ATM_WITHDRAWAL',jsonb_build_array(jsonb_build_object('accountId',p_account,'delta',-p_amount),
  jsonb_build_object('accountId','4f1a780b-3e31-48ef-a294-00000000a7e0'::uuid,'delta',p_amount)),p_authorization);
END $$;
GRANT EXECUTE ON FUNCTION core.atm_withdrawal(uuid,uuid,bigint) TO bank_core;

REVOKE UPDATE,DELETE,TRUNCATE ON payments.atm_operations,payments.atm_limits,payments.atm_settlement_files FROM bank_payments;
GRANT EXECUTE ON FUNCTION payments.atm_reconcile(text,date,text,jsonb) TO bank_payments;
