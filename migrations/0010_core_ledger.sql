-- Core owns all money. Runtime callers can execute guarded routines, never edit balances/journals.
CREATE TABLE core.ledger_accounts(
 id uuid PRIMARY KEY,kind text NOT NULL CHECK(kind IN ('CUSTOMER','SETTLEMENT','CONTROL')),
 currency text NOT NULL DEFAULT 'TST' CHECK(currency='TST'),scale smallint NOT NULL DEFAULT 2 CHECK(scale=2),
 state text NOT NULL DEFAULT 'PROVISIONING' CHECK(state IN ('PROVISIONING','ACTIVE','CLOSING','CLOSED')),
 debit_blocked boolean NOT NULL DEFAULT true,credit_blocked boolean NOT NULL DEFAULT false,
 booked bigint NOT NULL DEFAULT 0,held bigint NOT NULL DEFAULT 0 CHECK(held>=0),version bigint NOT NULL DEFAULT 1,
 CHECK(kind<>'CUSTOMER' OR (booked>=0 AND held<=booked)),CHECK(state<>'CLOSED' OR (booked=0 AND held=0))
);
CREATE TABLE core.holds(
 operation_id uuid PRIMARY KEY,account_id uuid NOT NULL REFERENCES core.ledger_accounts(id),amount bigint NOT NULL CHECK(amount>0),
 state text NOT NULL CHECK(state IN ('ACTIVE','CONSUMED','RELEASED')),submitted boolean NOT NULL DEFAULT false,
 release_evidence uuid,version bigint NOT NULL DEFAULT 1,created_at timestamptz NOT NULL DEFAULT now(),updated_at timestamptz NOT NULL DEFAULT now()
);
CREATE TABLE core.journal_batches(
 id uuid PRIMARY KEY DEFAULT gen_random_uuid(),operation_id uuid NOT NULL,posting_kind text NOT NULL,
 fingerprint text NOT NULL,original_batch uuid REFERENCES core.journal_batches(id),
 business_date date NOT NULL DEFAULT (now() AT TIME ZONE 'UTC')::date,created_at timestamptz NOT NULL DEFAULT now(),
 UNIQUE(operation_id,posting_kind)
);
CREATE TABLE core.journal_entries(
 batch_id uuid NOT NULL REFERENCES core.journal_batches(id),account_id uuid NOT NULL REFERENCES core.ledger_accounts(id),
 delta bigint NOT NULL CHECK(delta<>0),PRIMARY KEY(batch_id,account_id)
);
CREATE FUNCTION core.immutable_journal() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'Immutable journal'; END $$;
CREATE TRIGGER immutable_batches BEFORE UPDATE OR DELETE OR TRUNCATE ON core.journal_batches FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal();
CREATE TRIGGER immutable_entries BEFORE UPDATE OR DELETE OR TRUNCATE ON core.journal_entries FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal();
CREATE FUNCTION core.balanced_batch() RETURNS trigger LANGUAGE plpgsql SET search_path=pg_catalog,core AS $$
DECLARE target uuid; total numeric; count_entries integer;
BEGIN
 IF TG_TABLE_NAME='journal_batches' THEN target:=NEW.id; ELSE target:=NEW.batch_id; END IF;
 SELECT sum(delta::numeric),count(*) INTO total,count_entries FROM core.journal_entries WHERE batch_id=target;
 IF count_entries<2 OR total IS DISTINCT FROM 0::numeric THEN RAISE EXCEPTION 'Unbalanced journal batch'; END IF;
 RETURN NEW;
END $$;
CREATE CONSTRAINT TRIGGER balanced_batch AFTER INSERT ON core.journal_batches DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION core.balanced_batch();
CREATE CONSTRAINT TRIGGER balanced_entries AFTER INSERT ON core.journal_entries DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION core.balanced_batch();
REVOKE ALL ON core.ledger_accounts,core.holds,core.journal_batches,core.journal_entries FROM bank_core;
GRANT SELECT ON core.ledger_accounts,core.holds,core.journal_batches,core.journal_entries TO bank_core;

