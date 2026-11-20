# WP-25 document acceptance manifest

This manifest maps each AT-15 and AT-16 branch to the executable test that proves it. Database cases run under `make test-integration`, against disposable real PostgreSQL databases with migrations 0001–0062.

| Scenario branch | Executable evidence |
| --- | --- |
| AT-15: an open month publishes nothing; the refusal is recorded | `crates/documents/tests/cases/statements.rs` `statements_are_ready_gated_versioned_reproducible_and_retrieved_once_by_the_owner` |
| AT-15: generating twice returns one version (idempotent by snapshot) | same test |
| AT-15: exact opening + movements = closing | same test, plus the browser journey's API assertion |
| AT-15: cross-customer link refused (`NOT_AUTHORIZED`); foreign actor, reused, unknown and expired references denied and recorded | same test, plus the browser journey (HTTP 404 and 410) |
| AT-15: a linked correction creates a superseding version with identical totals; the earlier version stays retrievable | same test |
| AT-15: the next period carries the correction and opens at the prior closing | same test |
| AT-15: a backdated source fact becomes a visible `SOURCE_CHANGED` version | same test |
| AT-15: periodic finance readiness blocks a complete statement until finalization | `crates/finance/tests/cases/periodic_execution.rs` `monthly_fee_finalizes_once_posts_balanced_native_money_exports_gl_and_corrects_once` |
| AT-15: object write failure publishes no metadata; the worker cycle retries once | `failed_object_writes_publish_nothing_and_the_worker_cycle_retries_once` |
| AT-16: retention refuses early disposal; a legal hold refuses disposal; owner-path bypasses refused | `retention_legal_holds_disposal_and_data_requests_produce_traceable_outcomes` |
| AT-16: independent hold release (the placer cannot release, even holding the approver role) | same test, plus the browser journey |
| AT-16: object deletion failure leaves no disposal evidence | same test |
| AT-16: permitted disposal keeps a tombstone and evidence; retrieval and regeneration refused | same test |
| AT-16: verified deletion conflicting with retention and hold gives per-record restricted outcomes | same test, plus the browser journey (`RESTRICTED`) |
| AT-16: access export under its own policy; correction is restricted; export disposal is retention- and hold-gated | same test |
| Clients: customer and records portals, axe WCAG 2.2 AA, 390 px layout | `tests/browser-consumer/consumer.spec.ts` "monthly statements are readiness-gated, retrieved once, and held or restricted by independent records staff" |
