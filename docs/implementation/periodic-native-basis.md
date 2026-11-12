# Native periodic ledger basis observations

Date: 2026-11-11. WP-24 remains **in progress**. This increment replaces operator-supplied balance assumptions with a separate immutable Core evidence artifact. It enables observation and calculation previews; periodic enrollment and financial execution remain open.

## Authority and storage

[Migration 0051](../../migrations/0051_periodic_native_basis.sql) adds immutable `core.periodic_bases`, an account/journal lookup index, the guarded `core.capture_periodic_basis` function and a Finance audit-recovery cursor. Runtime Core can select artifacts and execute the guarded capture function, but cannot insert, update, delete or truncate the evidence table directly. Applied migrations 0001–0050 and existing report, journal and preview V1 formats remain unchanged.

Core exposes the narrow [BasisRecords capability](../../crates/core/src/accruals/basis.rs), alongside the separate PolicyRegistry, read-only FinanceRecords and funding-only CorrectionCommands capabilities. BasisRecords has no money, hold, enrollment or customer-terms command. Finance uses it through [basis orchestration](../../crates/finance/src/basis.rs). Captures never imply consent, account activation in a historical period, an amount owed, a booked fee or a completed accrual.

## Capture procedure and exact evidence

1. Require workforce authentication, current canonical Finance maker authority and `finance-operations/REVIEW` membership. The actor/client/non-nil key is the durable replay identity.
2. Acquire the actor/key advisory lock. Recheck canonical identity and current role/queue validity with the live wall clock after any wait. A caller whose authority expired while waiting cannot save an observation.
3. For a saved key, verify the exact policy/account/period/person scope and return the original native artifact. Current capture authority is still required. A changed scope returns conflict.
4. Resolve the exact published synthetic `PREVIEW_ONLY` policy and exact account product version. The account must be registered in Core. Reject future period starts, invalid whole-month boundaries and uncovered policy/calendar dates. Compute the due time with the frozen UTC calendar and `FOLLOWING_14_DAYS` convention.
5. In one native SQL statement snapshot, select account metadata/current position and every visible committed account journal fact with business date before the exclusive period end. Sort by business date, posted time and journal UUID. Retain at most 1,001 candidates to detect an excessive prefix, then reject over 1,000 without storing anything.
6. Store original actor, canonical person, native UTC capture clock, policy and digest, exact account/product/registration/state/restrictions, current booked/held/available position, selected native facts and derived totals. Seal source and whole-artifact SHA256 digests and insert atomically.
7. Re-read and verify the artifact against immutable native journal facts. Finance writes a stable audit event with the basis UUID and original person/time, then returns the observation.

A concurrent financial commit is either visible to this statement snapshot or excluded from it. The selected current position and journal prefix share the same MVCC visibility. The capture timestamp identifies the observation; it is not a global transaction watermark or proof that every transaction committed before that wall-clock instant is included.

Each source item preserves ordinal, native journal/operation UUIDs, posting kind, business date, original-journal link when present, original posted UTC timestamp, native journal fingerprint and this account's signed minor-unit delta. Capture does not expose other accounts' legs. Source membership is determined by **business date**, not submission time, provider event time or current booked balance. External settlement journals therefore follow their native settlement business date.

```text
openingBooked         = sum(account delta where businessDate < periodStart)
periodChange          = sum(account delta where periodStart <= businessDate < periodEnd)
observedClosingBooked = openingBooked + periodChange
currentAvailable      = currentBooked - currentHeld
```

All per-fact deltas/current monetary values are canonical signed-i64 strings. Historical sums and exact calculation numerators use checked signed-i128 arithmetic and canonical decimal strings. The browser uses BigInt-based formatting. Holds are shown separately and never reduce the historical booked-balance basis. Current booked/held/available values are retained at capture time and never replace a historical total.

The 1,000-fact limit includes opening history. Item pagination does not enlarge that limit or truncate source membership. Larger accounts need an independently designed checkpoint/aggregation capability; no synthetic starting balance is inferred.

## Observation status and calculation eligibility