-- Owner-only primitive. Explicit public routines below define the permitted financial paths.
CREATE FUNCTION core.post(p_operation uuid,p_kind text,p_legs jsonb,p_hold uuid DEFAULT NULL,p_original uuid DEFAULT NULL)
RETURNS TABLE(batch_id uuid,created boolean) LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,core AS $$
DECLARE digest text; existing core.journal_batches; account core.ledger_accounts; item record; reserved core.holds;
DECLARE target uuid:=gen_random_uuid(); total numeric;
BEGIN
 PERFORM pg_advisory_xact_lock(hashtextextended(p_operation::text,0));
 digest:=encode(sha256(convert_to(jsonb_build_object('legs',p_legs,'hold',p_hold,'original',p_original)::text,'UTF8')),'hex');
 SELECT * INTO existing FROM core.journal_batches WHERE operation_id=p_operation AND posting_kind=p_kind;
 IF FOUND THEN
  IF existing.fingerprint<>digest THEN RAISE EXCEPTION 'Posting identity conflict'; END IF;
  RETURN QUERY SELECT existing.id,false; RETURN;
 END IF;
 IF jsonb_typeof(p_legs)<>'array' OR jsonb_array_length(p_legs)<2 OR jsonb_array_length(p_legs)>16 THEN RAISE EXCEPTION 'Invalid journal legs'; END IF;
 SELECT sum((value->>'delta')::numeric) INTO total FROM jsonb_array_elements(p_legs);
 IF total IS DISTINCT FROM 0::numeric OR EXISTS(SELECT 1 FROM jsonb_array_elements(p_legs) WHERE (value->>'delta')::bigint=0 OR (value->>'delta') IS NULL OR (value->>'accountId') IS NULL)
 OR (SELECT count(DISTINCT value->>'accountId') FROM jsonb_array_elements(p_legs))<>jsonb_array_length(p_legs) THEN RAISE EXCEPTION 'Invalid balanced journal'; END IF;
 -- All callers lock the same accounts in ascending UUID order.
 FOR item IN SELECT (value->>'accountId')::uuid AS id FROM jsonb_array_elements(p_legs) ORDER BY 1 LOOP
  SELECT * INTO STRICT account FROM core.ledger_accounts WHERE id=item.id FOR UPDATE;
 END LOOP;
 IF p_hold IS NOT NULL THEN
  SELECT * INTO STRICT reserved FROM core.holds WHERE operation_id=p_hold FOR UPDATE;
  IF reserved.state<>'ACTIVE' OR NOT EXISTS(SELECT 1 FROM jsonb_array_elements(p_legs) WHERE (value->>'accountId')::uuid=reserved.account_id AND (value->>'delta')::numeric=-reserved.amount::numeric) THEN RAISE EXCEPTION 'Hold cannot be consumed'; END IF;
  UPDATE core.ledger_accounts SET held=held-reserved.amount WHERE id=reserved.account_id;
  UPDATE core.holds SET state='CONSUMED',version=version+1,updated_at=now() WHERE operation_id=p_hold;
 END IF;
 INSERT INTO core.journal_batches(id,operation_id,posting_kind,fingerprint,original_batch) VALUES(target,p_operation,p_kind,digest,p_original);
 FOR item IN SELECT (value->>'accountId')::uuid AS id,(value->>'delta')::bigint AS delta FROM jsonb_array_elements(p_legs) ORDER BY 1 LOOP
  SELECT * INTO STRICT account FROM core.ledger_accounts WHERE id=item.id;
  IF account.state<>'ACTIVE' OR (item.delta<0 AND account.debit_blocked) OR (item.delta>0 AND account.credit_blocked) THEN RAISE EXCEPTION 'Account restricted'; END IF;
  IF account.kind='CUSTOMER' AND account.booked::numeric+item.delta::numeric-account.held::numeric<0 THEN RAISE EXCEPTION 'Insufficient available funds'; END IF;
  UPDATE core.ledger_accounts SET booked=booked+item.delta,version=version+1 WHERE id=item.id;
  INSERT INTO core.journal_entries VALUES(target,item.id,item.delta);
 END LOOP;
 RETURN QUERY SELECT target,true;
END $$;

CREATE FUNCTION core.reserve(p_operation uuid,p_account uuid,p_amount bigint) RETURNS void LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,core AS $$
DECLARE account core.ledger_accounts; existing core.holds;
BEGIN
 IF p_amount IS NULL OR p_amount<=0 THEN RAISE EXCEPTION 'Positive amount required'; END IF;
 PERFORM pg_advisory_xact_lock(hashtextextended(p_operation::text,0));
 SELECT * INTO STRICT account FROM core.ledger_accounts WHERE id=p_account FOR UPDATE;
 SELECT * INTO existing FROM core.holds WHERE operation_id=p_operation;
 IF FOUND THEN
  IF existing.account_id<>p_account OR existing.amount<>p_amount THEN RAISE EXCEPTION 'Hold identity conflict'; END IF;
  RETURN;
 END IF;
 IF account.kind<>'CUSTOMER' OR account.state<>'ACTIVE' OR account.debit_blocked OR account.booked::numeric-account.held::numeric<p_amount THEN RAISE EXCEPTION 'Cannot reserve funds'; END IF;
 INSERT INTO core.holds(operation_id,account_id,amount,state) VALUES(p_operation,p_account,p_amount,'ACTIVE');
 UPDATE core.ledger_accounts SET held=held+p_amount,version=version+1 WHERE id=p_account;
