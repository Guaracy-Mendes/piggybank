CREATE TABLE journeys.applications(
 id uuid PRIMARY KEY,issuer text NOT NULL,subject text NOT NULL,create_key text NOT NULL,create_digest text NOT NULL,
 product_version_id uuid NOT NULL,terms_id uuid NOT NULL,acceptance_id uuid,
 profile jsonb,evidence_id uuid,consent_version text,marketing boolean NOT NULL DEFAULT false,attribution text,
 state text NOT NULL DEFAULT 'DRAFT' CHECK(state IN ('DRAFT','SUBMITTING','CONTROLS_PENDING','MANUAL_REVIEW','INFORMATION_REQUIRED','DECLINED','ACTIVE','EXPIRED')),
 next_action text NOT NULL DEFAULT 'COMPLETE_APPLICATION',version bigint NOT NULL DEFAULT 1,generation integer NOT NULL DEFAULT 0,
 account_id uuid,party_id uuid,created_at timestamptz NOT NULL DEFAULT now(),updated_at timestamptz NOT NULL DEFAULT now(),expires_at timestamptz NOT NULL DEFAULT now()+interval '7 days',
 UNIQUE(issuer,subject,create_key),CHECK(marketing OR attribution IS NULL)
);
CREATE TABLE journeys.submissions(
 application_id uuid NOT NULL REFERENCES journeys.applications(id),generation integer NOT NULL,key text NOT NULL,
 digest text NOT NULL,actor_context jsonb NOT NULL,snapshot jsonb NOT NULL,operation_id uuid,
 created_at timestamptz NOT NULL DEFAULT now(),PRIMARY KEY(application_id,generation),UNIQUE(application_id,key),UNIQUE(operation_id)
);
CREATE TABLE journeys.history(id uuid PRIMARY KEY,application_id uuid NOT NULL REFERENCES journeys.applications(id),version bigint NOT NULL,state text NOT NULL,reason_code text NOT NULL,occurred_at timestamptz NOT NULL DEFAULT now(),UNIQUE(application_id,version));
CREATE FUNCTION journeys.immutable_history() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'Immutable journey history'; END $$;
CREATE TRIGGER immutable_journey_history BEFORE UPDATE OR DELETE OR TRUNCATE ON journeys.history FOR EACH STATEMENT EXECUTE FUNCTION journeys.immutable_history();
