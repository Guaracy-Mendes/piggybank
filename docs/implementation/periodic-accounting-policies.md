# Periodic accounting policies and exact previews

Date: 2026-11-08. WP-24 remains **in progress**. This increment implements approved immutable policy configuration and deterministic previews. The original increment provided configuration and manual previews. Subsequent [native basis observations](periodic-native-basis.md) and [funding-only corrections](finance-corrections.md) are implemented separately; posting jobs, customer enrollment/terms and authoritative economic finalization remain unfinished.

## Ownership and capability boundary

Core owns `core.periodic_policies` and the exact calculation/calendar engine in [accruals](../../crates/core/src/accruals/mod.rs). Finance receives a narrow `PolicyRegistry` configuration capability in addition to its existing read-only `FinanceRecords` port. The registry can validate products, publish approved preview policies, read/resolve versions and calculate a preview. It exposes no ledger, reservation, allocation, reversal or account-enrollment method.

Every policy has `label=SYNTHETIC` and `activation=PREVIEW_ONLY`. No default policy is seeded and no customer receives a new periodic fee or interest posting. Existing payment fees retain their original single Core posting. An approved preview amount is not a journal, authoritative closing balance, liability, payment, GL booking or customer obligation.

Finance owns durable request/key scope, reason, approval registration recovery and the workforce interface. Controls owns `PERIODIC_POLICY` proposals, canonical identity, current grants/queue membership and independent approval. Core's immutable policy row is the authoritative publication receipt. Schema migration [0048](../../migrations/0048_periodic_preview_policies.sql) creates the separate storage and guarded configuration function without altering any prior financial or report V1 serialization. Forward migration [0049](../../migrations/0049_periodic_publication_clock.sql) rechecks approval/grant expiry with the live wall clock after a rule-lock wait and timestamps publication at insertion; applied migration 0048 remains unchanged.

## Policy and version rules

| Material field | Enforced meaning |
| --- | --- |
| ID / rule code / version | Non-nil UUID; uppercase code up to 32 characters; positive signed-i32 version |
| Product version | Exact existing immutable synthetic product; the whole policy period must fit its validity |
| Effective range | Inclusive start, exclusive end; 1–366 days, years 2000–2200 |
| First version | Version 1 for the product-version/rule-code namespace |
| Later versions | Exactly previous version + 1; start at or after prior end and current UTC date; calculation kind cannot change |
| Selection | A date resolves the single immutable interval; a gap or the exclusive end resolves no policy |
| Changes | A new policy UUID and independent approval; no update/delete/truncate of a published version |

Intervals are disjoint. An open interval cannot be silently superseded; choose finite intervals when planning subsequent prices. Publication serializes on product/rule identity so concurrent competing versions yield one winner. The calendar is a complete **policy-scoped snapshot**; its ID/code/version are labels within the policy, not an independently mutable global calendar registry. Every reference and preview also binds the whole policy digest.

## Explicit business calendar

The only supported timezone is `UTC`. Other timezones, including DST regions, are rejected. This is a deliberate finite synthetic profile; the broader BR-15 non-UTC/DST qualification remains open.

A calendar declares a non-nil identity, positive version, finite half-open coverage of at most 400 days, sorted unique ISO weekdays (Monday 1 through Sunday 7), at most 366 sorted unique covered holiday dates, exact `HH:MM:SS` cutoff (seconds 00–59) and `FOLLOWING_14_DAYS`. Coverage starts no later than the policy start and extends beyond policy end + 14 days. Holidays have no inferred real-world meaning: only explicitly declared dates close the calendar.

The due timestamp is the declared cutoff on the first business day on or after the period's exclusive end, searching offsets 0 through 14 inclusive. Equality at cutoff is due; a microsecond earlier is not due. Policy validation checks **every** possible period close, rejecting a closure that cannot roll within fourteen days. A daily period still accrues for a weekend/holiday date; its due time rolls. Daily periods with the same rolled due day remain distinct economic periods for future job design.

## Exact calculations

`MONTHLY_FIXED_FEE` requires a positive TST Money amount and first-of-month effective boundaries. A preview period starts on the first and ends at the next first. There is no proration and no supplied balance. The exact fraction is `amountMinor / 1`.

`DAILY_CREDIT_ACCRUAL` requires 1–10,000 annual basis points, `ACT_365_FIXED`, `NONNEGATIVE_CLOSING_BOOKED` and `HALF_UP_EACH_DAY`. The period is one actual calendar day, including leap day. A nonnegative operator-supplied TST Money balance is mandatory:

```text
numerator   = closingBookedMinor * annualBasisPoints       # i128
denominator = 10_000 * 365                               # 3_650_000
roundedMinor = (numerator + denominator / 2) / denominator  # integer division
```

