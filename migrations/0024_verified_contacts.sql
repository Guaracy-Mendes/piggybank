ALTER TABLE core.parties ADD COLUMN profile_version bigint NOT NULL DEFAULT 1;
CREATE TABLE core.contacts(party_id uuid PRIMARY KEY REFERENCES core.parties(id),email text NOT NULL,version bigint NOT NULL,verification_resource uuid NOT NULL,evidence jsonb NOT NULL,verified_at timestamptz NOT NULL DEFAULT now());
CREATE TABLE core.contact_changes(id uuid PRIMARY KEY,party_id uuid NOT NULL REFERENCES core.parties(id),fingerprint text NOT NULL,email text NOT NULL,version bigint NOT NULL,evidence jsonb NOT NULL,created_at timestamptz NOT NULL DEFAULT now());
REVOKE ALL ON core.contacts,core.contact_changes FROM bank_core;
GRANT SELECT ON core.contacts,core.contact_changes TO bank_core;
CREATE TRIGGER immutable_contact_changes BEFORE UPDATE OR DELETE OR TRUNCATE ON core.contact_changes FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal();
CREATE FUNCTION core.change_contact(p_id uuid,p_party uuid,p_email text,p_expected bigint,p_evidence jsonb) RETURNS TABLE(version bigint,created boolean)
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,core AS $$
DECLARE current_version bigint; old core.contact_changes; digest text;
BEGIN
 SELECT profile_version INTO STRICT current_version FROM core.parties WHERE id=p_party FOR UPDATE;
 digest:=encode(sha256(convert_to(jsonb_build_array(p_party,p_email,p_expected)::text,'UTF8')),'hex');
 SELECT * INTO old FROM core.contact_changes WHERE id=p_id;
 IF FOUND THEN IF old.fingerprint<>digest THEN RAISE EXCEPTION 'Contact change identity conflict'; END IF;RETURN QUERY SELECT old.version,false;RETURN;END IF;
 IF current_version<>p_expected OR NOT COALESCE((p_evidence->>'expiresAt')::timestamptz>now() AND length(p_evidence->>'revision')>0,false) THEN RAISE EXCEPTION 'Current verified contact version required'; END IF;
 current_version:=current_version+1;
 UPDATE core.parties SET profile_version=current_version WHERE id=p_party;
 INSERT INTO core.contacts VALUES(p_party,p_email,current_version,p_id,p_evidence,now()) ON CONFLICT(party_id) DO UPDATE SET email=EXCLUDED.email,version=EXCLUDED.version,verification_resource=EXCLUDED.verification_resource,evidence=EXCLUDED.evidence,verified_at=EXCLUDED.verified_at;
 INSERT INTO core.contact_changes VALUES(p_id,p_party,digest,p_email,current_version,p_evidence,now());
 RETURN QUERY SELECT current_version,true;
END $$;
GRANT EXECUTE ON FUNCTION core.change_contact(uuid,uuid,text,bigint,jsonb) TO bank_core;
CREATE TABLE journeys.contact_changes(id uuid PRIMARY KEY,issuer text NOT NULL,subject text NOT NULL,key text NOT NULL,actor_context jsonb NOT NULL,email text NOT NULL,expected_version bigint NOT NULL,input_digest text NOT NULL,operation_id uuid,state text NOT NULL DEFAULT 'PENDING',created_at timestamptz NOT NULL DEFAULT now(),UNIQUE(issuer,subject,key));
