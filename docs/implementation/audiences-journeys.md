# WP-38 audiences, insights and the local journey partner (`SIM_JOURNEY`)

WP-38 adds migration `0072_audiences_insights.sql` (schema 72) on top of the WP-37 warehouse. It builds consent-checked
audiences, advisory insights and **one local journey-partner simulator role**, following the user's direction to build one
local template where no partner is selected. Real audience and journey partners need a decision (DEC-05, DEC-06,
DEC-09), contracts and managed keys. None is claimed here.

## Signals and segments

Audiences use minimized boolean signals per party, computed from warehouse facts at a fact cut:

| Signal | Meaning |
| --- | --- |
| `ACTIVE_ACCOUNT` | The latest observed state of one of the party's accounts is ACTIVE. |
| `RECENTLY_CREDITED` | A customer account of the party received a credit with a business date in the last 30 days. |
| `NO_RECENT_ACTIVITY` | No ledger entry on the party's customer accounts in the last 30 days. |

No amount, balance, counterparty or account identifier is used outside the warehouse. Segments are versioned and carry
a definition digest:

| Segment | Purpose | Signals | Bank insight rule |
| --- | --- | --- | --- |
| `savings_starter` v1 | `MARKETING` | active, recently credited | `savings_starter_tip` v1 (suggestion) |
| `dormant_check_in` v1 | `SERVICE_OUTREACH` | active, no recent activity | `dormant_check_in_notice` v1 (service notice) |

`SERVICE_OUTREACH` is a new purpose for the bank's own operational outreach. It needs no marketing consent, still
respects `ANALYTICS` restrictions, and is never exported to a partner.

## Consent at every step

| Step | Check |
| --- | --- |
| Build | Requires a role for the segment's purpose. Each candidate is excluded if it has an `ANALYTICS` restriction or, for marketing, a `MARKETING` restriction or no current marketing permission (WP-26 preference, default off). Exclusions are counted. Members receive a per-audience pseudonymous token (`m_` plus 32 hex characters). |
| Export | Requires a purpose role, and the partner must permit the audience's purpose. Consent is re-checked per member, and withdrawn or restricted members are excluded and counted. |
| Delivery | The partner fetches its export with a signed request. Consent is re-checked again: members who withdrew since the export are withheld and counted. Only tokens, the segment and the purpose are returned. |
| Display | A customer sees service notices always, and suggestions only while marketing is permitted. Withdrawal hides suggestions at once. Nothing is deleted: audiences, members, exports, insights and partner messages are immutable evidence. |

## Journey partner channel (advice only)

`SIM_JOURNEY` signs requests and responses with HMAC-SHA256 using a synthetic local key (`BANK_JOURNEY_PARTNER_KEY`).
A wrong signature returns 401 with no effect. A recommendation names a member token, an allowlisted recommendation code
(`SAVINGS_TIP`, `BUDGET_REMINDER`, `PRODUCT_EDUCATION`), a plain-text title and body, and optionally an allowlisted
in-app route (`#/accounts`, `#/payments`, `#/profile`). Outcomes:

| Result | When |
| --- | --- |
| `ACCEPTED` | A valid recommendation for a member exported to this partner who still permits marketing. It becomes an advisory insight with partner provenance. |
| `SUPPRESSED` | The member no longer permits marketing. Recorded; no insight. |
| `REJECTED` | Any other field (for example approval, entitlement or limit fields), an unlisted code or route, markup, or an unknown token. Recorded with the reason. |

Each message id has one stored, signed response that duplicates replay; reusing an id with other content returns 409.
The only table a partner can cause to change is `analytics.insights`. No function in the channel reaches Core,
entitlements or onboarding. The WP-37 privilege test covers every analytics function, these included, and the
acceptance test checks that Core entries, holds, role grants and applications are unchanged.

## Customer insights

`GET /api/v1/insights` returns insights with source, purpose, an in-app route and a plain-language "why" (the rule and
version, or the partner). The customer portal shows them as **Suggestions and notices** on the Notifications page, next
to the marketing preference that controls them. Customers can dismiss their own insights. Opening a route only navigates:
every action goes through the normal journey and its controls.

## Outcome reports

An outcome report for an audience records:
- **Denominators:** candidates, members, exclusions at build, tokens exported, and members who withdrew since.
- **Outcomes:** members with a customer-account credit ingested after the build, insights created and dismissed, and
  partner suggestions accepted. Each figure states its measure.
- **Watermarks and completeness:** dataset watermarks, and the flags `MISSING_SOURCE`, `STALE_WATERMARK` and
  `LATE_DATA` (projected rows recorded after the build but not yet ingested). The status is `COMPLETE` only without
  flags.

Reports are immutable and digested.

## Interfaces

| Audience | Endpoints and pages |
| --- | --- |
| Staff | `GET`/`POST /api/v1/staff/audiences`, `POST /api/v1/staff/audiences/{id}/exports`, `POST /api/v1/staff/audiences/{id}/outcome-reports`. Staff portal: **Audiences & journeys** (data stewards). |
| Customer | `GET /api/v1/insights`, `POST /api/v1/insights/{id}/dismissal`. Customer portal: **Suggestions and notices** on Notifications. |
| Partner | `GET /channels/journeys/v1/exports/{id}`, `POST /channels/journeys/v1/recommendations` (signed). |

The model is [current-audiences-journeys.puml](current-audiences-journeys.puml).

## Not in scope

- Real journey or audience partners, their formats and managed keys (DEC-05, DEC-06, DEC-09).
- Campaign scheduling, message delivery to customers outside the app, and attribution beyond the stated measure.
- Advisory journeys that execute anything. Insights only link to existing in-app pages.
