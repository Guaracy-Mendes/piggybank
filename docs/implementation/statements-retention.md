# WP-25 statements, secure retrieval, retention and legal holds

Implemented 2026-11-17 as migration `0062_document_statements.sql` (schema 62). Covers FR-DOC-02, FR-DOC-03 and FR-DOC-04, and acceptance scenarios AT-15 and AT-16. AT-05 (exact accepted terms) was already delivered by WP-16 and is unchanged.

The Documents module owns this capability. Core remains the only financial authority: every statement figure is read from immutable Core journals. Documents never stores or changes a balance.

All money is synthetic TST. Retention periods are versioned **fixtures** labeled `SYNTHETIC_FIXTURE`. They are not a legal retention schedule, which DEC-06 must still decide.

## Statements (FR-DOC-02)

A statement covers one account for one UTC calendar month `[periodStart, periodEnd)`.

### Readiness

`documents.statement_readiness` publishes nothing unless every check passes. It returns these reasons:

| Reason | Meaning |
| --- | --- |
| `PERIOD_OPEN` | The month has not ended in Core's clock (`core.periodic_clock()`). |
| `PERIODIC_FINALIZATION_PENDING` | An enrolled periodic policy (monthly fee or daily accrual) has no WP-24 finalization through the month end. A policy stopped before the month end does not block. |
| `ACCOUNT_NOT_OPEN_IN_PERIOD` | The account opened after the month ended. |

Each refusal is recorded as `GENERATE / NOT_READY` access evidence, and the caller receives the reasons.

### Snapshot

Generation follows these steps:

1. **Lock.** It takes an advisory lock for (account, month), then locks the Core ledger account `FOR SHARE`. This waits for any poster still holding the account. Runtime bookings are always dated from Core's UTC clock (migration 0055), so once the month has ended no new runtime fact can enter it.
2. **Read.** It reads all of the account's journal entries with `business_date` in the month, plus the opening total before the month and the closing total before the month end. It refuses to proceed above 1,000 facts and never truncates.
3. **Reconcile.** It requires `closing = opening + Σ movements`, or refuses.
4. **Report corrections.** Corrections booked after the month whose `original_batch` lies in the month are listed under `linkedCorrections`. They are disclosed alongside the month's figures and never included in its totals.
5. **Digest.** `snapshotDigest` is the canonical SHA-256 of the account, the period, the opening and closing totals, the facts and the linked corrections.

The document contains:
- opening and closing totals
- credits, debits, net, fees (`PERIODIC_FEE`), interest (`PERIODIC_ACCRUAL`) and corrections (facts with an original batch)
- the facts and linked corrections
- the readiness basis
- the assigned retention policy and the generation time

Its canonical digest is `contentDigest`.

### Versions

Generation is idempotent by content:

| Situation | Result |
| --- | --- |
| Snapshot identical to the latest version | That version is returned (`created: false`). |
| Only `linkedCorrections` changed | New version `LINKED_CORRECTION`, superseding the previous one. Its totals are identical. |
| Opening or facts changed (only possible through an owner-path backdated insert) | New version `SOURCE_CHANGED`, so the discrepancy is visible instead of silently absorbed. |
| Latest version disposed | Regeneration is refused. |

Versions and objects are immutable. Earlier versions stay retrievable until their own retention ends.

The version metadata row, the object row and the access evidence are written in one transaction. A failed object write publishes nothing.

### Generation paths

- **Customer:** must be a current owner (`account_relationships` `OWNER`, not revoked).
- **Workforce:** requires `account_support` or `records_officer`.
- **Worker:** `documents.generate_due_statements(limit)` runs every 60 seconds. For accounts without a version for the latest closed month and whose periodic finality is complete, it calls the same internal function under a `SERVICE` actor. A failure is recorded and retried on a later pass.

## Secure retrieval (FR-DOC-03)

Statement content is never returned by a list endpoint. Retrieval takes two steps.

