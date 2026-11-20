# WP-29 deployment, CI, observability and operations tooling

WP-29 was implemented on 2026-11-17 as migration `0066_operations_paging.sql` (schema 66). It covers:
- requirements SEC-04, SEC-09, NFR-10 and NFR-12
- acceptance scenarios AT-17, AT-20, AT-25, AT-28 and AT-29, for their local branches

## Images and runtime configuration (step 1)

**Images.** Reproducible images already existed from the cloud packaging (`infra/cloud/shared/Dockerfile`):
- multi-stage builds with locked Rust, Node and image versions (`infra/versions.json`)
- a non-root runtime (uid 10001)
- `read_only` Compose services and host-bound networking with no published ports
- sealed, checksummed release archives verified by `make cloud-check` and the archived runtime acceptance

**Fail-closed configuration.** Any `BANK_PROFILE` other than `local` refuses to start the API or the worker. WP-29 adds startup validation of secret files:
- `BANK_SESSION_KEYS_FILE` must parse as a key ring.
- `BANK_IDENTITY_ADMIN_SECRET_FILE` (16+ characters) and `BANK_METRICS_TOKEN_FILE` (32+ characters) must be readable and not readable by group or others.

Values are never logged. `tests/integration/deployment/config_failure.py` proves each refusal.

**Database pools.** `BANK_DB_POOL_MAX` (1–32, default 5) sets the per-module pool ceiling. The sum across processes and modules must stay within the PostgreSQL `max_connections` budget. Pools are lazy, so idle modules hold no connections. The WP-30 load qualification used 10, with a peak of 88 of 100 backends.

**Upgrade-compatible migrations.** `contracts/migrations.lock.json` holds a SHA-384 for every released migration. `scripts/migrations.py check` (`make migrations-check`) fails if a released file changes or numbering has a gap. It also fails if a new migration contains destructive DDL: a table or column drop, a rename, a type change, a nullability tightening, a truncate or a function drop. Rolling upgrades are expand → deploy → contract. A contract step is a later migration whose statement lines carry `-- contract:` with a reason, applied only after every N-1 binary has gone. Run `make migrations-lock` once a migration is applied to a shared environment.

## Local CI-equivalent pipeline (step 2)

`make ci` (`scripts/ci.py`) runs the gated stages in order and stops at the first failure:

| Stage | Contents |
| --- | --- |
| Static | fmt, Clippy, contract and client drift, web types, builds, UI sources, docs, plan and models, migration lock, acceptance manifest (WP-30), mobile types |
| Disposable database | Integration, unit, API binary and build |
| Isolated runtime | Configuration failure, operations drills, native contract, issuer identity, security acceptance, resilience drills (WP-30), consumer browser |
| Optional `--apply-local` | Migrate the persistent database, then the port-bound browser suites and ingress |
| Optional `--load` | Release build and load smoke (WP-30). Fails only on broken financial invariants; NFR checks are recorded in the report. |
| Optional `--device` | Android emulator journey |

Each run writes `.local/ci/<UTC>/evidence.json` (commands, exit codes, durations, tool versions, revision and uncommitted path count; no environment values or secrets) plus per-step logs.

**No remote pipeline is configured**, because no CI host has been chosen (DEC-05). A pipeline definition would not be evidence of a remote run.

## Metrics, traces, alerts and on-call (step 3)

**Metrics.** `GET /metrics` serves Prometheus text when `BANK_METRICS_TOKEN_FILE` is set; otherwise it returns 404. It requires that bearer token, compared in constant time. It exposes:
- `bank_http_requests_total` and the `bank_http_request_duration_seconds` histogram, by method, route template (identifiers collapsed) and status class, recorded in the ingress boundary
- gauges from `controls.metrics_snapshot()`:
  - undelivered outbox events and oldest age, and pending dead letters, per module
  - awaiting payments, and open and overdue cases
  - open quality and monitoring alerts, pending and failed notifications
  - projection lag, unacknowledged and undelivered pages, oldest unacknowledged page age
  - journal batch count and **unbalanced journal batches** (an NFR-13 invariant that should be 0)

**Traces.** The WP-26 durable-identifier trace and the `x-request-id` header on every response and log line.

**Paging (NFR-10).** Sources are high-severity data-quality alerts, open monitoring alerts and escalated overdue cases. The worker's 5-second `controls.page_tick()` opens one page per source and delivers it by synthetic pager capture to the active primary on-call. If nobody acknowledges within the policy window (fixture: 300 s, so a 5-minute acknowledgement target), it escalates once to the secondary. Only a person who was paged can acknowledge; acknowledgement is audited and page history is append-only. The roster is changed by `operations_replay` or `security_admin` and every change is recorded. The local seed makes `reviewer` the primary and `approver` the secondary. The staff **Operations monitor** shows the roster, pages, delivery latency and **Acknowledge page**.

**Drill.** `tests/integration/deployment/operations.py` injects a projection gap. The worker scan and page tick delivered the page to the primary within the 60-second objective (measured value in the evidence), and the primary acknowledged it.

## Recovery and interfaces (step 4)

| Topic | Location |
| --- | --- |
| Backup and restore | [Backup and restore runbook](../runbooks/backup-restore.md). The drill restores a `pg_dump` into a new database, verifies migration checksums, balanced journals, booked balances equal to journal sums, held amounts equal to active holds, payment, audit and chain-head equality, and serves the same balance from an API on the restored copy before any write. |
| Rolling upgrade, rollback and forward recovery | [Deployment and upgrades](../runbooks/deployment-upgrades.md) |
| Rail pause | Existing WP-21 control. Pausing keeps processing of existing acknowledgements, returns and reconciliation. |
| On-call | [On-call runbook](../runbooks/oncall.md) |
| Secrets | File-based secrets with startup validation: session key ring, issuer admin secret, metrics token. Production custody (KMS or HSM) is WP-31. |

## Models

| View | Source |
| --- | --- |
| Operations tooling | [current-operations-tooling.puml](current-operations-tooling.puml) |

## Deliberate boundaries

- **Remote CI.** None configured; there is no chosen host. Remote results will be recorded separately when one exists.
- **Deployed infrastructure.** Production network policies, metrics scraping and storage, alert routing to a real pager, and staffed on-call are DEC-05, DEC-12 and WP-31 inputs.
- **Recovery measurement.** The backup drill measures local duration only. Production RPO and RTO, failover with fenced writers, and regional restore are WP-30 qualification.
