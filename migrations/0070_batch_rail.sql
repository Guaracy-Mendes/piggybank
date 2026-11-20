-- owners: payments (synthetic batch rail supplement, recipients, entries, files, network messages), core (batch clearing
-- account, credit and return postings). WP-36: one synthetic ACH-style batch credit rail (user decision 2026-11-18) as
-- the template for later rails. No real ACH network; certification and every other rail remain pending (DEC-04/09).

-- ===== Rail supplement (immutable fixture) =====
CREATE TABLE payments.batch_supplement(
 id integer PRIMARY KEY CHECK(id=1),rail text NOT NULL CHECK(rail='SIM_BATCH'),cutoff_utc time NOT NULL,settlement_lag_days integer NOT NULL CHECK(settlement_lag_days BETWEEN 0 AND 3),
 per_entry_max_minor bigint NOT NULL CHECK(per_entry_max_minor>0),return_window_days integer NOT NULL CHECK(return_window_days BETWEEN 1 AND 60),
 label text NOT NULL CHECK(label='SYNTHETIC_FIXTURE')
);
INSERT INTO payments.batch_supplement VALUES(1,'SIM_BATCH','16:00',1,2500000,2,'SYNTHETIC_FIXTURE');
CREATE TABLE payments.batch_return_reasons(code text PRIMARY KEY CHECK(code ~ '^R[0-9]{2}$'),description text NOT NULL,label text NOT NULL CHECK(label='SYNTHETIC_FIXTURE'));
INSERT INTO payments.batch_return_reasons VALUES
 ('R02','Account closed','SYNTHETIC_FIXTURE'),('R03','No account or unable to locate account','SYNTHETIC_FIXTURE'),('R04','Invalid account number','SYNTHETIC_FIXTURE');
CREATE TABLE payments.batch_holidays(day date PRIMARY KEY,label text NOT NULL CHECK(label='SYNTHETIC_FIXTURE'));
INSERT INTO payments.batch_holidays VALUES('2026-11-26','SYNTHETIC_FIXTURE'),('2026-12-25','SYNTHETIC_FIXTURE'),('2027-01-01','SYNTHETIC_FIXTURE');
CREATE TRIGGER immutable_batch_supplement BEFORE UPDATE OR DELETE OR TRUNCATE ON payments.batch_supplement FOR EACH STATEMENT EXECUTE FUNCTION controls.immutable_check_result();
CREATE TRIGGER immutable_batch_return_reasons BEFORE UPDATE OR DELETE OR TRUNCATE ON payments.batch_return_reasons FOR EACH STATEMENT EXECUTE FUNCTION controls.immutable_check_result();
CREATE TRIGGER immutable_batch_holidays BEFORE UPDATE OR DELETE OR TRUNCATE ON payments.batch_holidays FOR EACH STATEMENT EXECUTE FUNCTION controls.immutable_check_result();
-- Next business day strictly after p_day (weekends and fixture holidays skipped).
CREATE FUNCTION payments.batch_next_business_day(p_day date) RETURNS date LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,payments AS $$
DECLARE d date:=p_day+1;BEGIN
 WHILE extract(isodow FROM d)>5 OR EXISTS(SELECT 1 FROM payments.batch_holidays WHERE day=d) LOOP d:=d+1; END LOOP;
 RETURN d;
END $$;
-- Settlement date for an entry queued now: before the cutoff on a business day it settles on the next business day;
-- otherwise one business day later.
CREATE FUNCTION payments.batch_settlement_date(p_at timestamptz) RETURNS date LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,payments AS $$
DECLARE s payments.batch_supplement;today date:=(p_at AT TIME ZONE 'UTC')::date;effective date;BEGIN
 SELECT * INTO STRICT s FROM payments.batch_supplement WHERE id=1;
 effective:=today;
 IF extract(isodow FROM today)>5 OR EXISTS(SELECT 1 FROM payments.batch_holidays WHERE day=today) OR (p_at AT TIME ZONE 'UTC')::time>=s.cutoff_utc THEN
  effective:=payments.batch_next_business_day(today);
 END IF;
 FOR i IN 1..s.settlement_lag_days LOOP effective:=payments.batch_next_business_day(effective); END LOOP;
 RETURN effective;
END $$;

-- ===== Recipients (routing/account format validated; no account-validation network in the synthetic profile) =====
CREATE TABLE payments.batch_recipients(
 id uuid PRIMARY KEY,person_id uuid NOT NULL,issuer text NOT NULL,subject text NOT NULL,label text NOT NULL CHECK(length(trim(label)) BETWEEN 1 AND 80),
 routing_number text NOT NULL CHECK(routing_number ~ '^[0-9]{9}$'),account_number text NOT NULL CHECK(account_number ~ '^[0-9]{4,17}$'),
 request_key uuid NOT NULL,created_at timestamptz NOT NULL DEFAULT clock_timestamp(),UNIQUE(person_id,request_key)
);
CREATE TRIGGER immutable_batch_recipients BEFORE UPDATE OR DELETE OR TRUNCATE ON payments.batch_recipients FOR EACH STATEMENT EXECUTE FUNCTION controls.immutable_check_result();

