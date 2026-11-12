-- owner: controls. Append is available only through an owner-protected function.
CREATE TABLE controls.audit_heads (
  stream text PRIMARY KEY CHECK(length(stream) BETWEEN 1 AND 128),
  sequence bigint NOT NULL,
  digest text NOT NULL
);
CREATE TABLE controls.audit_events (
  id uuid PRIMARY KEY,
  stream text NOT NULL REFERENCES controls.audit_heads(stream),
  sequence bigint NOT NULL,
  previous_digest text NOT NULL,
  digest text NOT NULL,
  payload jsonb NOT NULL CHECK(jsonb_typeof(payload)='object' AND payload ?& ARRAY['actorIssuer','actorSubject','action','resource','outcome','policyVersion','reasonCode','correlationId','occurredAt']),
  recorded_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE(stream,sequence)
);
REVOKE INSERT,UPDATE,DELETE,TRUNCATE ON controls.audit_events,controls.audit_heads FROM bank_controls;
CREATE FUNCTION controls.reject_audit_mutation() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN RAISE EXCEPTION 'Audit history is append only'; END $$;
CREATE TRIGGER audit_immutable BEFORE UPDATE OR DELETE ON controls.audit_events FOR EACH ROW EXECUTE FUNCTION controls.reject_audit_mutation();
CREATE TRIGGER audit_no_truncate BEFORE TRUNCATE ON controls.audit_events FOR EACH STATEMENT EXECUTE FUNCTION controls.reject_audit_mutation();
CREATE FUNCTION controls.append_audit(p_id uuid,p_stream text,p_payload jsonb) RETURNS text
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,controls AS $$
DECLARE old_event controls.audit_events%ROWTYPE; head controls.audit_heads%ROWTYPE; new_digest text;
BEGIN
  INSERT INTO controls.audit_heads(stream,sequence,digest) VALUES(p_stream,0,repeat('0',64)) ON CONFLICT DO NOTHING;
  SELECT * INTO head FROM controls.audit_heads WHERE stream=p_stream FOR UPDATE;
  SELECT * INTO old_event FROM controls.audit_events WHERE id=p_id;
  IF FOUND THEN
    IF old_event.stream<>p_stream OR old_event.payload<>p_payload THEN RAISE EXCEPTION 'Audit identity conflict'; END IF;
    RETURN old_event.digest;
  END IF;
  new_digest:=encode(sha256(convert_to(head.digest||E'\n'||(head.sequence+1)::text||E'\n'||p_payload::text,'UTF8')),'hex');
  INSERT INTO controls.audit_events(id,stream,sequence,previous_digest,digest,payload)
    VALUES(p_id,p_stream,head.sequence+1,head.digest,new_digest,p_payload);
  UPDATE controls.audit_heads SET sequence=head.sequence+1,digest=new_digest WHERE stream=p_stream;
  RETURN new_digest;
END $$;
GRANT EXECUTE ON FUNCTION controls.append_audit(uuid,text,jsonb) TO bank_controls;
CREATE TABLE controls.evidence_objects (
  id uuid PRIMARY KEY,
  owner_issuer text NOT NULL,
  owner_subject text NOT NULL,
  purpose text NOT NULL CHECK(purpose IN ('IDENTITY','CASE','SYNTHETIC')),
  digest text NOT NULL CHECK(length(digest)=64),
  size bigint NOT NULL CHECK(size BETWEEN 0 AND 8388608),
  state text NOT NULL CHECK(state IN ('STAGED','READY','QUARANTINED')),
  created_at timestamptz NOT NULL DEFAULT now()
);
