# Customer periodic rule disclosures

Current status: essential synthetic WP-24 is complete. This document retains the narrower increment it introduced; the [financial lifecycle](periodic-finance-r1.md), [scope decision](wp24-essential-scope.md) and [completion evidence](evidence/WP-24-complete.md) describe the completed execution and deliberate deferrals. Earlier references to remaining work below describe that checkpoint.

Date: 2026-11-12. WP-24 was **in progress at this checkpoint**. Customers can read the exact approved synthetic periodic rule material for an owned account. Reading does not accept terms, enroll an account, activate a policy or post money.

## Material and ownership

The [Core disclosure module](../../crates/core/src/accruals/disclosures.rs) derives versioned text directly from an immutable native periodic policy. It verifies the native policy digest and binds the complete policy/calendar, native policy digest, account UUID, renderer format version, source label, title and exact generated text into `digest = SHA256(canonical(material))`.

The source label is `CORE_APPROVED_PERIODIC_DISCLOSURE_V1`. Observation time and `CURRENT_PREVIEW`/`UPCOMING_PREVIEW`/`ENDED_PREVIEW` availability are outside the material digest. The same account/policy/template returns the same material digest after refresh or later observation dates. A different account or approved policy version has a different digest. The V1 rendering algorithm is part of that material contract. The current `/api/v1` disclosure endpoint must retain it; a future renderer requires explicit version selection or a separate versioned endpoint. Future material/template changes require a new version rather than silently rewriting earlier material.

Customer trust is required. Core checks current Controls account-read authority and rechecks the active native OWNER relationship in the product lookup. Unknown/foreign accounts, revoked ownership, unpublished policies and another product's policy are hidden. Staff inquiry authority does not become customer ownership. Closed-account history remains readable by a current owner under the existing account inquiry rules.

Only the policy and policy digest enter the disclosure. Maker/checker people, proposal IDs, publication reason, operator notes, audit actors, balances, holds and native basis artifacts are excluded. The source account must be registered against exactly the disclosed product version.

## Exact content

The disclosure includes the policy code/version, exact effective half-open range, TST synthetic label, frozen calendar identity/version/coverage, UTC timezone, explicit business weekdays/holidays, due cutoff and following-fourteen-calendar-days rule. No holidays are inferred.

Monthly text renders positive signed-i64 minor units exactly as TST decimal units and original minor units, without floating point. It explains complete-month charging and no proration. Daily text renders basis points as an exact percentage, nonnegative closing booked basis, ACT/365 fixed including leap years, denominator 3,650,000 and half-up rounding per day. It distinguishes the day being calculated from the rolled due date and separates held funds from booked balance.

Every displayed rule is `SYNTHETIC/PREVIEW_ONLY`. No default enrollment, customer obligation, legal consent, financial activation, historical calculation or booked interest is inferred. This text is product information for the evaluation system; legal/material terms publication and attributable customer acceptance remain separate future work.

## API and customer workflow

| Method and path | Behavior |
| --- | --- |
| `GET /api/v1/accounts/{id}/periodic-disclosures` | Current customer owner; all current/upcoming published policies for the exact account product, sorted code/version, up to 100 |
| `GET /api/v1/accounts/{id}/periodic-disclosures/{policyId}` | Same ownership/product checks; original version including an ended version outside the list |

The list reads at most 101 candidates and refuses over 100 with 422 rather than returning a truncated apparent complete list. A known saved policy can still be inquired independently. The customer screen can show a verified saved disclosure alongside the list-limit message, without fabricating a partial list. There is no balance, evaluation clock, enrollment, consent or mutation input.

The **Accounts** page embeds **Periodic rule previews**, exact generated text, current/upcoming/earlier labels and optional version/digest verification. Saved routes use `#/accounts/{accountId}/periodic/{policyId}` and survive fresh issuer login. Async replies are scoped to the current account/selection and invalidated on unmount. A failed inquiry clears prior displayed material. Plain text rendering avoids interpreting source content as HTML.

See the [customer/operator runbook](../runbooks/customer-periodic-disclosures.md), [checkpoint evidence](evidence/WP-24-disclosures.md), [C4 components](current-periodic-disclosures.puml) and [read sequence](current-periodic-disclosure-read.puml).

## Deployment compatibility and remaining scope

No migration is added: schema 52 is sufficient. A fresh release is required for the new code/UI. The package manifest now declares optional validated feature metadata; `customer-periodic-disclosures-v1` is derived from the implemented API inventory. Actual archive acceptance checks its embedded OpenAPI, customer bundle and channel guards when that feature is declared. Older schema-52 archives without it retain their original validation path and seals.

This increment prepares material visibility. Durable document publication/retention where needed, explicit customer acceptance/enrollment, current financial activation, authoritative economic period finalization, posting jobs/counter-account mapping and periodic corrections remain open. A preview disclosure must never be treated as consent to a different active policy or as a native financial receipt.

The subsequent [retained preview review increment](periodic-preview-reviews.md) adds explicit acknowledgment and an immutable copy of this exact material with private identity attribution. It preserves this V1 rendering contract and establishes historical review only. Financial consent/enrollment, legal/material terms publication where required, activation and posting remain open.