-- ===== Entries, files and network messages =====
CREATE TABLE payments.batch_files(
 id uuid PRIMARY KEY,entry_count integer NOT NULL CHECK(entry_count>0),total_minor bigint NOT NULL CHECK(total_minor>0),digest text NOT NULL CHECK(length(digest)=64),
 document jsonb NOT NULL,state text NOT NULL CHECK(state IN ('SUBMITTED','ACKNOWLEDGED','SETTLED')),closed_by text NOT NULL,
 created_at timestamptz NOT NULL DEFAULT clock_timestamp(),acknowledged_at timestamptz,settled_at timestamptz
);
CREATE FUNCTION payments.guard_batch_file() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN
 IF TG_OP<>'UPDATE' OR (to_jsonb(NEW)-ARRAY['state','acknowledged_at','settled_at']) IS DISTINCT FROM (to_jsonb(OLD)-ARRAY['state','acknowledged_at','settled_at'])
  OR NOT ((OLD.state='SUBMITTED' AND NEW.state='ACKNOWLEDGED') OR (OLD.state='ACKNOWLEDGED' AND NEW.state='SETTLED'))
 THEN RAISE EXCEPTION 'Batch files are immutable'; END IF;
 RETURN NEW;END $$;
CREATE TRIGGER batch_file_guard BEFORE UPDATE OR DELETE ON payments.batch_files FOR EACH ROW EXECUTE FUNCTION payments.guard_batch_file();
CREATE TABLE payments.batch_entries(
 id uuid PRIMARY KEY,person_id uuid NOT NULL,issuer text NOT NULL,subject text NOT NULL,actor_context jsonb NOT NULL,account_id uuid NOT NULL,
 recipient_id uuid NOT NULL REFERENCES payments.batch_recipients(id),amount_minor bigint NOT NULL CHECK(amount_minor>0),request_key uuid NOT NULL,
 state text NOT NULL CHECK(state IN ('DRAFT','QUEUED','CANCELLED','SUBMITTED','REJECTED','SETTLED','RETURNED')),
 challenge_id uuid,file_id uuid REFERENCES payments.batch_files(id),settlement_date date,reason_code text,settle_batch uuid,return_batch uuid,late_return_at timestamptz,
 created_at timestamptz NOT NULL DEFAULT clock_timestamp(),updated_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 UNIQUE(person_id,request_key),
 CHECK(state NOT IN ('SETTLED','RETURNED') OR settle_batch IS NOT NULL),CHECK((state='RETURNED')=(return_batch IS NOT NULL)),
 CHECK(state NOT IN ('SUBMITTED','REJECTED','SETTLED','RETURNED') OR file_id IS NOT NULL)
);
CREATE INDEX batch_entries_queued ON payments.batch_entries(created_at) WHERE state='QUEUED';
CREATE INDEX batch_entries_person ON payments.batch_entries(person_id,created_at DESC);
CREATE FUNCTION payments.guard_batch_entry() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE mutable text[]:=ARRAY['state','challenge_id','file_id','settlement_date','reason_code','settle_batch','return_batch','late_return_at','updated_at'];BEGIN
 IF TG_OP<>'UPDATE' OR (to_jsonb(NEW)-mutable) IS DISTINCT FROM (to_jsonb(OLD)-mutable)
  OR (OLD.challenge_id IS NOT NULL AND NEW.challenge_id IS DISTINCT FROM OLD.challenge_id)
  OR (OLD.file_id IS NOT NULL AND NEW.file_id IS DISTINCT FROM OLD.file_id)
  OR (OLD.settle_batch IS NOT NULL AND NEW.settle_batch IS DISTINCT FROM OLD.settle_batch)
  OR NOT ((OLD.state=NEW.state AND OLD.state IN ('DRAFT','SETTLED')) OR (OLD.state='DRAFT' AND NEW.state IN ('QUEUED','CANCELLED')) OR (OLD.state='QUEUED' AND NEW.state IN ('SUBMITTED','CANCELLED'))
   OR (OLD.state='SUBMITTED' AND NEW.state IN ('REJECTED','SETTLED')) OR (OLD.state='SETTLED' AND NEW.state='RETURNED'))
 THEN RAISE EXCEPTION 'Batch entry identity and lifecycle are immutable'; END IF;
 RETURN NEW;END $$;
