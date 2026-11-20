# WP-36 synthetic batch credit rail supplement (`SIM_BATCH`)

WP-36 adds migration `0070_batch_rail.sql` (schema 70). It builds **one synthetic ACH-style batch credit rail**, which
the user chose on 2026-11-18 as the template for later rails. ACH on a real network, checks, debit, bill pay, wire,
lockbox, SWIFT and P2P each need a selection (DEC-04, DEC-09), their own supplement and certification. None is
claimed here.

## Contract (immutable fixture)

| Term | Value |
| --- | --- |
| Direction | Outbound credits from a customer's account to a recipient at another (synthetic) bank |
| Cutoff | 16:00 UTC on business days (weekends and fixture holidays skipped) |
| Settlement | T+1 business day after the effective queue date |
| Limit | TST 25,000.00 per entry, plus the delegate limit when a delegate initiates |
| Returns | R02 account closed, R03 no or unlocatable account, R04 invalid account number. Accepted for 2 business days after the settlement date. |
| Recipients | Name, 9-digit routing number with ABA-style checksum, and 4–17-digit account number. Customers only see the last four digits back. |

## Lifecycle and financial keys

| Step | Effect |
| --- | --- |
| Draft | Common rules apply: ownership or delegated PAY within its limit, account restrictions and the per-entry limit. Opens the standard action-bound approval challenge, with rail `SIM_BATCH` and the entry id as quote. |
| Confirm | Consumes the step-up proof (browser or native, the same as every payment) and reserves a Core hold keyed by the entry id. The entry becomes `QUEUED` with its settlement date. |
| Cancel | Allowed while `DRAFT` or `QUEUED`; releases the hold. |
| Window close | At the cutoff the worker seals every queued entry into one immutable outbound file. A payment operator can close it early. Entries become `SUBMITTED`. |
| Acknowledgement | Signed network message. Rejected entries release their holds (`REJECTED`, with reason). |
| Settlement | Signed network message. One `BATCH_CREDIT` posting per entry (customer → batch clearing), consuming the hold. Repeats post nothing more. |
| Return | Signed network message. Inside the window: one `BATCH_RETURN` (clearing → customer), linked to the original credit. After the window: flagged `REVIEW_REQUIRED`, never posted. |

Network messages are HMAC-SHA256 signed (`Piggybank-Batch-Signature`), using a synthetic key in the local profile
(SF-005). Each message id has one stored, signed response that duplicates replay; reusing an id with different
content returns 409. The bank does not police the network's settlement timing in the synthetic profile: the simulator
decides when to send settlement.

## Interfaces

| Audience | Endpoints and pages |
| --- | --- |
| Customer | `GET`/`POST /api/v1/batch/recipients`, `GET`/`POST /api/v1/batch/payments`, `GET /api/v1/batch/payments/{id}`, `POST .../confirmation`, `POST .../cancellation`. Customer portal: **Next-day transfers** in **Payments**. |
| Staff | `GET /api/v1/staff/batch`, `POST /api/v1/staff/batch/window-closure` (payment inquiry authority). Staff portal: **Batch rail**. |
| Network | `POST /channels/batch/v1/messages` |

The model is [current-batch-rail.puml](current-batch-rail.puml).

## Template for later rails

A new rail needs:
- its own supplement table (terms, calendar, reason codes) and simulator role;
- signed network messages with stored responses;
- Core postings keyed by the entry id with a rail-specific posting kind;
- the common approval challenge;
- an acceptance test that proves the duplicate, return and late-return paths.

## Not in scope

- Real ACH network formats (NACHA), prenotes and account validation, debits, same-day windows.
- Fees, operator-initiated reversals, and reconciliation of rail totals beyond per-entry evidence.
