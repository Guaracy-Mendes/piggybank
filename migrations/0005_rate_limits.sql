-- owner: platform. Shared atomic ingress budgets across API instances.
CREATE TABLE platform.rate_limits (
  scope text NOT NULL,
  window_start timestamptz NOT NULL,
  requests integer NOT NULL CHECK(requests>0),
  PRIMARY KEY(scope,window_start)
);
