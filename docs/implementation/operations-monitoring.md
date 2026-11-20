# WP-26 alerts, transaction monitoring and data-quality operations

WP-26 was implemented on 2026-11-17 as migration `0063_operations_monitoring.sql` (schema 63). It covers:
- requirements FR-CTRL-04, FR-CTRL-05, FR-DATA-05, FR-OPS-02 and NFR-08 (watermark display)
- acceptance scenarios AT-13, AT-20 (monitoring branches), AT-25 and AT-30

Ownership is split between two modules:
- **Controls** owns monitoring rules, alerts, dispositions and operations snapshots.
- **Data** owns data-quality alerts, projection repairs and notifications.

Neither module can write a Core balance or journal. Every rule, threshold and template is a synthetic fixture.

## Transaction monitoring (FR-CTRL-04, AT-30)

**Source of facts.** Monitoring reads `data.transaction_views`, which the Data consumer builds from durable `journal.batch-posted` events. Duplicate deliveries of an event are absorbed by the inbox. A conflicting event identity or a version gap is quarantined, so a fact is never projected twice. Amounts and posting times come from the immutable Core journal.

**Rules.** `controls.monitoring_rules` holds immutable versioned fixtures:

| Rule | Pattern |
| --- | --- |
| `RAPID_OUTFLOW` v1 | Three or more debits of kind `INTERNAL_PAYMENT`, `EXTERNAL_PAYMENT` or `SIMULATED_CARD_PURCHASE` within 15 minutes, totaling at least 300.00 TST |
| `STRUCTURED_CREDITS` v1 | Three or more credits each between 90.00 and 99.99 TST within 24 hours |

Only accounts in the account registry are monitored.

**Stream evaluation.** The worker calls `controls.monitoring_tick(500)` every 5 seconds. It evaluates each projected fact from the last seven days exactly once per current rule, recorded in `monitoring_evaluations`.

**Economic scope.** When a pattern completes:
1. The alert's anchor is the earliest operation in the qualifying window.
2. `UNIQUE(rule, account, anchor)` stops a second alert for the same episode.
3. An alert whose window overlaps an existing alert for the same rule and account is also skipped.

Duplicate events, replays and retrospective scans therefore produce one alert per economic episode. The evidence lists every fact, with its origin event and operation, plus the rule digest.

**Retrospective investigation.** A case reviewer can re-evaluate a window of up to 31 days and 5,000 facts. Scans are recorded and audited. A scan reports evaluated, new and existing counts, and never duplicates an alert.

**Cases and disposition.**
- Each alert opens one `MONITORING` case in the `controls-operations` queue.
- Monitoring cases are never linked to the customer and are never shown to them (no tipping off).
- A controls reviewer proposes `NO_FURTHER_ACTION` or `REFER_FINANCIAL_CRIME` with a rationale. A different person with the approver role approves it, which closes the case.
- Every step writes a case action, a `case.changed` event and an audit record. Dispositions are immutable apart from the single approval.

## Data quality (FR-DATA-05)

The worker calls `data.quality_scan()` every 30 seconds. Each finding opens one `OPEN` alert per (check, subject), with a `DATA` case in the `data-operations` queue and a stated next action. When the condition no longer holds, the alert becomes `CLEARED`, and its history is append-only.

| Check | Detects | Next action |
| --- | --- | --- |
| `MISSING_FINANCIAL_EVENT` | A journal batch on a registry account that is not in the read model | Check the relay, then repair the projection |
| `QUARANTINED_EVENT` | A pending dead letter (duplicate identity conflict or version gap) | Inspect, then replay or reject |
| `BROKEN_REFERENCE` | A transaction without an account view, or an account view without a party reference | Replay the reference events |
| `STALE_PROJECTION` | Oldest unprojected fact older than the objective (fixture 30 s, NFR-08 p99) | Restore the consumer and watch the watermark |
| `UNDELIVERED_EVENTS` | Any module outbox with events older than the fixture threshold | Restore the event relay |
| `PROVIDER_BACKLOG` | A rail submission without a final outcome after 10 minutes | Inquire with the original instruction |
| `RECONCILIATION_BREAK_AGED` | An open payment reconciliation break older than 1 hour | Resolve the break |
| `NOTIFICATION_FAILED` | A message that failed all six delivery attempts | Restore the channel and review the message |

Thresholds live in `data.quality_thresholds`. Only the migration owner can change them; tests lower them in disposable databases.

**Repair.** A data steward can run `data.repair_projection(batch)`. It re-projects one journal batch from the immutable Core journal into `data.transaction_views` only. It requires the original durable `journal.batch-posted` event, is idempotent, and is recorded in `projection_repairs`. When the event is delivered later, it is absorbed without a duplicate.

Repair cannot touch money: the Data role cannot update `core.ledger_accounts` or insert journal entries, and the integration test verifies this.

