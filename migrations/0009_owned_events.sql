-- owner: each module. Identical transport tables retain independent privileges/transactions.
DO $$ DECLARE s text; BEGIN
FOREACH s IN ARRAY ARRAY['platform','identity','core','controls','journeys','payments','finance','documents','data','simulator'] LOOP
  EXECUTE format('CREATE TABLE %I.outbox(event_id uuid PRIMARY KEY,aggregate_id uuid NOT NULL,aggregate_version bigint NOT NULL CHECK(aggregate_version>0),document jsonb NOT NULL,fingerprint text NOT NULL,lease_id uuid,lease_until timestamptz,delivered_at timestamptz,created_at timestamptz NOT NULL DEFAULT now(),UNIQUE(aggregate_id,aggregate_version),CHECK(document->>''producer''=%L))',s,s);
  EXECUTE format('CREATE TABLE %I.inbox(consumer text NOT NULL,event_id uuid NOT NULL,fingerprint text NOT NULL,received_at timestamptz NOT NULL DEFAULT now(),PRIMARY KEY(consumer,event_id))',s);
  EXECUTE format('CREATE TABLE %I.consumer_versions(consumer text NOT NULL,producer text NOT NULL,aggregate_id uuid NOT NULL,version bigint NOT NULL DEFAULT 0,PRIMARY KEY(consumer,producer,aggregate_id))',s);
  EXECUTE format('CREATE TABLE %I.dead_letters(consumer text NOT NULL,event_id uuid NOT NULL,reason text NOT NULL,document jsonb NOT NULL,state text NOT NULL DEFAULT ''PENDING'',created_at timestamptz NOT NULL DEFAULT now(),PRIMARY KEY(consumer,event_id))',s);
  EXECUTE format('CREATE TABLE %I.replay_audit(id uuid PRIMARY KEY,consumer text NOT NULL,event_id uuid NOT NULL,actor_issuer text NOT NULL,actor_subject text NOT NULL,state text NOT NULL,created_at timestamptz NOT NULL DEFAULT now())',s);
  EXECUTE format('CREATE TABLE %I.invalid_deliveries(consumer text NOT NULL,digest text NOT NULL,payload bytea NOT NULL,received_at timestamptz NOT NULL DEFAULT now(),PRIMARY KEY(consumer,digest))',s);
END LOOP; END $$;
CREATE TABLE data.operation_views(operation_id uuid PRIMARY KEY,version bigint NOT NULL,state text NOT NULL,kind text NOT NULL,updated_at timestamptz NOT NULL DEFAULT now());
