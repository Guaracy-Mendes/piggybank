-- A verified person may be explicitly linked before approved onboarding creates a bank party.
-- Materialize that existing canonical ID; never replace a conflicting bank identity or merge by contact data.
CREATE FUNCTION core.ensure_party_linked(p_issuer text,p_subject text,p_application uuid,p_digest text,p_person uuid) RETURNS uuid
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,core AS $$
DECLARE existing core.parties; target uuid:=COALESCE(p_person,gen_random_uuid());
BEGIN
 PERFORM pg_advisory_xact_lock(hashtextextended(p_issuer||E'\n'||p_subject,3));
 SELECT * INTO existing FROM core.parties WHERE issuer=p_issuer AND subject=p_subject;
 IF FOUND THEN
  IF existing.identity_digest<>p_digest OR (p_person IS NOT NULL AND existing.id<>p_person) THEN RAISE EXCEPTION 'Canonical bank identity change requires review'; END IF;
  RETURN existing.id;
 END IF;
 IF EXISTS(SELECT 1 FROM core.parties WHERE id=target) THEN RAISE EXCEPTION 'Canonical person belongs to another bank identity'; END IF;
 INSERT INTO core.parties(id,issuer,subject,source_application,identity_digest) VALUES(target,p_issuer,p_subject,p_application,p_digest);
 RETURN target;
END $$;
GRANT EXECUTE ON FUNCTION core.ensure_party_linked(text,text,uuid,text,uuid) TO bank_core;
