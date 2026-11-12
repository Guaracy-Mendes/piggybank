CREATE FUNCTION payments.guard_intent_identity() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF TG_OP<>'UPDATE' OR NEW.id<>OLD.id OR NEW.issuer<>OLD.issuer OR NEW.subject<>OLD.subject OR NEW.client_id<>OLD.client_id
 OR NEW.request_key<>OLD.request_key OR NEW.quote_id<>OLD.quote_id OR NEW.actor_context<>OLD.actor_context OR NEW.created_at<>OLD.created_at
 OR (OLD.challenge_id IS NOT NULL AND NEW.challenge_id IS DISTINCT FROM OLD.challenge_id)
 OR (OLD.operation_id IS NOT NULL AND NEW.operation_id IS DISTINCT FROM OLD.operation_id)
 OR (OLD.batch_id IS NOT NULL AND NEW.batch_id IS DISTINCT FROM OLD.batch_id)
 THEN RAISE EXCEPTION 'Immutable payment identity'; END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER immutable_intent_identity BEFORE UPDATE OR DELETE ON payments.intents FOR EACH ROW EXECUTE FUNCTION payments.guard_intent_identity();
CREATE TRIGGER immutable_intent_set BEFORE TRUNCATE ON payments.intents FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal();
CREATE FUNCTION payments.guard_beneficiary() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF TG_OP<>'UPDATE' OR NEW.id<>OLD.id OR NEW.issuer<>OLD.issuer OR NEW.subject<>OLD.subject OR NEW.request_key<>OLD.request_key
 OR NEW.label<>OLD.label OR NEW.receiving_reference<>OLD.receiving_reference OR NEW.payee_account_id<>OLD.payee_account_id
 OR NEW.fingerprint<>OLD.fingerprint OR NEW.verified_at<>OLD.verified_at OR OLD.state<>'VERIFIED' OR NEW.state<>'REVOKED'
 OR NEW.version<>OLD.version+1 THEN RAISE EXCEPTION 'Immutable verified beneficiary'; END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER immutable_beneficiary BEFORE UPDATE OR DELETE ON payments.beneficiaries FOR EACH ROW EXECUTE FUNCTION payments.guard_beneficiary();
CREATE TRIGGER immutable_beneficiary_set BEFORE TRUNCATE ON payments.beneficiaries FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal();
CREATE TRIGGER immutable_receiving_reference BEFORE UPDATE OR DELETE OR TRUNCATE ON core.receiving_references FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal();
