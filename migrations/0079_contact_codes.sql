-- owner: identity (codes) and simulator (outbox). UI-03 local template: a customer proves they own the email held by
-- the issuer with a one-time 6-digit code. Delivery is SIMULATED: the code is written to a synthetic outbox owned by
-- the simulator module; no real email or SMS is sent. Codes are stored only as salted SHA-256 digests, expire after
-- 10 minutes, allow 5 attempts, can be re-sent after 45 seconds and are limited to 5 per subject per hour.
-- The identity role cannot touch either table directly; it uses the functions below.
CREATE TABLE identity.contact_codes(
 id uuid PRIMARY KEY,issuer text NOT NULL,subject text NOT NULL,channel text NOT NULL CHECK(channel='EMAIL'),
 destination text NOT NULL CHECK(length(destination) BETWEEN 3 AND 254),code_hash text NOT NULL CHECK(code_hash ~ '^[0-9a-f]{64}$'),
 created_at timestamptz NOT NULL DEFAULT clock_timestamp(),expires_at timestamptz NOT NULL,resend_after timestamptz NOT NULL,
 attempts integer NOT NULL DEFAULT 0 CHECK(attempts BETWEEN 0 AND 5),consumed_at timestamptz,superseded_at timestamptz,
 CHECK(expires_at=created_at+interval '10 minutes' AND resend_after=created_at+interval '45 seconds'),
 CHECK(consumed_at IS NULL OR superseded_at IS NULL)
);
CREATE INDEX contact_codes_subject ON identity.contact_codes(issuer,subject,created_at DESC);
CREATE TABLE simulator.contact_outbox(
 id uuid PRIMARY KEY,channel text NOT NULL CHECK(channel='EMAIL'),destination text NOT NULL,subject_line text NOT NULL,body text NOT NULL,
 created_at timestamptz NOT NULL DEFAULT clock_timestamp()
);
CREATE INDEX contact_outbox_destination ON simulator.contact_outbox(destination,created_at DESC);
REVOKE ALL ON identity.contact_codes FROM bank_identity;
REVOKE ALL ON simulator.contact_outbox FROM bank_simulator;
GRANT SELECT ON simulator.contact_outbox TO bank_simulator;

-- Simulated delivery: the only way to write the outbox.
CREATE FUNCTION simulator.deliver_contact_code(p_channel text,p_destination text,p_code text) RETURNS uuid
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,simulator AS $$
DECLARE v_id uuid:=gen_random_uuid();BEGIN
 IF p_channel<>'EMAIL' OR p_code !~ '^[0-9]{6}$' THEN RAISE EXCEPTION 'Invalid simulated delivery';END IF;
 INSERT INTO simulator.contact_outbox(id,channel,destination,subject_line,body)
 VALUES(v_id,p_channel,p_destination,'Your PiggyBank verification code',
  'Your synthetic PiggyBank verification code is '||p_code||'. It expires in 10 minutes. This is a local simulated message.');
 RETURN v_id;
END $$;

CREATE FUNCTION identity.contact_code_status(p_issuer text,p_subject text) RETURNS jsonb
LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog,identity AS $$
 SELECT jsonb_build_object('id',c.id,'expiresAt',c.expires_at,'resendAfter',c.resend_after,'attemptsLeft',5-c.attempts,
  'expired',c.expires_at<=clock_timestamp(),'locked',c.attempts>=5)
 FROM identity.contact_codes c WHERE c.issuer=p_issuer AND c.subject=p_subject AND c.consumed_at IS NULL AND c.superseded_at IS NULL
 ORDER BY c.created_at DESC LIMIT 1
$$;

-- Issues a new code (superseding any open one) and delivers it to the simulated outbox in the same transaction.
CREATE FUNCTION identity.contact_code_issue(p_issuer text,p_subject text,p_destination text,p_code text) RETURNS jsonb
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,identity AS $$
DECLARE v_open identity.contact_codes;v_recent integer;v_id uuid:=gen_random_uuid();v_now timestamptz:=clock_timestamp();BEGIN
 IF p_code !~ '^[0-9]{6}$' THEN RAISE EXCEPTION 'Invalid code format';END IF;
 PERFORM pg_advisory_xact_lock(hashtextextended('contact-code:'||p_issuer||':'||p_subject,0));
 SELECT * INTO v_open FROM identity.contact_codes WHERE issuer=p_issuer AND subject=p_subject AND consumed_at IS NULL AND superseded_at IS NULL
  ORDER BY created_at DESC LIMIT 1;
 IF FOUND AND v_open.resend_after>v_now THEN RAISE EXCEPTION 'RESEND_TOO_SOON';END IF;
 SELECT count(*) INTO v_recent FROM identity.contact_codes WHERE issuer=p_issuer AND subject=p_subject AND created_at>v_now-interval '1 hour';
 IF v_recent>=5 THEN RAISE EXCEPTION 'TOO_MANY_CODES';END IF;
 UPDATE identity.contact_codes SET superseded_at=v_now WHERE issuer=p_issuer AND subject=p_subject AND consumed_at IS NULL AND superseded_at IS NULL;
 INSERT INTO identity.contact_codes(id,issuer,subject,channel,destination,code_hash,created_at,expires_at,resend_after)
 VALUES(v_id,p_issuer,p_subject,'EMAIL',p_destination,encode(sha256(convert_to(v_id::text||':'||p_code,'UTF8')),'hex'),
  v_now,v_now+interval '10 minutes',v_now+interval '45 seconds');
 PERFORM simulator.deliver_contact_code('EMAIL',p_destination,p_code);
 RETURN identity.contact_code_status(p_issuer,p_subject);
END $$;

-- Checks a code against the current open one. A match consumes it; a miss uses one of 5 attempts.
CREATE FUNCTION identity.contact_code_check(p_issuer text,p_subject text,p_code text) RETURNS jsonb
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,identity AS $$
DECLARE v_open identity.contact_codes;BEGIN
 PERFORM pg_advisory_xact_lock(hashtextextended('contact-code:'||p_issuer||':'||p_subject,0));
 SELECT * INTO v_open FROM identity.contact_codes WHERE issuer=p_issuer AND subject=p_subject AND consumed_at IS NULL AND superseded_at IS NULL
  ORDER BY created_at DESC LIMIT 1 FOR UPDATE;
 IF NOT FOUND OR v_open.expires_at<=clock_timestamp() THEN RETURN jsonb_build_object('result','EXPIRED','attemptsLeft',0);END IF;
 IF v_open.attempts>=5 THEN RETURN jsonb_build_object('result','LOCKED','attemptsLeft',0);END IF;
 IF p_code ~ '^[0-9]{6}$' AND encode(sha256(convert_to(v_open.id::text||':'||p_code,'UTF8')),'hex')=v_open.code_hash THEN
  UPDATE identity.contact_codes SET consumed_at=clock_timestamp() WHERE id=v_open.id;
  RETURN jsonb_build_object('result','VERIFIED','destination',v_open.destination,'attemptsLeft',5-v_open.attempts);
 END IF;
 UPDATE identity.contact_codes SET attempts=attempts+1 WHERE id=v_open.id;
 RETURN jsonb_build_object('result',CASE WHEN v_open.attempts+1>=5 THEN 'LOCKED' ELSE 'MISMATCH' END,'attemptsLeft',5-(v_open.attempts+1));
END $$;
GRANT EXECUTE ON FUNCTION identity.contact_code_status(text,text),identity.contact_code_issue(text,text,text,text),
 identity.contact_code_check(text,text,text) TO bank_identity;