CREATE TRIGGER batch_entry_guard BEFORE UPDATE OR DELETE ON payments.batch_entries FOR EACH ROW EXECUTE FUNCTION payments.guard_batch_entry();
CREATE TABLE payments.batch_messages(
 message_id text PRIMARY KEY CHECK(message_id ~ '^[A-Za-z0-9-]{8,64}$'),kind text NOT NULL,fingerprint text NOT NULL CHECK(length(fingerprint)=64),
 document jsonb NOT NULL,response jsonb,received_at timestamptz NOT NULL DEFAULT clock_timestamp(),responded_at timestamptz
);
CREATE TRIGGER batch_message_guard BEFORE UPDATE OR DELETE ON payments.batch_messages FOR EACH ROW EXECUTE FUNCTION payments.guard_atm_message();

-- Seals every queued entry into one outbound file (idempotent per call: an empty queue produces no file).
CREATE FUNCTION payments.batch_close_window(p_closed_by text) RETURNS jsonb LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,payments AS $$
DECLARE f uuid:=gen_random_uuid();ids uuid[];doc jsonb;total bigint;BEGIN
 PERFORM pg_advisory_xact_lock(hashtextextended('payments.batch_close_window',0));
 SELECT array_agg(id ORDER BY created_at,id),sum(amount_minor) INTO ids,total FROM payments.batch_entries WHERE state='QUEUED';
 IF ids IS NULL THEN RETURN NULL; END IF;
 SELECT jsonb_build_object('fileId',f,'rail','SIM_BATCH','entries',jsonb_agg(jsonb_build_object('entryId',e.id,'routingNumber',r.routing_number,'accountNumber',r.account_number,
   'amountMinor',e.amount_minor::text,'settlementDate',e.settlement_date) ORDER BY e.created_at,e.id))
  INTO doc FROM payments.batch_entries e JOIN payments.batch_recipients r ON r.id=e.recipient_id WHERE e.id=ANY(ids);
 INSERT INTO payments.batch_files(id,entry_count,total_minor,digest,document,state,closed_by) VALUES(f,cardinality(ids),total,encode(sha256(convert_to(doc::text,'UTF8')),'hex'),doc,'SUBMITTED',p_closed_by);
 UPDATE payments.batch_entries SET state='SUBMITTED',file_id=f,updated_at=clock_timestamp() WHERE id=ANY(ids) AND state='QUEUED';
 RETURN doc||jsonb_build_object('entryCount',cardinality(ids),'totalMinor',total::text);
END $$;

-- ===== Core: batch clearing account, credit and return postings =====
INSERT INTO core.ledger_accounts(id,kind,state,debit_blocked) VALUES('4f1a780b-3e31-48ef-a294-00000000ba70','SETTLEMENT','ACTIVE',false) ON CONFLICT DO NOTHING;
-- Settles an accepted entry once: consumes the entry's hold (UNIQUE operation/kind).
CREATE FUNCTION core.batch_credit(p_entry uuid,p_account uuid,p_amount bigint) RETURNS TABLE(batch_id uuid,created boolean)
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,core AS $$
BEGIN
 IF p_amount IS NULL OR p_amount<=0 OR NOT EXISTS(SELECT 1 FROM core.ledger_accounts WHERE id=p_account AND kind='CUSTOMER') THEN RAISE EXCEPTION 'Invalid batch credit'; END IF;
 RETURN QUERY SELECT * FROM core.post(p_entry,'BATCH_CREDIT',jsonb_build_array(jsonb_build_object('accountId',p_account,'delta',-p_amount),
  jsonb_build_object('accountId','4f1a780b-3e31-48ef-a294-00000000ba70'::uuid,'delta',p_amount)),p_entry);
END $$;
-- Credits a returned entry back once, linked to the original settlement batch.
CREATE FUNCTION core.batch_return(p_entry uuid,p_account uuid,p_amount bigint,p_original uuid) RETURNS TABLE(batch_id uuid,created boolean)
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,core AS $$
BEGIN
 IF p_amount IS NULL OR p_amount<=0 OR NOT EXISTS(SELECT 1 FROM core.journal_batches WHERE id=p_original AND operation_id=p_entry AND posting_kind='BATCH_CREDIT') THEN RAISE EXCEPTION 'Invalid batch return'; END IF;
 RETURN QUERY SELECT * FROM core.post(p_entry,'BATCH_RETURN',jsonb_build_array(jsonb_build_object('accountId','4f1a780b-3e31-48ef-a294-00000000ba70'::uuid,'delta',-p_amount),
  jsonb_build_object('accountId',p_account,'delta',p_amount)),NULL,p_original);
END $$;
GRANT EXECUTE ON FUNCTION core.batch_credit(uuid,uuid,bigint),core.batch_return(uuid,uuid,bigint,uuid) TO bank_core;
REVOKE UPDATE,DELETE,TRUNCATE ON payments.batch_supplement,payments.batch_return_reasons,payments.batch_holidays,payments.batch_recipients FROM bank_payments;
GRANT EXECUTE ON FUNCTION payments.batch_next_business_day(date),payments.batch_settlement_date(timestamptz),payments.batch_close_window(text) TO bank_payments;