| Field or outcome | Meaning |
| --- | --- |
| `CORE_BUSINESS_DATE_BASIS_V1` | Native committed journal prefix, distinct from the original operator-supplied preview DTO |
| `OPEN_PERIOD_PROVISIONAL` | Exclusive period end is after the captured UTC date; additional facts can arrive during the period |
| `CLOSED_PERIOD_OBSERVATION` | Period ended by the captured UTC date; this is not a global book close, finalized economic period or guarantee against late postings |
| `ACCOUNT_REGISTERED_AFTER_PERIOD_START` | Evidence is retained, but no calculation is returned for a period preceding account registration |
| Negative or out-of-i64 daily closing basis | Evidence is retained with no calculation; the value is never clamped into a chargeable balance |
| Eligible daily basis | Exact existing ACT/365 fixed calculation using native observed closing booked balance |
| Eligible monthly basis | Exact fixed fee for the complete month; no balance-dependent fee formula |

Calculation uses the saved native capture clock for `isDue`, the saved policy/calendar for due time and the saved native prefix for daily basis. Its `basisSource` remains native and `provisional` follows period status. A returned monthly preview amount does not assert enrollment or an obligation. Account registration metadata alone does not establish customer consent or historical activation.

Late business-date facts require a **fresh key and artifact**, even for an already-ended period. Saved capture/detail/item reads never rescan new facts into the old evidence. Exact replay keeps original clock, totals, source count and digest; it cannot silently become a newer observation. Future periodic posting requires separate economic uniqueness, finalization and explicit accepted basis/terms decisions.

## Integrity and recovery

Every read checks raw artifact SHA256, exact typed/raw representation, policy digest, source hash/count/order/uniqueness, actor/person/scope/timestamp columns, calendar/period/status, exact current availability and historical totals. It then independently loads the saved native journal UUIDs and account legs and compares the original immutable native facts. Recomputing all stored hashes around a forged amount is insufficient to pass native readback. Missing/damaged facts or scope conflicts fail closed; original evidence is not repaired by replacing financial facts.

Core insertion can succeed before Finance audit acknowledgment. A worker scans fifty Core basis UUIDs with a persisted wrapping cursor, verifies each artifact and inserts the original audit event idempotently. The cursor advances past damaged/unavailable records and wraps to retry them. Recovery never posts money, captures a replacement basis, approves a policy or changes a saved artifact. Stable audit UUID prevents duplicate audit delivery after response loss.

## HTTP and workforce workflow

All paths use `/api/v1/staff/finance`. Workforce session/bearer authentication, current Finance authority and BFF Origin/CSRF for POST apply.

| Method / path | Contract |
| --- | --- |
| `POST /periodic-bases` | `Idempotency-Key` UUID plus exact native `policyId`, `accountId`, `periodStart`; returns 201, including replay |
| `GET /periodic-bases?policyId=...&accountId=...` | Optional exact filters, newest 100 verified artifacts |
| `GET /periodic-bases/{id}` | Reopen original verified detail, independent of list window |
| `GET /periodic-bases/{id}/items?after=0&limit=25` | Stable ordinal pages; after 0 through source count; limit 1–100 |

Key conflicts and native account/product/period guards return 409; structurally invalid input, nil identifiers, zero/oversized pages and excessive native history return 422. Authentication/authorization and dependency errors retain existing safe boundaries. Unknown input fields such as a supplied balance or clock are rejected.

The published **Periodic policies** screen embeds native capture, saved basis list, account/date entry, original UTC clocks, current position versus historical totals, exact fractions, source journal provenance and five-item incremental pages. It preserves actor/material-derived session keys for retries, with a separate explicit **Capture fresh observation** action. A saved URL `#/finance/policies/{financeRequestId}/bases/{basisId}` binds the exact native policy; a mismatched policy URL cannot display another policy's basis. Checkers can inspect but cannot capture. Async responses are scoped to the current policy/selection and invalidated when the screen closes, so a delayed committed capture cannot navigate the operator back into a workspace they left.

See the [step-by-step runbook](../runbooks/periodic-native-basis.md), [evidence](evidence/WP-24-native-basis.md), [C4 components](current-finance-components.puml), [capture sequence](current-periodic-native-basis.puml) and [domain UML](current-periodic-basis-domain.puml).

## Remaining implementation

WP-24 still requires explicit customer enrollment/material terms, scheduled durable jobs, globally authoritative economic period uniqueness/finalization, accepted native fee/accrual commands/counter-account mappings, and periodic correction eligibility. Native observations do not by themselves complete those requirements. Existing funding-only corrections remain separate.
