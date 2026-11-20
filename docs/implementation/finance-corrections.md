# Controlled synthetic funding corrections

Current status: essential synthetic WP-24 is complete. This document retains the narrower increment it introduced; the [financial lifecycle](periodic-finance-r1.md), [scope decision](wp24-essential-scope.md) and [completion evidence](evidence/WP-24-complete.md) describe the completed execution and deliberate deferrals. Earlier references to remaining work below describe that checkpoint.

Date: 2026-11-10. WP-24 was **in progress at this checkpoint**. This increment adds an independently approved **financial command** for full reversal of original synthetic funding journals. It does not enable periodic customer charges, change payment settlement or reverse incoming/return movements.

## Ownership and financial boundary

Core remains the sole money authority. [CorrectionCommands](../../crates/core/src/corrections.rs) is a narrow capability separate from the existing read-only `FinanceRecords` port and preview-only `PolicyRegistry`. Finance coordinates an immutable request, exact approval and explicit execution; Core owns the original source, ordered account locks, native posting, one-source correction uniqueness and immutable receipt. Reporting/reconciliation continue to observe financial facts without posting corrections.

The source must be a `SYNTHETIC_FUNDING` journal with no original link, exactly one positive CUSTOMER leg and one negative SETTLEMENT leg, balanced in TST scale 2. The command fully reverses those exact legs. Payment, external settlement, incoming, return, other correction, periodic fee/accrual and arbitrary journal sources are rejected. Partial reversal is unsupported. The runtime profile is local synthetic evaluation; no real currency or provider refund is implied.

Migration [0050](../../migrations/0050_finance_funding_corrections.sql) adds separate immutable Core receipts, Finance requests/observations/cursor and a guarded native command. Applied migrations 0001–0049 remain unchanged. Existing source journals, FinancialBatch/ReportSummary V1 serializers and earlier reconciliation snapshots remain immutable.

## Exact request and independent approval

1. A current Finance maker with REVIEW queue membership submits original Core journal ID, exact reason and an actor/client-scoped idempotency UUID. Finance reads the original source through Core and freezes its complete FinancialBatch V1, native journal fingerprint and source digest.
2. The immutable details bind format/kind/label, original batch and operation, original business date, currency, original/reversing account legs, exact reason and a server-generated execution UUID. This UUID is also the proposal/request identity. Money uses canonical signed-i64 decimal strings with checked negation; no floating point carries money.
3. Finance commits intent/source/details/digest/expiry/audit before Controls registration. A wrapping fifty-request cursor retries registration past failed/revoked makers using the same original proposal UUID. Recovery never approves or executes financial commands.
4. `FINANCE_CORRECTION` is a separate Controls purpose. A current Finance approver with APPROVE queue membership reviews the complete frozen source, both reversing legs, reason, digest and expiry. A login alias for the maker's canonical person cannot act as checker. The fixed approval lifetime is 24 hours.
5. The original canonical maker explicitly executes the approved scope. Current Finance authority, exact digest and original person are rechecked even on replay. A different reason/key cannot repurpose an accepted command.

The original source digest and native fingerprint are distinct: the former binds the existing typed FinancialBatch V1 read model; the latter binds Core's native immutable journal posting identity. Both remain visible for review.

## Native financial execution

Core serializes on the original journal, then locks its accounts in ascending UUID order. After **both** source and financial-account waits, the native guarded function checks the complete canonical command SHA256, approval purpose/resource/digest, exact independent people, live maker/checker grants and queue relationships, and current approval expiry. Transaction-start time cannot retain authority that expired during a lock wait.

The native source/operation/date/fingerprint and exact original/reversing legs must still match. An existing correction of that original, including an earlier legacy Core correction, blocks a second financial reversal. The existing native one-correction-per-original constraints remain in force across competing approvals.

Posting uses Core's balanced journal primitive with active-account, debit/credit restriction and available-funds checks. Customer availability includes held funds. The command neither consumes nor releases a hold. Insufficient available funds, spent funding, frozen/closed/restricted accounts or an ineligible source leave money and the original journal unchanged. A transaction spanning UTC midnight is rolled back for an original-reference retry in the current UTC date, keeping the primitive's business date consistent.

The new linked `CORRECTION` journal, native receipt and Core outbox event commit atomically. The original source is preserved. Existing discovery delivers the new journal through the normal sealed GL pipeline with original-source lineage; GL delivery status is separate from native financial completion.

## Lost responses and read integrity

Finance commits an execution-request observation before crossing Core. Afterward it independently inquires by the original execution UUID and exact details, recording receipt-observed/not-observed/inquiry-unavailable as an immutable observation. A missing transport response or one negative inquiry does not establish financial failure. Repeated execution uses the same identity and recovers the accepted native receipt; it cannot create another journal.

Readback verifies the frozen source, material details/digest, Controls identity and native receipt. Receipt verification re-reads the original immutable source and checks that the new actual Core batch is a CORRECTION for the original execution, linked to the exact original and containing precisely the approved reversing legs. A receipt reference changed by an incorrect trusted storage repair cannot claim completion. A source corrected by a different request is `SOURCE_ALREADY_CORRECTED`, with no new execution control.

Historical accepted receipts survive later approval expiry or checker revocation. Current maker/inquiry authority remains required. A raw Core financial receipt, rather than a Finance label, is required for `EXECUTED`.

## API and workforce review

Full prefix: `/api/v1/staff/finance`.

| Method/path | Behavior |
| --- | --- |
| `GET /corrections` | Latest 100 durable requests, receipts and links |
| `POST /corrections` | Stage original source/reason using `Idempotency-Key`; 202 |
| `GET /corrections/{id}` | Reopen original frozen scope, native receipt and latest 50 observations |
| `POST /corrections/{id}/approval` | Independent canonical checker approves `detailsDigest` |
| `POST /corrections/{id}/execution` | Original current maker executes/replays exact approved scope |

This increment introduced four patterns/five operations at the 106-pattern checkpoint; the [native basis increment](periodic-native-basis.md) subsequently expands the contract. Workforce authentication, current Finance grants/queue, Origin/CSRF, safe errors and ingress quota apply. Invalid funding sources/fields return 422; conflicting identities/current states/native restrictions return 409; dependency failures retain safe unavailable responses.

**Financial corrections** in the staff portal exposes the captured original source, native/typed hashes, exact two-leg amounts, reason, people, expiry, material approval checkbox and explicit customer-debit execution acknowledgment. Saved URLs reopen the same request; uncertain proposal keys remain in session storage. A native receipt panel shows accepted journal/operation/original/digest/UTC time, with links to original and correction GL exports. Checker inquiry does not provide execution authority. The navigation scrolls within its desktop sidebar so all workspace items and the synthetic footer remain accessible.

See [sequence UML](current-finance-corrections.puml), [domain UML](current-finance-correction-domain.puml), [C4 components](current-finance-components.puml), [operator runbook](../runbooks/finance-corrections.md) and [checkpoint evidence](evidence/WP-24-corrections.md).

## Remaining scope

[Native historical basis observations](periodic-native-basis.md) are implemented separately. Periodic enrollment/material terms, authoritative economic finalization and fee/accrual posting jobs remain open. Corrections for future periodic journals need their own native source eligibility once those journals exist; they are not authorized by this funding-only capability. Broader provider/payment refund controls continue to use their existing signed settlement/return workflows. Production financial policy, non-UTC/DST qualification and live-cloud/provider certification remain separate work.
