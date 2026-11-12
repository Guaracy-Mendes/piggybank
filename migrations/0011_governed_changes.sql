CREATE TABLE controls.change_proposals(
 id uuid PRIMARY KEY,kind text NOT NULL CHECK(kind IN ('PRODUCT','CORRECTION')),resource_id uuid NOT NULL,digest text NOT NULL,
 maker_person uuid NOT NULL,checker_person uuid,state text NOT NULL DEFAULT 'PENDING' CHECK(state IN ('PENDING','APPROVED')),
 expires_at timestamptz NOT NULL,created_at timestamptz NOT NULL DEFAULT now(),CHECK(checker_person IS NULL OR checker_person<>maker_person)
);
CREATE TABLE core.correction_receipts(approval_id uuid PRIMARY KEY,original_batch uuid NOT NULL UNIQUE REFERENCES core.journal_batches(id),maker_person uuid NOT NULL,checker_person uuid NOT NULL,CHECK(maker_person<>checker_person));
REVOKE ALL ON core.correction_receipts FROM bank_core;
GRANT SELECT ON core.correction_receipts TO bank_core;
CREATE UNIQUE INDEX one_correction_per_batch ON core.journal_batches(original_batch) WHERE posting_kind='CORRECTION';
CREATE FUNCTION core.reverse_batch(p_operation uuid,p_original uuid,p_approval uuid,p_maker uuid,p_checker uuid)
RETURNS TABLE(batch_id uuid,created boolean) LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,core AS $$
DECLARE legs jsonb;
BEGIN
 IF p_maker IS NULL OR p_checker IS NULL OR p_maker=p_checker THEN RAISE EXCEPTION 'Independent correction approval required'; END IF;
 PERFORM pg_advisory_xact_lock(hashtextextended(p_original::text,1));
 IF NOT EXISTS(SELECT 1 FROM core.journal_batches WHERE id=p_original AND posting_kind<>'CORRECTION') THEN RAISE EXCEPTION 'Original journal batch required'; END IF;
 INSERT INTO core.correction_receipts VALUES(p_approval,p_original,p_maker,p_checker) ON CONFLICT(approval_id) DO NOTHING;
 IF NOT EXISTS(SELECT 1 FROM core.correction_receipts WHERE approval_id=p_approval AND original_batch=p_original AND maker_person=p_maker AND checker_person=p_checker) THEN RAISE EXCEPTION 'Correction approval conflict'; END IF;
 SELECT jsonb_agg(jsonb_build_object('accountId',account_id,'delta',-delta) ORDER BY account_id) INTO legs FROM core.journal_entries WHERE core.journal_entries.batch_id=p_original;
 RETURN QUERY SELECT * FROM core.post(p_operation,'CORRECTION',legs,NULL,p_original);
END $$;
GRANT EXECUTE ON FUNCTION core.reverse_batch(uuid,uuid,uuid,uuid,uuid) TO bank_core;
