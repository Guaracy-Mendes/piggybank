-- Explicit current-policy recovery; ordinary opening-key conflicts still fail.
CREATE TABLE core.opening_revisions(account_id uuid NOT NULL REFERENCES core.account_registry(id),revision bigint NOT NULL,previous_acceptance uuid NOT NULL,new_acceptance uuid NOT NULL,evidence jsonb NOT NULL,created_at timestamptz NOT NULL DEFAULT now(),PRIMARY KEY(account_id,revision));
REVOKE ALL ON core.opening_revisions FROM bank_core;
GRANT SELECT ON core.opening_revisions TO bank_core;
CREATE TRIGGER immutable_opening_revisions BEFORE UPDATE OR DELETE OR TRUNCATE ON core.opening_revisions FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal();
CREATE FUNCTION core.rebind_staged_terms(p_application uuid,p_party uuid,p_product uuid,p_acceptance uuid,p_digest text,p_evidence jsonb) RETURNS uuid
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,core AS $$
DECLARE registry core.account_registry; account core.ledger_accounts;
BEGIN
 PERFORM pg_advisory_xact_lock(hashtextextended(p_application::text,4));
 SELECT * INTO STRICT registry FROM core.account_registry WHERE application_id=p_application;
 SELECT * INTO STRICT account FROM core.ledger_accounts WHERE id=registry.id FOR UPDATE;
 IF registry.party_id<>p_party OR registry.product_version_id<>p_product OR account.state<>'PROVISIONING' OR account.booked<>0 OR account.held<>0 THEN RAISE EXCEPTION 'Restricted opening recovery required'; END IF;
 IF NOT COALESCE((p_evidence->>'applicationId')::uuid=p_application AND (p_evidence->>'partyId')::uuid=p_party AND (p_evidence->>'productVersionId')::uuid=p_product AND (p_evidence->>'acceptanceId')::uuid=p_acceptance AND (p_evidence->>'expiresAt')::timestamptz>now() AND length(p_evidence->>'controlsRevision')>0,false) THEN RAISE EXCEPTION 'Current bound opening evidence required'; END IF;
 IF registry.acceptance_id=p_acceptance THEN RETURN registry.id; END IF;
 INSERT INTO core.opening_revisions VALUES(registry.id,registry.revision+1,registry.acceptance_id,p_acceptance,p_evidence,now());
 UPDATE core.account_registry SET acceptance_id=p_acceptance,opening_digest=p_digest,revision=revision+1 WHERE id=registry.id;
 RETURN registry.id;
END $$;
GRANT EXECUTE ON FUNCTION core.rebind_staged_terms(uuid,uuid,uuid,uuid,text,jsonb) TO bank_core;
