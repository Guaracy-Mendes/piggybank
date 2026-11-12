ALTER TABLE controls.change_proposals DROP CONSTRAINT change_proposals_kind_check;
ALTER TABLE controls.change_proposals ADD CONSTRAINT change_proposals_kind_check CHECK(kind IN ('PRODUCT','TERMS','CORRECTION'));
CREATE TABLE documents.terms(
 id uuid PRIMARY KEY,product_version_id uuid NOT NULL,version integer NOT NULL CHECK(version>0),
 effective_from date NOT NULL,effective_until date NOT NULL CHECK(effective_until>effective_from),
 document jsonb NOT NULL,digest text NOT NULL,approval_id uuid NOT NULL UNIQUE,
 maker_person uuid NOT NULL,checker_person uuid NOT NULL,published_at timestamptz NOT NULL DEFAULT now(),
 CHECK(maker_person<>checker_person),CHECK(document->>'label'='SYNTHETIC'),CHECK(octet_length(document->>'text') BETWEEN 1 AND 32768),UNIQUE(product_version_id,version)
);
CREATE TABLE documents.acceptances(
 id uuid PRIMARY KEY DEFAULT gen_random_uuid(),terms_id uuid NOT NULL REFERENCES documents.terms(id),product_version_id uuid NOT NULL,
 application_id uuid NOT NULL,issuer text NOT NULL,subject text NOT NULL,channel text NOT NULL,digest text NOT NULL,
 accepted_at timestamptz NOT NULL DEFAULT now(),UNIQUE(issuer,subject,application_id,terms_id)
);
CREATE FUNCTION documents.immutable_record() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'Immutable document record'; END $$;
CREATE TRIGGER immutable_terms BEFORE UPDATE OR DELETE OR TRUNCATE ON documents.terms FOR EACH STATEMENT EXECUTE FUNCTION documents.immutable_record();
CREATE TRIGGER immutable_acceptances BEFORE UPDATE OR DELETE OR TRUNCATE ON documents.acceptances FOR EACH STATEMENT EXECUTE FUNCTION documents.immutable_record();
REVOKE ALL ON documents.terms,documents.acceptances FROM bank_documents;
GRANT SELECT ON documents.terms,documents.acceptances TO bank_documents;
CREATE FUNCTION documents.publish_terms(p_document jsonb,p_approval uuid,p_maker uuid,p_checker uuid) RETURNS boolean
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,documents AS $$
DECLARE existing documents.terms; product uuid:=(p_document->>'productVersionId')::uuid;
BEGIN
 PERFORM pg_advisory_xact_lock(hashtextextended(product::text,72));
 SELECT * INTO existing FROM documents.terms WHERE id=(p_document->>'id')::uuid;
 IF FOUND THEN IF existing.document<>p_document OR existing.approval_id<>p_approval THEN RAISE EXCEPTION 'Terms identity conflict'; END IF;RETURN false;END IF;
 IF EXISTS(SELECT 1 FROM documents.terms WHERE product_version_id=product AND version>=(p_document->>'version')::integer) THEN RAISE EXCEPTION 'Terms version must advance'; END IF;
 INSERT INTO documents.terms(id,product_version_id,version,effective_from,effective_until,document,digest,approval_id,maker_person,checker_person)
 VALUES((p_document->>'id')::uuid,product,(p_document->>'version')::integer,(p_document->>'effectiveFrom')::date,(p_document->>'effectiveUntil')::date,p_document,encode(sha256(convert_to(p_document->>'text','UTF8')),'hex'),p_approval,p_maker,p_checker);
 RETURN true;
END $$;
CREATE FUNCTION documents.accept_terms(p_terms uuid,p_application uuid,p_issuer text,p_subject text,p_channel text) RETURNS TABLE(acceptance_id uuid,created boolean)
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,documents AS $$
DECLARE terms documents.terms; existing uuid; target uuid;
BEGIN
 SELECT * INTO STRICT terms FROM documents.terms WHERE id=p_terms;
 PERFORM pg_advisory_xact_lock(hashtextextended(terms.product_version_id::text,72));
 IF p_terms IS DISTINCT FROM (SELECT id FROM documents.terms WHERE product_version_id=terms.product_version_id AND effective_from<=(now() AT TIME ZONE 'UTC')::date AND effective_until>(now() AT TIME ZONE 'UTC')::date ORDER BY version DESC LIMIT 1) THEN RAISE EXCEPTION 'Current terms acceptance required'; END IF;
 INSERT INTO documents.acceptances(terms_id,product_version_id,application_id,issuer,subject,channel,digest) VALUES(p_terms,terms.product_version_id,p_application,p_issuer,p_subject,p_channel,terms.digest) ON CONFLICT(issuer,subject,application_id,terms_id) DO NOTHING RETURNING id INTO target;
 IF target IS NOT NULL THEN RETURN QUERY SELECT target,true;RETURN;END IF;
 SELECT id INTO STRICT existing FROM documents.acceptances WHERE issuer=p_issuer AND subject=p_subject AND application_id=p_application AND terms_id=p_terms;
 RETURN QUERY SELECT existing,false;
END $$;
GRANT EXECUTE ON FUNCTION documents.publish_terms(jsonb,uuid,uuid,uuid),documents.accept_terms(uuid,uuid,text,text,text) TO bank_documents;
