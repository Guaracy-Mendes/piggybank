-- Row immutability must also cover owner-level statement truncation.
CREATE TRIGGER immutable_configuration_history
BEFORE TRUNCATE ON controls.configuration_drafts
FOR EACH STATEMENT EXECUTE FUNCTION controls.immutable_check_result();
