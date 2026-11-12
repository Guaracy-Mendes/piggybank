-- Recheck live authority AFTER waiting for the product/rule publication lock.
-- Transaction-start now() can predate an approval or role expiry during that wait.
ALTER TABLE core.periodic_policies ALTER COLUMN published_at SET DEFAULT clock_timestamp();
CREATE OR REPLACE FUNCTION core.publish_periodic_policy(p_document jsonb,p_digest text,p_approval uuid,p_maker uuid,p_checker uuid) RETURNS boolean
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,core AS $$
DECLARE original core.periodic_policies; prior core.periodic_policies; product core.product_versions;
 policy_id uuid:=(p_document->>'id')::uuid; product_id uuid:=(p_document->>'productVersionId')::uuid;
 code_value text:=p_document->>'code'; version_value integer:=(p_document->>'version')::integer;
 from_value date:=(p_document->>'effectiveFrom')::date; until_value date:=(p_document->>'effectiveUntil')::date;
BEGIN
 PERFORM pg_advisory_xact_lock(hashtextextended(product_id::text||'/'||code_value,176));
 SELECT * INTO original FROM core.periodic_policies WHERE id=policy_id;
 IF FOUND THEN
  IF original.document<>p_document OR original.digest<>p_digest OR original.approval_id<>p_approval OR original.maker_person<>p_maker OR original.checker_person<>p_checker THEN RAISE EXCEPTION 'Periodic policy identity conflict'; END IF;
  RETURN false;
 END IF;
 IF encode(sha256(convert_to(core.periodic_policy_canonical(p_document),'UTF8')),'hex')<>p_digest THEN RAISE EXCEPTION 'Periodic policy digest mismatch'; END IF;
 IF NOT EXISTS(SELECT 1 FROM controls.change_proposals WHERE id=p_approval AND kind='PERIODIC_POLICY' AND resource_id=policy_id AND digest=p_digest AND maker_person=p_maker AND checker_person=p_checker AND state='APPROVED' AND expires_at>clock_timestamp()) THEN RAISE EXCEPTION 'Exact approved periodic policy required'; END IF;
 IF p_maker=p_checker OR NOT EXISTS(SELECT 1 FROM controls.role_grants WHERE person_id=p_maker AND role='finance_adjuster' AND scope IS NULL AND revoked_at IS NULL AND valid_from<=clock_timestamp() AND valid_until>clock_timestamp()) OR NOT EXISTS(SELECT 1 FROM controls.role_grants WHERE person_id=p_checker AND role='finance_approver' AND scope IS NULL AND revoked_at IS NULL AND valid_from<=clock_timestamp() AND valid_until>clock_timestamp()) OR NOT EXISTS(SELECT 1 FROM controls.case_queue_members WHERE person_id=p_maker AND queue='finance-operations' AND capability='REVIEW' AND revoked_at IS NULL) OR NOT EXISTS(SELECT 1 FROM controls.case_queue_members WHERE person_id=p_checker AND queue='finance-operations' AND capability='APPROVE' AND revoked_at IS NULL) THEN RAISE EXCEPTION 'Current independent Finance role and queue required'; END IF;
 SELECT * INTO STRICT product FROM core.product_versions WHERE id=product_id;
 IF from_value<product.effective_from OR until_value>product.effective_until THEN RAISE EXCEPTION 'Periodic policy outside product validity'; END IF;
 SELECT * INTO prior FROM core.periodic_policies WHERE product_version_id=product_id AND code=code_value ORDER BY version DESC LIMIT 1;
 IF FOUND THEN
  IF version_value<>prior.version+1 OR from_value<prior.effective_until OR from_value<(clock_timestamp() AT TIME ZONE 'UTC')::date OR (prior.document->'calculation'->>'kind') IS DISTINCT FROM (p_document->'calculation'->>'kind') THEN RAISE EXCEPTION 'Sequential future nonoverlapping periodic version required'; END IF;
 ELSE
  IF version_value<>1 THEN RAISE EXCEPTION 'First periodic version must be one'; END IF;
 END IF;
 INSERT INTO core.periodic_policies(id,product_version_id,code,version,effective_from,effective_until,document,digest,approval_id,maker_person,checker_person)
 VALUES(policy_id,product_id,code_value,version_value,from_value,until_value,p_document,p_digest,p_approval,p_maker,p_checker);
 RETURN true;
END $$;
