# Resilience drills (WP-30)

`tests/resilience/drills.py` starts its own disposable database and runs the API, worker and provider simulator as
separate processes on ports 4180–4182, so each process can be killed on its own. NATS and Keycloak are paused and
resumed through podman, and are always resumed in `finally`.

```sh
cargo build -p bank-api -p bank-worker -p provider-simulator
make test-resilience
```

| Drill | Fault | What must hold |
| --- | --- | --- |
| D1 | Worker SIGKILL while Core keeps committing | Events wait in the outbox. After restart they publish and are projected exactly once. |
| D2 | API SIGKILL during 20 concurrent payment commands | Every acknowledged command is still queryable. Same-key retries return the same payment. At most one intent exists per quote. |
| D3 | NATS paused | Core commits stay durable and the outbox backlog drains after resume. |
| D4 | Keycloak paused | The API stays live. Protected calls fail closed with the standard error envelope only. Sign-in recovers. |
| D5 | Rail provider SIGKILL before an approved payment is submitted | The payment never reports SETTLED while the provider is down. After restart, recovery settles it once. |

The ledger invariants are asserted after every drill:
- every journal batch balances;
- booked balances equal the journal sums;
- no economic posting is duplicated.

Related drills live elsewhere:
- Backup and restore: `tests/integration/deployment/operations.py`.
- Lost responses and timeouts from slow or unresponsive providers, using the simulator's `LOST_RESPONSE` and
  `BEFORE_ACCEPT_TIMEOUT` modes: the payments, controls and finance database suites.

**Scope.** These are local, single-host drills that kill processes and pause containers. They are not multi-zone or
regional failover, and they do not verify NFR-06 (RTO/RPO) or NFR-07 (zone loss). Those objectives need the
production topology in WP-31/WP-32.