**1. Link.** `POST …/statements/{id}/links` rechecks authority (owner, or a support/records role) and refuses disposed versions. It creates a random 64-hex reference that expires after **two minutes**, stores only its SHA-256, and binds it to the issuer, subject, trust, client and person.

**2. Redeem.** `GET …/downloads/{token}[?format=pdf]` locks the link and denies any of the following:

| Denial | HTTP |
| --- | --- |
| Unknown reference | 404 |
| Another actor (`ACTOR_MISMATCH`) | 404 |
| Already used | 410 |
| Expired | 410 |
| Owner no longer current | 404 |
| Disposed | 410 |
| Object digest mismatch | 404 |

On success it consumes the link and returns the exact version, with `supersededBy` when a newer version exists.

Every link, retrieval and denial is written to `documents.document_access`, which is append-only. Denials return normally instead of raising, so their evidence commits. Customer and workforce use separate routes and sessions. The PDF is rendered from the redeemed version only.

Notifications that avoid sensitive content remain WP-26 scope. No statement content is sent through notifications.

## Retention, legal holds and disposal (FR-DOC-04)

### Policies

`documents.retention_policies` holds immutable versioned fixture policies:

| Class | Retention | Disposal |
| --- | --- | --- |
| `STATEMENT` | 2,557 days after the period end (seven years) | Permitted |
| `DATA_EXPORT` | 30 days after creation | Permitted |
| `ACCESS_EVIDENCE` | 3,650 days | Not enabled in this release |

Each record stores the policy and the `retainUntil` date assigned when it was created. A later policy version never shortens an existing record.

### Legal holds

A hold has scope `ACCOUNT` or `PERSON`. A person hold also covers accounts the person owns, the owners recorded on a version, and the account's current owners.

- **Placement:** requires `records_officer` and an idempotency key. The same key with different details is refused.
- **Release:** requires `records_approver`, and the approver must be a different person from whoever placed the hold. This is enforced by the function and again by a trigger, even when the person who placed the hold also holds the approver role.
- **Ordering:** placement and release take an exclusive advisory lock, and disposal takes the shared form. A hold committed first therefore always blocks a later disposal.

### Disposal

Disposal requires `records_officer` and a reason. Restrictions are returned as outcomes and recorded:

| Outcome | Cause |
| --- | --- |
| `RESTRICTED_RETENTION` | `retainUntil` is still in the future |
| `RESTRICTED_LEGAL_HOLD` | An active hold covers the record |
| `RESTRICTED_POLICY` | The policy does not permit disposal |

A permitted disposal first inserts `statement_disposals` evidence, then deletes the object, in one transaction. The evidence records:
- the content and object digests
- the size
- the policy and its version
- `retainUntil`
- the business date
- the hold check
- the reason, the request and the actor

The version row remains as a minimized tombstone. It keeps identity, digests and retention data, but no figures.

Triggers enforce the same rules on the owner path:
- Disposal evidence cannot be inserted while retention is running or a hold is active.
- An object can be deleted only when its disposal evidence exists.
- Updates and truncation are refused.

Access-request export manifests follow the same pattern through `data_export_disposals`.

## Data-subject requests (AT-16)

A signed-in customer submits `ACCESS`, `CORRECTION` or `DELETION` with an idempotency key. Verification is `AUTHENTICATED_CUSTOMER_SESSION`.

A records officer processes the request once; replays return the stored outcome. Processing gives every statement version of the person's accounts an item:

| Request | Item outcome |
| --- | --- |
| `ACCESS` | `INCLUDED`, or `ALREADY_DISPOSED`. A `DOCUMENTS_DATA_EXPORT_V1` manifest of included version identities and digests is stored under the `DATA_EXPORT` policy. Statements are still retrieved one at a time through single-use links. |
| `DELETION` | The permitted disposal path runs for each version, giving `DISPOSED`, `ALREADY_DISPOSED`, `RESTRICTED_RETENTION`, `RESTRICTED_LEGAL_HOLD` or `RESTRICTED_POLICY`. |
| `CORRECTION` | `RESTRICTED_FINANCIAL_RECORD`. Financial errors are corrected through linked journals and a superseding version. |

