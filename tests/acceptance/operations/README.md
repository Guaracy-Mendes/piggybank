# WP-26 operations acceptance manifest

This manifest maps each scenario branch to the executable test that proves it. Database cases run under `make test-integration` against disposable PostgreSQL databases with migrations 0001–0063.

| Scenario branch | Executable evidence |
| --- | --- |
| AT-30: a seeded structured-credit pattern with duplicate and conflicting durable events gives one economically scoped alert and case | `crates/data/tests/cases/operations.rs` `duplicate_durable_events_and_replay_produce_one_economic_alert_with_independent_disposition` |
| AT-30 / FR-CTRL-04: stream tick and retrospective replay detect the scenario once (0 new, 1 existing) | same test |
| AT-30: attributable disposition — proposer cannot approve; a different approver closes the case; dispositions immutable | same test, plus the browser journey |
| Monitoring cases are not visible to the customer | same test |
| AT-13 / FR-DATA-05: missing journal projection, stale watermark and undelivered outbox open owned alerts with next actions | `seeded_data_defects_open_owned_alerts_and_repair_cannot_write_balances` |
| Repair writes only the read model; the Data role cannot update balances or insert journal entries; later event delivery is absorbed | same test |
| Broken reference and quarantined (conflicting) event defects | same test |
| FR-CTRL-05 / AT-25: fixed-snapshot operations report reconciles to underlying counts; trace by durable identifiers without identity data | same test, plus the browser journey |
| Deduplicated notifications, purpose preferences, suppression, masked recipient and redacted content | `notifications_are_deduplicated_purpose_scoped_redacted_and_retried_through_outage` |
| AT-20 (messages): channel outage fails and backs off; recovery delivers once; exhausted attempts raise an owned alert | same test |
| Clients: customer Messages and preferences, staff Operations monitor, axe WCAG 2.2 AA | `tests/browser-consumer/consumer.spec.ts` "operations monitor records a fixed snapshot, owns monitoring alerts with independent disposition and customers see safe messages" |

Not covered here, and left to its owning work:
- AT-20 provider callback key rotation and rail pause belong to WP-21 and WP-22.
- AT-25 full journey reconstruction remains a WP-30 qualification exercise.
