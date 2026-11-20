# Finance export execution — current synthetic slice

Current status: essential synthetic WP-24 is complete. This document retains the narrower increment it introduced; the [financial lifecycle](periodic-finance-r1.md), [scope decision](wp24-essential-scope.md) and [completion evidence](evidence/WP-24-complete.md) describe the completed execution and deliberate deferrals. Earlier references to remaining work below describe that checkpoint.

WP-24 was in progress when this increment was recorded. The GL slice implements independently approved charts, balanced export, receipt recovery, owned exceptions and workforce inquiry. Subsequent [reporting/liquidity](finance-reporting.md) and [controlled revision](finance-revisions.md) increments add immutable observations, independent synthetic cash evidence and approved replacement links. The [outgoing matching increment](finance-reconciliation.md) adds independent clearing-file, instruction, Core and journal/GL comparisons. The [incoming/return increment](finance-credit-reconciliation.md) adds independent credit files, native suspense/allocation and lifetime original-return controls. [Closed grouped clearing snapshots](finance-group-reconciliation.md) support multiple bounded files with exact row lineage. Periodic posting and its financial corrections remain within WP-24. The [funding correction increment](finance-corrections.md) provides a separate narrow Core command for independently approved exact synthetic funding reversals; the financial read port remains read-only. The existing payment reconciler continues to own payment/hold/finality inconsistencies. These local increments do not establish production Finance readiness.

## Source and mapping authority

Core exposes a narrow `FinanceRecords` capability: paged immutable journal IDs and a complete balanced journal with authoritative account classifications. Finance receives no reserve, release or posting method through that capability. Its PostgreSQL principal cannot read or write Core tables or execute financial commands. All currently enabled products require the explicit `synthetic-chart-v1` code.

A chart contains an immutable ID, code/version, effective half-open date interval, account classifications, account-kind mappings and explicit account overrides. Customer accounts map to liabilities and settlement accounts to assets. Control accounts require explicit account mappings; the local fixture maps payment fees to revenue and incoming suspense to a liability. The independently approved local chart has a stable fixture ID. Publication rechecks the current maker/checker grants, canonical human independence, exact document digest, proposal kind and expiry. Chart approval cannot be used as a Core correction approval.

When first staging a source, Finance resolves the highest published chart version effective on the source business date. The resulting export seals the complete mapped legs, chart ID/version/digest, source digest, Core operation, business date and linked original batch. A unique source-batch constraint serializes concurrent event/file/discovery deliveries. Later chart versions never remap an already sealed export. Changes to historical effective periods are independently approved configuration, not edits to an existing export.

Every leg retains its Core account ID and exact signed TST amount. Negative deltas are debits; positive deltas are credits. Zero, duplicate-account, foreign-currency, unbalanced or oversized batches are rejected. Per-leg amounts use the existing signed 64-bit minor-unit string contract. Aggregate debit/credit totals use signed 128-bit arithmetic and canonical decimal strings, so a balanced multi-leg total can exceed the per-leg limit without floating point or overflow.

## Durable transport and acknowledgment

Discovery wraps a persistent UUID cursor in pages of 50. A missing chart or mapping creates an owned Finance exception; the source remains discoverable after publication. Export staging commits before Platform scheduling. Scheduling uses a stable export ID and immutable payload, recovering the original operation/job after a lost scheduling acknowledgment. A second cursor ensures unresolved old sources cannot permanently starve later exports.

Recovery also visits booked exports whose scheduling link was lost, links earlier recovery generations to their original jobs, and assigns any recorded exception whose case acknowledgment was lost. Re-reading an existing sealed source completes a pending mapping-match acknowledgment. A match does not erase the exception or leave its ownership permanently unassigned.

The first transport attempt books the sealed batch. Later attempts inquire using that same batch. A six second lease and monotonic fence are committed before HTTP; no database transaction spans transport. HTTP is restricted to the loopback synthetic provider, rejects redirects and replies over 64 KiB, and has a 1.5 second deadline. GL uses a separate HMAC key from payment rails and control decisions.

