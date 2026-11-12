ALTER TABLE controls.check_requests ADD COLUMN owner_queue text NOT NULL DEFAULT 'controls-operations';
ALTER TABLE controls.check_requests ADD COLUMN next_action text NOT NULL DEFAULT 'INQUIRE_OR_REVIEW';