A steward acknowledges a cleared alert with a note, which closes its case. An alert that is still open cannot be acknowledged.

**Freshness.** The data-quality view shows the projection watermark: the oldest unprojected fact, the lag and the status (`CURRENT`, `WITHIN_OBJECTIVE` or `STALE`). Spending decisions always use Core.

## Operations reporting and correlation (FR-CTRL-05, FR-OPS-02, AT-25)

**Snapshot.** `controls.operations_snapshot` runs inside one `REPEATABLE READ` transaction, so every count observes the same committed snapshot. It counts:
- payment intents awaiting controls, submissions awaiting an outcome, open recovery requests and open breaks
- pending and overdue checks
- open cases by queue and kind, overdue cases and escalated cases
- Finance reconciliation and periodic issues
- undelivered outbox events and pending dead letters for every module
- open monitoring alerts, open data-quality alerts and notification states

Each snapshot is stored immutably with its canonical digest, and lists up to 20 overdue cases.

**Trace.** `controls.operation_trace(id)` correlates the following by durable identifiers only (no names, emails or identity evidence):
- journal batches, legs and projection status
- the payment intent, rail instruction and receipts
- outbox events from every module (by aggregate, correlation or `operationId`)
- cases, monitoring alerts and notifications

## Notifications

**Purposes.** `data.notification_purposes` defines four:

| Purpose | Behavior |
| --- | --- |
| `SECURITY` | Mandatory |
| `SERVICE` | Mandatory |
| `ACTIVITY` | Optional, on by default |
| `MARKETING` | Optional, off by default |

Customers change optional purposes; every change is recorded in a history table.

**Derivation.** `data.derive_notifications` queues one message per economic or document identity, using a unique `dedupe_key`:
- statement version published (SERVICE)
- data request processed (SERVICE)
- an outgoing payment projected from a durable event (ACTIVITY)
- a developer token created (SECURITY)

Replays and duplicate events send nothing twice. A disabled optional purpose produces a `SUPPRESSED` record with its reason, and is never delivered.

**Safe content.** Fixed templates contain no amounts, balances, full identifiers, tokens, attachments or links carrying secrets. The account is identified only by its last four characters, and the email recipient is masked.

**Synthetic capture.** Delivery is recorded in `notification_attempts` and never sent. The channel is `SYNTHETIC_EMAIL` when a verified contact exists, otherwise `IN_APP`.

**Outage handling.** An operator with `operations_replay` can mark a channel unavailable, with a reason, to run an outage drill (AT-20). Each failed attempt backs off for 30 s × 2ⁿ. After six attempts the message is `FAILED` and raises a `NOTIFICATION_FAILED` data-quality alert. A delivered message is final.

Customers see their own messages under **Notifications → Messages**, and staff see counts and recent attempts.

## API

| Method and path | Actor |
| --- | --- |
| `GET /api/v1/staff/monitoring/alerts` | Case reviewer or approver |
| `POST /api/v1/staff/monitoring/scans` | Case reviewer |
| `POST /api/v1/staff/monitoring/alerts/{id}/disposition` | Case reviewer in the controls queue |
| `POST /api/v1/staff/monitoring/alerts/{id}/approval` | A different case approver in the controls queue |
| `GET /api/v1/staff/operations-report` | Operations roles |
| `GET /api/v1/staff/traces/{id}` | Operations roles |
| `GET /api/v1/staff/data-quality` | Data steward, case or operations roles |
| `POST /api/v1/staff/data-quality/repairs` | Data steward |
| `POST /api/v1/staff/data-quality/alerts/{id}/acknowledgement` | Data steward |
| `GET /api/v1/staff/notifications` | Operations, data steward or case roles |
| `POST /api/v1/staff/notifications/channels` | `operations_replay` |
| `GET /api/v1/notifications` | Customer |
| `POST /api/v1/notifications/preferences` | Customer |

`/api/v1/staff/me` adds the capabilities `operations.view`, `data.steward` and `events.replay`. The local seed grants `data_steward` to `reviewer`. `approver` holds `case_approver`, and `security` holds `operations_replay`.

## Models

| View | Source |
| --- | --- |
| C4 components | [current-operations-components.puml](current-operations-components.puml) |
| Sequence | [current-operations-monitoring.puml](current-operations-monitoring.puml) |
| Domain | [current-operations-domain.puml](current-operations-domain.puml) |

## Deliberate boundaries

- The two monitoring rules are fixtures. Real typologies, tuning, sanctions screening of transactions, regulatory reporting (SAR filing) and model validation require DEC-06 policy and remain later work.
- Delivery is synthetic capture only. Real email, SMS or push providers, templates, translations and consent law are DEC-10 and WP-31 inputs.
- Thresholds are fixtures. Measured NFR-08 p95 and p99 lag under load is WP-30 qualification.
- No dashboarding stack (metrics, tracing, alert routing) is added here; that is WP-29.
