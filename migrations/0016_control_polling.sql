ALTER TABLE controls.check_requests ADD COLUMN next_poll_at timestamptz NOT NULL DEFAULT now();
ALTER TABLE controls.check_requests ADD COLUMN attempts integer NOT NULL DEFAULT 0;
CREATE INDEX control_poll_due ON controls.check_requests(next_poll_at) WHERE state='PENDING';
