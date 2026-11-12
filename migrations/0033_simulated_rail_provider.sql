CREATE TABLE simulator.rail_recipients(reference text PRIMARY KEY,enabled boolean NOT NULL DEFAULT true,currency text NOT NULL DEFAULT 'TST' CHECK(currency='TST'));
INSERT INTO simulator.rail_recipients VALUES('SIM00000000000000000000000000000001',true,'TST');
CREATE TABLE simulator.rail_instructions(
 id uuid PRIMARY KEY,fingerprint text NOT NULL,instruction jsonb NOT NULL,state text NOT NULL CHECK(state IN ('ACCEPTED','PENDING','SETTLED','REJECTED','CANCELLED')),
 version bigint NOT NULL,evidence_id uuid NOT NULL,created_at timestamptz NOT NULL DEFAULT now()
);
CREATE TABLE simulator.rail_events(id uuid PRIMARY KEY,instruction_id uuid NOT NULL REFERENCES simulator.rail_instructions(id),version bigint NOT NULL,document jsonb NOT NULL,UNIQUE(instruction_id,version));
CREATE TRIGGER immutable_rail_events BEFORE UPDATE OR DELETE OR TRUNCATE ON simulator.rail_events FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal();
CREATE TABLE simulator.rail_scenarios(instruction_id uuid PRIMARY KEY,mode text NOT NULL CHECK(mode IN ('ACCEPT','REJECT','LOST_RESPONSE','BEFORE_ACCEPT_TIMEOUT','CONFLICT')));
CREATE FUNCTION simulator.guard_rail_identity() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF TG_OP<>'UPDATE' OR NEW.id<>OLD.id OR NEW.fingerprint<>OLD.fingerprint OR NEW.instruction<>OLD.instruction OR NEW.created_at<>OLD.created_at
 OR OLD.state IN ('SETTLED','REJECTED','CANCELLED') OR NEW.version<>OLD.version+1 THEN RAISE EXCEPTION 'Immutable provider instruction or final outcome'; END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER immutable_rail_identity BEFORE UPDATE OR DELETE ON simulator.rail_instructions FOR EACH ROW EXECUTE FUNCTION simulator.guard_rail_identity();
CREATE TRIGGER immutable_rail_set BEFORE TRUNCATE ON simulator.rail_instructions FOR EACH STATEMENT EXECUTE FUNCTION core.immutable_journal();
CREATE UNIQUE INDEX rail_event_head_scope ON simulator.rail_events(id,instruction_id,version);
ALTER TABLE simulator.rail_instructions ADD CONSTRAINT rail_current_evidence_scope
FOREIGN KEY(evidence_id,id,version) REFERENCES simulator.rail_events(id,instruction_id,version) DEFERRABLE INITIALLY DEFERRED;