Deletion and correction also add a `CORE_JOURNAL` item marked `RESTRICTED_RETENTION`, because Core journals are outside document disposal.

The overall outcome is `ALLOWED`, `PARTIALLY_RESTRICTED` or `RESTRICTED`. A digest of the outcome is retained.

Customers see their requests and outcomes, but never which legal hold applies or who placed it.

## API

| Method and path | Actor | Purpose |
| --- | --- | --- |
| `GET /api/v1/documents` | Customer | Owned accounts, the last 24 months with readiness and versions, requests and policies |
| `POST /api/v1/accounts/{id}/statements` | Customer | Generate or replay, or receive `NOT_READY` with reasons |
| `POST /api/v1/documents/statements/{id}/links` | Customer | Single-use reference |
| `GET /api/v1/documents/downloads/{id}` | Customer | Redeem as JSON or `?format=pdf` |
| `POST /api/v1/documents/data-requests` | Customer | Submit a request (`Idempotency-Key`) |
| `GET /api/v1/staff/documents/accounts/{id}` | Support or records | Versions with retention and hold flags, holds, the last 50 access records, requests |
| `POST /api/v1/staff/documents/accounts/{id}/statements` | Support or records officer | Generate or replay |
| `POST /api/v1/staff/documents/statements/{id}/links` | Support or records officer | Workforce single-use reference |
| `GET /api/v1/staff/documents/downloads/{id}` | Support or records officer | Redeem |
| `POST /api/v1/staff/documents/statements/{id}/disposal` | Records officer | Dispose, or receive the restricted outcome |
| `POST /api/v1/staff/documents/legal-holds` | Records officer | Place a hold (`Idempotency-Key`) |
| `POST /api/v1/staff/documents/legal-holds/{id}/release` | Records approver (independent) | Release |
| `GET /api/v1/staff/documents/data-requests` | Records | The 100 most recent requests |
| `POST /api/v1/staff/documents/data-requests/{id}/processing` | Records officer | Per-record outcome |
| `POST /api/v1/staff/documents/data-requests/{id}/export-disposal` | Records officer | Dispose an expired, unheld export |

`/api/v1/staff/me` now reports the `records.read`, `records.manage` and `records.approve` capabilities. The local seed grants `records_officer` to `reviewer` and `records_approver` to `approver`.

## Clients

- **Customer portal:** `#/documents` ("Statements & documents") lists each account's months with readiness text, the current version's figures, earlier versions, single-use **View** and **Download PDF** actions, and data requests with per-record outcomes. The older custom activity report remains under Preferences.
- **Staff portal:** `#/records/{accountId}` ("Records & retention") shows statement periods with version, closing, retention, legal-hold and disposal state. It provides retrieval, disposal requests, hold placement and independent release, the access-evidence table and the data-request queue.

## Models

| View | Source |
| --- | --- |
| C4 components | [current-statement-components.puml](current-statement-components.puml) |
| Lifecycle sequence | [current-statement-lifecycle.puml](current-statement-lifecycle.puml) |
| Domain | [current-statement-domain.puml](current-statement-domain.puml) |
| States | [current-statement-states.puml](current-statement-states.puml) |

## Deliberate boundaries

- Statement periods are UTC calendar months only. Other cycles, time zones and proration are not supported.
- The fixture retention durations are not legal advice. Real schedules, jurisdictional holds and certification are external DEC-06 and DEC-11 inputs.
- Objects are stored in PostgreSQL rows rather than an external object store. The production object-store adapter, encryption keys and backup or legal-hold propagation remain WP-28, WP-29 and WP-31 scope.
- Access evidence is retained without disposal in this release.
- Notifications are WP-26 scope. Native mobile retrieval is WP-27 scope.
- Facts are capped at 1,000 per account-month, consistent with WP-24. Larger histories need checkpoint design.