The simulator stores the original source ID, export ID, exact request fingerprint and immutable booking receipt before returning. The source ID and export ID are unique across process restarts. Changed content or a replacement export ID for the same source conflicts. A lost booking response therefore recovers the original provider journal reference and receipt through inquiry.

For this selected GL protocol, an authenticated exact-scope `NOT_FOUND` permits replay of the **same sealed batch**. Finance first records the signed observation under the current fence. Provider strict source deduplication prevents a late request or concurrent retry from booking again. This permission is specific to the synthetic GL protocol; payment-rail `NOT_FOUND` continues to retain submitted funds and does not authorize resubmission or release.

`BOOKED` requires a signed, scope-checked receipt for the original export/source/digest, business date, exact control totals and stable `SIMGL-{source UUID without hyphens}` reference. Receipt persistence and the BOOKED transition share one Finance transaction. Readback verifies archived receipt integrity without treating an older valid receipt as a new live message. A stale transport fence cannot replace a receipt. Successful GL export posts no Core money and charges no additional fee.

## Exceptions and operator recovery

Malformed, unauthenticated, wrong-scope or conflicting acknowledgments cannot establish booking. Bounded packet observations are immutable and private to the service; workforce responses contain safe status/history and case links, without raw HMAC envelopes. Exhausted transport/job budgets create a stable owned `finance-operations` case. Chart/mapping gaps and unknown GL outcomes are visible separately from Core booking status.

Finance inquiry requires current `finance_adjuster` or `finance_approver` authority and matching REVIEW/APPROVE Finance queue membership. Recovery additionally requires the adjuster role and REVIEW membership. BFF mutations enforce workforce trust, Origin and CSRF checks. A recovery request seals the requesting issuer/subject/client, UUID idempotency key, export, exact reason and generation before scheduling. Reusing a key with changed input fails; revocation blocks even a key replay.

Each request adds a new bounded allowance of eight transport attempts; lifetime counters never decrease. Older job generations become `SUPERSEDED` with no transport effect. The recovery job has its own stable Platform identity and continues inquiry of the original export. It cannot create a new GL reference, remap the source, change the journal or mark an unauthenticated outcome booked. Matching retains the owned Controls case and immutable audit/history.

## Implemented interfaces

| Interface | Purpose |
| --- | --- |
| `GET /api/v1/staff/finance/exports` | Current authorized list, most recent 100 |
| `GET /api/v1/staff/finance/exports/{id}` | Sealed mapping, exact totals, safe receipt, retained history, recovery requests and case links |
| `POST /api/v1/staff/finance/exports/{id}/recovery-requests` | Durable original-export recovery with `{reason}` and UUID `Idempotency-Key`; `202` means accepted |
| `GET /api/v1/staff/finance/breaks` | Most recent 100 owned export/mapping exceptions |
| Simulator `POST /gl/batches`, `POST /gl/inquiry` | Separate signed synthetic GL protocol |
| Simulator `POST /admin/gl-scenarios` | Authenticated local fault injection: ACCEPT, LOST_RESPONSE, BEFORE_ACCEPT_TIMEOUT, CONFLICT |

See the [C4 components](current-finance-components.puml), [UML sequence](current-gl-export.puml), [recovery runbook](../runbooks/finance-recovery.md) and [execution board](WORK_PACKAGES.md).

The [periodic policy increment](periodic-accounting-policies.md) adds independently approved Core configuration and exact previews through a narrow registry; financial posting jobs remain unfinished.

Customer payment detail now preserves monotonic instruction versions when status requests complete out of order. An older DRAFT response cannot replace accepted/settled status and disable polling. The browser regression deliberately releases a saved authentic DRAFT response after confirmation commits and checks the same native journal/operation and balances.
