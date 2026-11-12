# Outgoing payment reconciliation

This WP-24 increment matches original internal and SIMULATED outgoing instructions, independently authenticated clearing files, Core accounting receipts, frozen journal legs and authenticated GL bookings. It preserves the existing V1 journal/report digests. Finance has read-only Core and Payments ports; reconciliation creates observations, audit intents and owned Finance cases without posting money, changing holds, sending rail instructions or replaying GL exports.

The [operator runbook](../runbooks/finance-reconciliation.md) covers reproducible local use. See the [C4 components](current-finance-components.puml), [matching sequence](current-finance-reconciliation.puml) and [provider contract](../../contracts/providers/finance/README.md).

## Authority and source ownership

Creation/replay requires a current Finance adjuster role and Finance REVIEW queue membership. Inquiry requires the existing Finance inquiry role/queue rules; an independent Finance approver can read. Security administration alone supplies no Finance access. Workforce BFF, CSRF, origin checks and ingress quotas apply to every new route. The service rechecks current write authority after source capture and inside its commit boundary. An actor/client/key binds exactly one parent report and file ID; a changed scope returns 409, including after a prior successful capture.

Core's new `payment_accounting` capability reads native payment budgets/executions, external finality and journal links under the Core principal in a bounded repeatable-read transaction. `bank-payments::finance::FinanceReader` reads confirmed instructions, immutable quotes/transport identities and archived final receipts under the Payments principal, also in a bounded repeatable-read transaction. Neither port exposes commands. Finance never queries another module's tables with its own SQL principal. The co-located runtime shares existing bounded module pools.

The signed file inquiry uses the existing loopback Finance provider base, 1.5-second client deadline, no redirects, a nonce-bound query and an independently verified signed response. Files are immutable in the provider's own store. The request kind distinguishes this protocol from GL booking. A structurally valid, authentic file can contain duplicate rows or incorrect currency/value/fee/date claims; these are matched as differences. Invalid authentication, binding, envelope or bounds produce dependency-unavailable and save no reconciliation. This path does not treat an unavailable file as an empty file.

## Artifact and temporal semantics

Input is `{reportId, fileId}`. The file period must equal the selected immutable report's half-open UTC period. The service verifies the parent report and its paged GL receipt archives, captures the file, discovers Core identities, captures Payments, then confirms native Core receipts again before committing its own artifact. The confirmation includes explicit file/workflow identities, including records with dates outside the selected period. It retains each source clock explicitly; there is no global atomic snapshot claim. Provider `generatedAt` is the file's declared cutoff, distinct from the bank's observation time.

The run seals:

- Parent report ID/digest and original journal/GL membership.
- File ID, complete file digest and private authenticated response packet.
- Separate Core and Payments source snapshots/digests, including private archived rail packets.
- Stable UUID-sorted payment rows, ordinal membership, exact controls and classification codes.
- Per-source as-of times and file cutoff, artifact format/scope and final digest.

Native per-leg and clearing-row amounts are canonical decimal i64 strings. Aggregates use i128 strings. Provider controls are partitioned by currency code **and scale**; mismatched currency rows are never added to TST controls. Duplicates remain visible in file totals. The customer bank fee comes from the original quote and is matched independently against Core and journal legs. The explicit `SYNTHETIC_PROVIDER_ZERO_FEE_V1` policy expects zero provider clearing fee; a nonzero provider fee creates an exception and does not become a second customer fee.

A period supports at most 1,000 records per bank source/union and 1,000 captured journal batches, with bounded final-receipt capture. A synthetic file supports at most 100 rows and 60,000 serialized bytes; the signed response stays within 64 KiB. Oversized sources fail with 422 rather than publishing truncated complete results. Use narrower periods for this implemented single-file workflow.

## Matching and classification

Every candidate operation from bank instructions, accounting, payment journal membership or file rows remains in the union, including provider orphans and missing bank instructions. Matching checks quote identity/digest, recipient binding, transport digest, authenticated finality, final evidence identity/version, principal, currency/scale, product version, requested date, payer/payee, separately posted bank fee, actual journal business date, exact balanced legs and captured GL status/mapping/receipt.

