# WP-24 progress — Finance GL export slice

This is the GL slice record from 2026-10-31. The subsequent [reporting/liquidity specification](../finance-reporting.md) records the additional immutable observation and independent cash capabilities introduced on 2026-11-01; remaining-work statements below describe the earlier checkpoint.

Verified locally on 2026-10-31. **WP-24 remains in progress.** This evidence covers versioned chart publication, unique balanced GL export, acknowledgment recovery, owned export exceptions and the workforce inquiry/recovery view. It does not complete period reporting, multi-source reconciliation, liquidity feeds or periodic accrual automation.

The new `bank-finance` crate receives a narrow immutable Core journal/classification read capability. Its database principal cannot read Core tables or execute Core financial commands; tests verify PostgreSQL permission-denied code `42501`. A separate FINANCE_CHART proposal kind binds the exact document, current canonical maker, independent checker, live grants and expiry. Publication rechecks after the chart lock. Immutable chart versions resolve by business date when a source is first sealed; later versions cannot remap that export. Control accounts require explicit mappings.

Every export preserves the original Core batch/operation, linked original batch, source/chart digests, chart version, exact account legs and business date. Unique source/export identities survive concurrent discovery and retries. Per-leg signed 64-bit minor units remain strings; aggregate control totals use 128-bit arithmetic and decimal strings. Tests cover values above JavaScript's safe integer limit and balanced totals above signed 64-bit range. GL export/recovery never posts Core money or charges another fee.

The separate signed GL protocol and persistent simulator deduplicate the exact original source across process restarts. A transport lease/fence commits before bounded HTTP; no database transaction spans transport. Lost booking responses recover the original authenticated receipt through inquiry. Only an authenticated exact-scope NOT_FOUND permits replay of the same sealed GL batch under this selected strict-deduplication protocol. This does not change payment-rail NOT_FOUND behavior. Wrong-scope replies are quarantined privately and create an owned Finance case. BOOKED requires a verified immutable receipt and matching control totals/reference.

Recovery requests seal the actor/client/key, exact reason, original export and generation before scheduling. They add eight attempts without resetting lifetime counters. Old generations are superseded without transport effects. A booked export with a lost scheduling acknowledgment still recovers its original operation/job. Earlier recovery requests with missing operation links are also repaired. Re-reading a sealed source completes a lost mapping-match acknowledgment, and recorded exceptions with a lost case-assignment acknowledgment acquire the original owned case even after matching. Immutable history and durable audit intents are retained.

Workforce inquiry requires current Finance adjuster/approver authority and Finance queue membership; recovery requires the adjuster role and REVIEW membership. Current revocation blocks reads and idempotency replay. Customer identities and security-administration authority alone cannot access Finance. The generated client and implemented OpenAPI add four route patterns, bringing the checked total to 80. The local seed independently publishes a stable synthetic chart through the service path.

The real Chromium Finance flow uses a dedicated disposable synthetic ledger source, preserving the later customer onboarding fixtures. It signs in through the real customer/workforce issuers, verifies trust/role denial, quarantines a conflicting GL acknowledgment, submits a reason-bound staff recovery and recovers the original BOOKED receipt. It verifies unchanged Core money, one original Core source and one provider GL batch. Accessibility and browser-error checks pass. The workforce screenshot `.local/finance-gl-workbench.png` was inspected.

| Local verification | Observed result |
| --- | --- |
| `make test-integration` on final source | 38 cases passed across nine suite binaries: 37 database-backed cases and one pure exact-total case |
| Finance coverage within that suite | Six cases passed: independent/alias/current chart authority, exact sealed export/deduplication, lost response/restart/NOT_FOUND and scheduling/case acknowledgment recovery, quarantined receipt/audited generation recovery/revocation, missing mapping repair and expired lease/retry exhaustion |
| `make test-payments` | All four real issuer/browser payment and Finance flows passed |
| Final focused Finance browser rerun after acknowledgment repairs | Passed; original batch/receipt and unchanged Core balances verified |
| `make test-ingress` | 80 actual route patterns passed trust/error, size, shared quota and forwarded-address checks |
| `make test-customer test-staff test-unit` | Customer and workforce regressions and eight library unit cases passed |
| `make app-check` on final source | Warning-free Rust checks, real OpenAPI validation, route/method/client drift checks and strict TypeScript passed |
| `make web-build check` | Both browser builds and plan/documentation/catalog checks passed; 33 PlantUML sources passed syntax validation |
| `make diagrams` | All 33 SVG views rendered and passed XML checks: 22 target and 11 implementation/historical models |
| `make migrate` and `make dev-seed` | Migration 0042 applied and stable synthetic chart published in the persistent local database |

Logs: `.local/wp24-final-database.log`, `.local/wp24-checked.log`, `.local/wp24-final-browser.log`, `.local/wp24-final-finance-browser.log`, `.local/wp24-ingress.log`, `.local/wp24-regression.log`, `.local/wp24-diagrams.log`, `.local/wp24-final-docs-build.log`, `.local/wp24-migrate.log` and `.local/wp24-seed.log`. The [execution specification](../finance-execution.md), [C4 components](../current-finance-components.puml), [UML export sequence](../current-gl-export.puml) and [recovery runbook](../../runbooks/finance-recovery.md) describe this slice.

## Remaining work before completing WP-24

1. Add consistent bounded Core reporting snapshots and narrow Payments/provider lineage ports. Match instruction, provider evidence, journal and GL by exact currency, account, amount, fee and business date; retain classified owned missing/duplicate/timing/value breaks.
2. Persist reproducible Finance report snapshots with immutable source membership, totals, freshness/completeness and drill-down. Corrections must create linked new records with a reason and independent approval, preserving the original report and journal.
3. Add authenticated synthetic liquidity evidence with explicit as-of/expiry semantics. Distinguish available, reserved, expected and externally settled funds; missing/stale evidence cannot produce confirmed available liquidity.
4. Implement periodic fee/accrual commands only where explicitly enabled by a versioned approved rule. Preserve account/rule/period uniqueness, exact arithmetic and the selected business calendar. Existing payment fees are already booked by Core and must not be charged again.
5. Complete staff reporting/liquidity views and the remaining AT-12/24/25 cases, including late feeds, period boundaries, changed rules and independently approved corrections. Record complete evidence before changing WP-24 to done.

Real GL/rail providers, statutory report formats, market calendars, cloud deployment, production capacity and regulatory qualification were not verified by these synthetic checks. Later release packages remain tracked on the [execution board](../WORK_PACKAGES.md).
