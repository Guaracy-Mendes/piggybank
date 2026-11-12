CREATE TABLE core.parties(id uuid PRIMARY KEY DEFAULT gen_random_uuid(),issuer text NOT NULL,subject text NOT NULL,source_application uuid NOT NULL UNIQUE,identity_digest text NOT NULL,created_at timestamptz NOT NULL DEFAULT now(),UNIQUE(issuer,subject));
CREATE TABLE core.duplicate_suspicions(id uuid PRIMARY KEY DEFAULT gen_random_uuid(),party_id uuid NOT NULL REFERENCES core.parties(id),other_party uuid NOT NULL REFERENCES core.parties(id),reason_code text NOT NULL,state text NOT NULL DEFAULT 'REVIEW_REQUIRED',CHECK(party_id<>other_party),UNIQUE(party_id,other_party));
CREATE TABLE core.account_registry(
 id uuid PRIMARY KEY REFERENCES core.ledger_accounts(id),application_id uuid NOT NULL UNIQUE,
 party_id uuid NOT NULL REFERENCES core.parties(id),product_version_id uuid NOT NULL REFERENCES core.product_versions(id),
 acceptance_id uuid NOT NULL,opening_digest text NOT NULL,activation_evidence jsonb,
 revision bigint NOT NULL DEFAULT 1,created_at timestamptz NOT NULL DEFAULT now()
);
CREATE TABLE core.account_relationships(account_id uuid NOT NULL REFERENCES core.account_registry(id),person_id uuid NOT NULL,kind text NOT NULL CHECK(kind IN ('OWNER','SUPPORT')),revoked_at timestamptz,PRIMARY KEY(account_id,person_id,kind));
CREATE TABLE core.obligations(operation_id uuid PRIMARY KEY,account_id uuid NOT NULL REFERENCES core.ledger_accounts(id),kind text NOT NULL,state text NOT NULL CHECK(state IN ('OPEN','RESOLVED')));
REVOKE ALL ON core.parties,core.account_registry,core.account_relationships,core.obligations FROM bank_core;
GRANT SELECT ON core.parties,core.account_registry,core.account_relationships,core.obligations TO bank_core;
CREATE FUNCTION core.ensure_party(p_issuer text,p_subject text,p_application uuid,p_digest text) RETURNS uuid
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,core AS $$
DECLARE existing core.parties;
BEGIN
 PERFORM pg_advisory_xact_lock(hashtextextended(p_issuer||E'\n'||p_subject,3));
 SELECT * INTO existing FROM core.parties WHERE issuer=p_issuer AND subject=p_subject;
 IF FOUND THEN
  IF existing.identity_digest<>p_digest THEN RAISE EXCEPTION 'Legal identity change requires review'; END IF;
  RETURN existing.id;
 END IF;
 INSERT INTO core.parties(issuer,subject,source_application,identity_digest) VALUES(p_issuer,p_subject,p_application,p_digest) RETURNING * INTO existing;
 RETURN existing.id;
END $$;
CREATE FUNCTION core.stage_account(p_application uuid,p_party uuid,p_product uuid,p_acceptance uuid,p_digest text) RETURNS uuid
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,core AS $$
DECLARE existing core.account_registry; target uuid:=gen_random_uuid();
BEGIN
 PERFORM pg_advisory_xact_lock(hashtextextended(p_application::text,4));
 SELECT * INTO existing FROM core.account_registry WHERE application_id=p_application;
 IF FOUND THEN
  IF existing.opening_digest<>p_digest THEN RAISE EXCEPTION 'Account opening identity conflict'; END IF;
  RETURN existing.id;
 END IF;
 INSERT INTO core.ledger_accounts(id,kind) VALUES(target,'CUSTOMER');
 INSERT INTO core.account_registry(id,application_id,party_id,product_version_id,acceptance_id,opening_digest) VALUES(target,p_application,p_party,p_product,p_acceptance,p_digest);
 INSERT INTO core.account_relationships VALUES(target,p_party,'OWNER',NULL);
 RETURN target;
