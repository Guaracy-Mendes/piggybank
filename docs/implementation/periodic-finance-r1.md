# Essential periodic financial execution

WP-24 adds financial execution separately from the existing preview registry and review receipts. See the [accepted scope](wp24-essential-scope.md), [operator runbook](../runbooks/periodic-finance-r1.md), [C4 components](current-periodic-execution-components.puml), [sequence](current-periodic-execution.puml) and [domain UML](current-periodic-execution-domain.puml).

## Authority and lifecycle

Documents derives financial offer material V1 from the exact approved policy/calendar. A distinct `PERIODIC_OFFER` proposal binds its complete digest and requires current, independent Finance maker/checker people and queues. Publication retains immutable material and publication proof. Existing PREVIEW_ONLY policies and PREVIEW_REVIEW_ONLY receipts keep their original meaning.

Only the current sole active owner may explicitly accept an eligible product-bound offer. The customer supplies its displayed digest and `accepted: true`; amounts, text, actors and clocks come from native sources. Consent retains the complete published offer, accepted time, earliest whole period and private attribution digest. Actor/client/key mappings and redacted Documents outbox facts commit atomically. Original retries return the actual current receipt state. A withdrawal before activation is immutable and terminal for that consent.

Finance stages activation using the consent ID and explicit control counter account. Its exact consent digest/details require a distinct `PERIODIC_ACTIVATION` independent approval. Core rechecks the original maker, actual checker, roles/queues/expiry, current sole ownership, consent/material integrity, account eligibility and approved GL classification under native locks. Fees require an explicit revenue mapping; interest requires an explicit expense mapping. The first whole period starts after both consent and activation, never by reading or preview acknowledgment. A current Finance maker may stop future effects; no resume is available in this scope.

## Native period finality and money

Core serializes financial account rows in a stable order. Finalization accepts only the next sequential closed due UTC period and refuses a skipped or still-unposted prior period. It records at most 1,000 committed journal facts before period end, including opening history, their original IDs/dates/deltas/fingerprints, closing booked basis, source digest, due time and native finalization clock. The receipt is immutable and unique for account/policy/period.

A closed account-period watermark prevents later runtime financial entries from being backdated into its source prefix. Runtime journals use the native bank business date; a provider's settlement/value date remains original provider evidence. This is account-period finality, not a global accounting close. Historical preview observations are not finalizations.

Monthly fees use exact approved minor units for whole calendar months. Daily interest uses nonnegative closing booked minor units × annual basis points / 3,650,000, rounded half up each day. Native integer/numeric arithmetic rejects overflow. Positive effects post exactly two balanced legs through existing Core account/available-funds/hold/restriction controls; zero accrual saves an explicit posted receipt with no zero journal. Funds insufficiency retains the immutable finalization for later retry. The native receipt independently verifies source facts, calculation and actual journal legs; rehashing a forged amount is insufficient.

A new journal and its Core outbox event commit atomically. Existing Finance discovery exports one source batch with the approved chart and authenticated simulator evidence. A job flag, scheduling acknowledgment or GL projection cannot substitute for the native financial receipt.

## Durable processing and corrections

Finance commits scoped period intent before scheduling. A retained wrapping cursor repairs unscheduled intents and scans active enrollments, deriving the next due period from native policy/clock. Multiple jobs or keys may converge on the same economic period; Core permits one effect. The stored original maker must still have current authority. Failed source/authority/posting work creates an owned Finance exception without preventing later cursor pages from progressing. Worker retries are bounded; a new authorized manual run can resume the same retained Core period after review.

The existing full correction workflow now accepts native `PERIODIC_FEE` and `PERIODIC_ACCRUAL` originals as well as synthetic funding. `FULL_PERIODIC_REVERSAL_V1` approval binds the exact original source/fingerprint/legs/reason. Core verifies the original period receipt, independent current authority and available funds/restrictions, then writes one linked reversal. Original journals, original closed basis and prior reports stay unchanged; later observations show the correction and GL lineage.

## Files and interfaces

Migrations 0054–0056 add offer/consent custody, native activation/finalization/posting and periodic correction controls. Narrow Rust capabilities live in `bank_documents::offers`, `bank_core::accruals::execution` and `bank_finance::periodic_execution`. API inputs deny unknown fields and never accept an execution amount or clock. The [implemented OpenAPI](../../contracts/openapi/implemented.json) and generated client describe all methods.

| Audience | Interface |
| --- | --- |
| Finance staff | `/api/v1/staff/finance/periodic-offers`, `/periodic-activations`, `/periodic-runs`, with scoped detail/approval/publication/execution/stop operations |
| Customer owner | `/api/v1/accounts/{id}/periodic-offers/{offerId}` and explicit `/enrollment`, `/withdrawal` |
| Staff portal | `#/finance/periodic/offers/{id}`, `#/finance/periodic/activations/{id}`; original calculation previews remain under `#/finance/policies` |
| Customer portal | `#/accounts/{id}/offers/{offerId}`; native movements/corrections remain in account history |

The seed publishes approved chart v2 with interest expense control account ending `000005`; immutable chart v1 remains unchanged. Fee revenue control account ends `000004`. These are synthetic fixture accounts, not real banking accounts.