All monetary operands/results use canonical signed-i64 decimal strings; widened numerator and denominator use decimal strings too. No floating point or JavaScript Number carries money. Zero balance or a small amount can produce an explicit zero preview. Negative basis, missing daily basis, supplied monthly basis, numeric rather than string money, unsupported rate/day-count/rounding and an out-of-period preview are rejected. The chosen day-count remains 365 in leap years; it is an explicit fixture convention, not a production financial policy decision.

For example, basis `9007199254740993` at 365 basis points yields exact numerator `3287627727980462445`, denominator `3650000` and rounded amount `900719925474` minor units. A 5,000-minor-unit basis rounds an exact half minor unit to one. Daily rounding does not imply a later period-level rounding or accumulation strategy.

## Durable approval and replay

1. Current Finance maker role and `finance-operations/REVIEW` membership are checked. Finance commits the exact policy, reason, actor/client/key, digest and fixed 24-hour expiry before crossing Controls.
2. The worker registers the original proposal UUID and whole policy/calendar digest. The bounded fifty-request cursor advances past failed/revoked commands and wraps; an acknowledgment lost after Controls committed is repaired by idempotent registration.
3. A current Finance checker with `APPROVE` membership reviews and approves the exact digest. A second session/issuer for the maker's canonical person cannot serve as checker. Wrong purpose, stale/revoked authority, changed details and expiry fail closed.
4. The original canonical maker publishes through the narrow Core registry. The guarded native function independently checks canonical JSON SHA256, exact approved scope, independent people, current maker/checker roles and queues, product bounds and serialized version rules. Live expiry checks run after any publication-lock wait; transaction-start time cannot preserve expired authority.
5. Core commits the immutable publication receipt. A lost response needs no new policy, approval or financial command: read/replay the original request. Published replay still requires current maker authority and exact scope, while the accepted historical receipt survives later approval expiry.

Only registration is automatic. The worker never approves or publishes a staged policy. Request material is immutable; only registration timestamp/attempt/reason can progress. A competing approved proposal may remain approved after another policy wins publication; it cannot replace that winner.

Controls approval binds all financial/configuration material. The immutable request reason is displayed alongside that material; it remains request metadata and is not part of the policy's reusable calculation digest.

## HTTP and staff interface

| Method and staff Finance path | Behavior |
| --- | --- |
| `GET /periodic-policies` | Latest 100 durable requests with publication receipts |
| `POST /periodic-policies` | Stage exact policy/reason using `Idempotency-Key`; 202 |
| `GET /periodic-policies/{id}` | Reopen original request and publication receipt |
| `POST /periodic-policies/{id}/approval` | Independent checker approves `detailsDigest` |
| `POST /periodic-policies/{id}/publication` | Original canonical maker publishes `detailsDigest` |
| `POST /periodic-policies/{id}/preview` | Read-only deterministic computation from a published policy and supplied period/basis/clock |

The full prefix is `/api/v1/staff/finance`. All routes require workforce authentication and current Finance inquiry authority; writes require maker or checker authority as appropriate. Every POST, including calculation, enforces the existing BFF Origin/CSRF boundary. Input/identity conflicts produce 422/409; dependency failures retain safe errors. No provider HTTP request is needed for calculation.

The generated OpenAPI/client describe five patterns and seven operations. The portal's **Periodic policies** workspace offers material fields, exact large-string fee/balance entry, immutable calendar/digest/people review, independent approval, publication receipt and computed due/amount/fraction, with explicit returned UTC due and evaluation clocks. It uses the same actor/material-derived session-stored key and UUIDs after an uncertain response. Saved request URLs reopen records, including records older than the list window; current checker UI can inspect and calculate without proposing/publishing.

## Verification and next increment

See [checkpoint evidence](evidence/WP-24-periodic-policies.md) for actual executed results and [operator runbook](../runbooks/periodic-accounting-policies.md) for both preview examples. [C4 components](current-finance-components.puml), [domain UML](current-periodic-policy-domain.puml) and [publication sequence](current-periodic-policies.puml) show the implemented boundaries.

Execution remains a separate WP-24 increment: explicit customer enrollment and material terms, product/rule/period economic uniqueness, authoritatively finalized native basis building on [Core observations](periodic-native-basis.md), available-funds/restriction checks, native fee/accrual journal commands and original receipts, durable scheduled jobs/cutoff recovery, approved counter-account/GL mapping, and independently approved linked corrections. A preview's operator-supplied basis must never be reused as that authoritative historical basis. Production fee/rate/tax/calendar decisions remain DEC-03 inputs.

The subsequent [funding correction increment](finance-corrections.md) exposes an independently approved native financial command for original synthetic funding only. It does not enable periodic postings or authorize reversal of future periodic fee/accrual journals.

The subsequent [native basis increment](periodic-native-basis.md) captures immutable committed journal prefixes and account metadata directly in Core. Its calculation uses the saved native clock, with explicit provisional/closed-observation status. It does not turn a closed observation into authoritative economic finalization or enable financial postings.
