-- Controls owns immutable proposed configuration and acknowledgement of publication.
CREATE TABLE controls.configuration_drafts(
 proposal_id uuid PRIMARY KEY REFERENCES controls.change_proposals(id),
 resource_id uuid NOT NULL UNIQUE, kind text NOT NULL CHECK(kind IN ('PRODUCT','TERMS')),
 details jsonb NOT NULL, published_at timestamptz
);
CREATE FUNCTION controls.guard_configuration_draft() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF TG_OP<>'UPDATE' OR NEW.proposal_id<>OLD.proposal_id OR NEW.resource_id<>OLD.resource_id
 OR NEW.kind<>OLD.kind OR NEW.details<>OLD.details OR OLD.published_at IS NOT NULL
 OR NEW.published_at IS NULL THEN RAISE EXCEPTION 'Immutable configuration draft'; END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER immutable_configuration_draft BEFORE UPDATE OR DELETE ON controls.configuration_drafts
FOR EACH ROW EXECUTE FUNCTION controls.guard_configuration_draft();
