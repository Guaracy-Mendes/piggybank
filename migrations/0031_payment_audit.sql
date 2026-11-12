CREATE TABLE payments.audit_pending(id uuid PRIMARY KEY,record jsonb NOT NULL,delivered_at timestamptz);
CREATE FUNCTION payments.guard_audit_pending() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF TG_OP<>'UPDATE' OR NEW.id<>OLD.id OR NEW.record<>OLD.record OR OLD.delivered_at IS NOT NULL OR NEW.delivered_at IS NULL THEN RAISE EXCEPTION 'Immutable payment audit intent'; END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER immutable_payment_audit BEFORE UPDATE OR DELETE ON payments.audit_pending FOR EACH ROW EXECUTE FUNCTION payments.guard_audit_pending();
CREATE TRIGGER immutable_payment_audit_set BEFORE TRUNCATE ON payments.audit_pending FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal();