END $$;
CREATE FUNCTION core.activate_account(p_account uuid,p_evidence jsonb) RETURNS void
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,core AS $$
DECLARE registry core.account_registry; account core.ledger_accounts;
BEGIN
 SELECT * INTO STRICT account FROM core.ledger_accounts WHERE id=p_account FOR UPDATE;
 SELECT * INTO STRICT registry FROM core.account_registry WHERE id=p_account;
 IF NOT COALESCE(registry.application_id=(p_evidence->>'applicationId')::uuid AND registry.party_id=(p_evidence->>'partyId')::uuid AND registry.product_version_id=(p_evidence->>'productVersionId')::uuid AND registry.acceptance_id=(p_evidence->>'acceptanceId')::uuid AND (p_evidence->>'expiresAt')::timestamptz>now() AND length(p_evidence->>'controlsRevision')>0,false) THEN RAISE EXCEPTION 'Activation evidence mismatch or expired'; END IF;
 IF NOT EXISTS(SELECT 1 FROM core.product_versions WHERE id=registry.product_version_id AND effective_from<=(now() AT TIME ZONE 'UTC')::date AND effective_until>(now() AT TIME ZONE 'UTC')::date) THEN RAISE EXCEPTION 'Offer no longer effective'; END IF;
 IF account.state='ACTIVE' THEN RETURN; END IF;
 IF account.state<>'PROVISIONING' THEN RAISE EXCEPTION 'Account cannot activate'; END IF;
 UPDATE core.ledger_accounts SET state='ACTIVE',debit_blocked=false,version=version+1 WHERE id=p_account;
 UPDATE core.account_registry SET activation_evidence=p_evidence,revision=revision+1 WHERE id=p_account;
END $$;
CREATE FUNCTION core.restrict_account(p_account uuid,p_debit boolean,p_credit boolean,p_freeze_only boolean) RETURNS void
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,core AS $$
BEGIN
 PERFORM 1 FROM core.ledger_accounts WHERE id=p_account AND state='ACTIVE' FOR UPDATE;
 IF NOT FOUND THEN RAISE EXCEPTION 'Active account required'; END IF;
 IF p_freeze_only AND (NOT p_debit OR NOT EXISTS(SELECT 1 FROM core.ledger_accounts WHERE id=p_account AND credit_blocked=p_credit)) THEN RAISE EXCEPTION 'Freeze cannot remove restrictions'; END IF;
 UPDATE core.ledger_accounts SET debit_blocked=p_debit,credit_blocked=p_credit,version=version+1 WHERE id=p_account AND (debit_blocked<>p_debit OR credit_blocked<>p_credit);
 IF FOUND THEN UPDATE core.account_registry SET revision=revision+1 WHERE id=p_account; END IF;
END $$;
CREATE FUNCTION core.close_account(p_account uuid) RETURNS void
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,core AS $$
DECLARE account core.ledger_accounts;
BEGIN
 SELECT * INTO STRICT account FROM core.ledger_accounts WHERE id=p_account FOR UPDATE;
 IF account.state='CLOSED' THEN RETURN; END IF;
 IF account.state NOT IN ('ACTIVE','CLOSING') OR account.booked<>0 OR account.held<>0 OR EXISTS(SELECT 1 FROM core.obligations WHERE account_id=p_account AND state='OPEN') THEN RAISE EXCEPTION 'Residual value or obligations block closure'; END IF;
 UPDATE core.ledger_accounts SET state='CLOSED',debit_blocked=true,credit_blocked=true,version=version+1 WHERE id=p_account;
 UPDATE core.account_registry SET revision=revision+1 WHERE id=p_account;
END $$;
CREATE FUNCTION core.set_obligation(p_operation uuid,p_account uuid,p_kind text,p_resolved boolean) RETURNS void
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,core AS $$
BEGIN
 -- Same account lock as closure and postings; new obligations cannot race a close.
 PERFORM 1 FROM core.ledger_accounts WHERE id=p_account AND state='ACTIVE' FOR UPDATE;
 IF NOT FOUND AND NOT p_resolved THEN RAISE EXCEPTION 'Active account required'; END IF;
 INSERT INTO core.obligations VALUES(p_operation,p_account,p_kind,CASE WHEN p_resolved THEN 'RESOLVED' ELSE 'OPEN' END) ON CONFLICT(operation_id) DO NOTHING;
 IF NOT EXISTS(SELECT 1 FROM core.obligations WHERE operation_id=p_operation AND account_id=p_account AND kind=p_kind) THEN RAISE EXCEPTION 'Obligation identity conflict'; END IF;
 IF p_resolved THEN UPDATE core.obligations SET state='RESOLVED' WHERE operation_id=p_operation; END IF;
END $$;
GRANT EXECUTE ON FUNCTION core.ensure_party(text,text,uuid,text),core.stage_account(uuid,uuid,uuid,uuid,text),core.activate_account(uuid,jsonb),core.restrict_account(uuid,boolean,boolean,boolean),core.close_account(uuid),core.set_obligation(uuid,uuid,text,boolean) TO bank_core;