| Classification | Meaning | Ownership |
| --- | --- | --- |
| `MATCHED` | Original instruction, appropriate provider evidence/file, Core receipt/journal and captured GL agree | No new discrepancy case |
| `NOT_POSTED` | Terminal unbooked instruction has no journal or contradictory provider claim | No financial effect asserted |
| `PENDING` | A confirmed payment has not reached a financial finality | Partial; owned after five minutes from original creation |
| `TIMING` | Only explicit snapshot/cutoff/date/acknowledgment/GL timing differences remain | Cases for durable timing observations; short acknowledgment/GL grace remains unowned |
| `EXCEPTION` | Missing, duplicate, scope, account, fee, principal, currency or evidence conflict | Owned immutable issue/case |

Representative codes include `MISSING_PAYMENT_INSTRUCTION`, `MISSING_CORE_PAYMENT`, `MISSING_CORE_JOURNAL`, `MISSING_PROVIDER_SETTLEMENT`, `DUPLICATE_PROVIDER_LINE`, `DUPLICATE_CORE_JOURNAL`, `PROVIDER_CURRENCY_MISMATCH`, `PROVIDER_PRINCIPAL_MISMATCH`, `PROVIDER_FEE_UNEXPECTED`, `CORE_BANK_FEE_MISMATCH`, `CORE_ACCOUNT_MISMATCH`, `CORE_JOURNAL_LEGS_MISMATCH` and `PROVIDER_SETTLEMENT_EVIDENCE_MISMATCH`. Original parent-report GL difference codes remain applicable. A valid current journal absent from the parent snapshot is a visibility/timing difference even when its transaction timestamp predates that snapshot. File cutoff comparisons use authenticated provider event time, separately from bank processing time. Valid delayed outcomes use `CORE_AFTER_REPORT`, `SETTLED_AFTER_FILE`, `SETTLED_OUTSIDE_REPORT_PERIOD`, `SOURCE_SNAPSHOT_TIMING` or `PAYMENT_ACK_TIMING`. No missing financial fact is silently replaced with a workflow success flag.

Completeness is `EXCEPTIONS` when a row conflicts, `PARTIAL` when pending/timing rows remain, otherwise `COMPLETE` for **OUTGOING_PAYMENTS_V1** only. This does not certify liquidity, incoming movements, statements or the whole parent report. An empty outgoing period is explicitly complete with zero records. The first/next item pages retain the same artifact digest.

## Replay, ownership and recovery

Run metadata, source archives and items reject update/delete/truncate. Issues may acquire `case_id` once, preserving their original identity and code. Summary/item/private-source hashes and archived file authentication are checked on read. Replay returns the original artifact without refreshing evidence or correcting its original classifications. A later good file and fresh report create a new run; prior observations remain inspectable.

Creation commits immutable observations and the audit intent together. Case assignment uses a stable Controls origin per issue, so a lost acknowledgment returns the same case. The worker's bounded wrapping cursor attempts fifty pending assignments per pass and advances past failures; a bad scope cannot starve later issues. Pending ownership is visible, with a safe diagnostic code. Closed/dispositioned cases do not rewrite the captured report/run.

## HTTP and staff workflow

| Method | Route | Behavior |
| --- | --- | --- |
| GET / POST | `/api/v1/staff/finance/reconciliations` | Most recent 100 summaries, optionally filtered by `reportId`; capture with UUID `Idempotency-Key` |
| GET | `/api/v1/staff/finance/reconciliations/{id}` | Verified summary/digest and case-assignment wrapper |
| GET | `/api/v1/staff/finance/reconciliations/{id}/items` | Stable `after` ordinal, `limit` 1–100 |

The staff report view accepts a published file UUID, preserves an uncertain request key in session storage, displays separate controls/clocks and links payment, GL and case lineage. A URL ending in `/finance/reports/{reportId}/reconciliations/{runId}` reopens the saved run. Read-only approvers do not receive capture controls. Provider HMAC packets remain private; the view exposes safe identities, exact amounts and matching reasons.

## Remaining WP-24 scope

This increment covers original internal/SIMULATED outgoing payments. [Incoming/return-file reconciliation](finance-credit-reconciliation.md) is implemented separately. [Closed multi-file groups](finance-group-reconciliation.md) are also implemented within explicit source bounds. Versioned periodic fee/accrual commands, their calendar/cutoff behavior and independent linked financial corrections remain open. Existing bank payment fees are already posted by Core and must not be charged again. Real provider files, operating policies and production qualification remain separate gates.