END $$;
CREATE FUNCTION core.mark_submitted(p_operation uuid) RETURNS void LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,core AS $$ BEGIN
 PERFORM pg_advisory_xact_lock(hashtextextended(p_operation::text,0));
 UPDATE core.holds SET submitted=true,version=version+1,updated_at=now() WHERE operation_id=p_operation AND state='ACTIVE' AND NOT submitted;
 IF NOT EXISTS(SELECT 1 FROM core.holds WHERE operation_id=p_operation AND state='ACTIVE') THEN RAISE EXCEPTION 'No active hold'; END IF;
END $$;
CREATE FUNCTION core.release_hold(p_operation uuid,p_evidence uuid DEFAULT NULL) RETURNS void LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,core AS $$
DECLARE reserved core.holds;
BEGIN
 PERFORM pg_advisory_xact_lock(hashtextextended(p_operation::text,0));
 SELECT * INTO STRICT reserved FROM core.holds WHERE operation_id=p_operation;
 PERFORM 1 FROM core.ledger_accounts WHERE id=reserved.account_id FOR UPDATE;
 IF reserved.state='RELEASED' THEN RETURN; END IF;
 IF reserved.state<>'ACTIVE' OR (reserved.submitted AND p_evidence IS NULL) THEN RAISE EXCEPTION 'Nonexecution evidence required'; END IF;
 UPDATE core.ledger_accounts SET held=held-reserved.amount,version=version+1 WHERE id=reserved.account_id;
 UPDATE core.holds SET state='RELEASED',release_evidence=p_evidence,version=version+1,updated_at=now() WHERE operation_id=p_operation;
END $$;
CREATE FUNCTION core.transfer(p_operation uuid,p_from uuid,p_to uuid,p_amount bigint,p_hold uuid DEFAULT NULL)
RETURNS TABLE(batch_id uuid,created boolean) LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,core AS $$
BEGIN
 IF p_amount IS NULL OR p_amount<=0 OR p_from=p_to THEN RAISE EXCEPTION 'Invalid transfer'; END IF;
 IF NOT EXISTS(SELECT 1 FROM core.ledger_accounts WHERE id=p_from AND kind='CUSTOMER') OR NOT EXISTS(SELECT 1 FROM core.ledger_accounts WHERE id=p_to AND kind='CUSTOMER') THEN RAISE EXCEPTION 'Internal customer accounts required'; END IF;
 RETURN QUERY SELECT * FROM core.post(p_operation,'INTERNAL',jsonb_build_array(jsonb_build_object('accountId',p_from,'delta',-p_amount),jsonb_build_object('accountId',p_to,'delta',p_amount)),p_hold);
END $$;
CREATE FUNCTION core.fund_synthetic(p_operation uuid,p_account uuid,p_clearing uuid,p_amount bigint)
RETURNS TABLE(batch_id uuid,created boolean) LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,core AS $$
BEGIN
 IF p_amount IS NULL OR p_amount<=0 OR NOT EXISTS(SELECT 1 FROM core.ledger_accounts WHERE id=p_clearing AND kind='SETTLEMENT') OR NOT EXISTS(SELECT 1 FROM core.ledger_accounts WHERE id=p_account AND kind='CUSTOMER') THEN RAISE EXCEPTION 'Invalid synthetic funding'; END IF;
 RETURN QUERY SELECT * FROM core.post(p_operation,'SYNTHETIC_FUNDING',jsonb_build_array(jsonb_build_object('accountId',p_clearing,'delta',-p_amount),jsonb_build_object('accountId',p_account,'delta',p_amount)));
END $$;
GRANT EXECUTE ON FUNCTION core.reserve(uuid,uuid,bigint),core.mark_submitted(uuid),core.release_hold(uuid,uuid),core.transfer(uuid,uuid,uuid,bigint,uuid),core.fund_synthetic(uuid,uuid,uuid,bigint) TO bank_core;
