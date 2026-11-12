-- Qualify the PL/pgSQL record independently of the terms table name.
CREATE OR REPLACE FUNCTION documents.accept_terms(p_terms uuid,p_application uuid,p_issuer text,p_subject text,p_channel text) RETURNS TABLE(acceptance_id uuid,created boolean)
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,documents AS $$
DECLARE accepted_terms documents.terms; existing uuid; target uuid;
BEGIN
 SELECT * INTO STRICT accepted_terms FROM documents.terms WHERE id=p_terms;
 PERFORM pg_advisory_xact_lock(hashtextextended(accepted_terms.product_version_id::text,72));
 IF p_terms IS DISTINCT FROM (SELECT id FROM documents.terms WHERE product_version_id=accepted_terms.product_version_id AND effective_from<=(now() AT TIME ZONE 'UTC')::date AND effective_until>(now() AT TIME ZONE 'UTC')::date ORDER BY version DESC LIMIT 1) THEN RAISE EXCEPTION 'Current terms acceptance required'; END IF;
 INSERT INTO documents.acceptances(terms_id,product_version_id,application_id,issuer,subject,channel,digest) VALUES(p_terms,accepted_terms.product_version_id,p_application,p_issuer,p_subject,p_channel,accepted_terms.digest) ON CONFLICT(issuer,subject,application_id,terms_id) DO NOTHING RETURNING id INTO target;
 IF target IS NOT NULL THEN RETURN QUERY SELECT target,true;RETURN;END IF;
 SELECT id INTO STRICT existing FROM documents.acceptances WHERE issuer=p_issuer AND subject=p_subject AND application_id=p_application AND terms_id=p_terms;
 RETURN QUERY SELECT existing,false;
END $$;
