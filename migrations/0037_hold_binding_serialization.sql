-- Serialize generic hold commands with creation of a bound external reservation.
-- Validation happens after the same operation lock, including uncommitted reservations.
REVOKE EXECUTE ON FUNCTION core.release_hold(uuid,uuid),core.mark_submitted(uuid) FROM bank_core;
CREATE OR REPLACE FUNCTION core.release_unsubmitted(p_operation uuid) RETURNS void
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,core AS $$
BEGIN
 PERFORM pg_advisory_xact_lock(hashtextextended(p_operation::text,0));
 IF EXISTS(SELECT 1 FROM core.external_reservations WHERE operation_id=p_operation) THEN RAISE EXCEPTION 'Use bound external lifecycle release'; END IF;
 PERFORM core.release_hold(p_operation,NULL);
END $$;
CREATE OR REPLACE FUNCTION core.mark_generic_submitted(p_operation uuid) RETURNS void
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,core AS $$
BEGIN
 PERFORM pg_advisory_xact_lock(hashtextextended(p_operation::text,0));
 IF EXISTS(SELECT 1 FROM core.external_reservations WHERE operation_id=p_operation) THEN RAISE EXCEPTION 'Use bound external submission authority'; END IF;
 PERFORM core.mark_submitted(p_operation);
END $$;
CREATE OR REPLACE FUNCTION core.transfer(p_operation uuid,p_from uuid,p_to uuid,p_amount bigint,p_hold uuid DEFAULT NULL)
RETURNS TABLE(batch_id uuid,created boolean) LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,core AS $$
BEGIN
 -- Take both operation locks in one order before checking the hold's domain owner.
 PERFORM pg_advisory_xact_lock(k) FROM (SELECT DISTINCT hashtextextended(id::text,0) AS k FROM unnest(ARRAY[p_operation,p_hold]) AS ids(id) WHERE id IS NOT NULL ORDER BY k) AS keys;
 IF p_amount IS NULL OR p_amount<=0 OR p_from=p_to THEN RAISE EXCEPTION 'Invalid transfer'; END IF;
 IF EXISTS(SELECT 1 FROM core.external_reservations WHERE operation_id=p_hold) THEN RAISE EXCEPTION 'Use authenticated external settlement'; END IF;
 IF NOT EXISTS(SELECT 1 FROM core.ledger_accounts WHERE id=p_from AND kind='CUSTOMER') OR NOT EXISTS(SELECT 1 FROM core.ledger_accounts WHERE id=p_to AND kind='CUSTOMER') THEN RAISE EXCEPTION 'Internal customer accounts required'; END IF;
 RETURN QUERY SELECT * FROM core.post(p_operation,'INTERNAL',jsonb_build_array(jsonb_build_object('accountId',p_from,'delta',-p_amount),jsonb_build_object('accountId',p_to,'delta',p_amount)),p_hold);
END $$;
GRANT EXECUTE ON FUNCTION core.release_unsubmitted(uuid),core.mark_generic_submitted(uuid) TO bank_core;
