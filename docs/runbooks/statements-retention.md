# Statements and records runbook

This runbook covers WP-25 [statements, secure retrieval, retention and legal holds](../implementation/statements-retention.md). TST has no monetary value, and the retention durations are synthetic fixtures.

## 1. Prepare

1. **Start the environment.** Start the persistent environment with [local development](local-development.md) or `python3 scripts/local_cx.py prepare && python3 scripts/local_cx.py start`. Migrations must report schema 62, with all earlier checksums unchanged.
2. **Grant roles.** `scripts/seed.py`, which `prepare` runs, grants `records_officer` to the `reviewer` workforce user and `records_approver` to `approver`. `account_support` (also held by `reviewer`) can generate and retrieve. Holds, disposal and request processing need the records roles.
3. **Use separate browser profiles.** One profile for the customer portal (default `http://127.0.0.1:4300`, user `alice`). One for operations as `reviewer` and one as `approver` (default `http://127.0.0.1:4301`). The person who places a hold cannot release it, even with both roles.

## 2. Customer statements

1. **Open the page.** In the customer portal, open **Statements & documents** (`#/documents`). Each owned account lists up to 24 months.
2. **Read the month status.** Each month shows one of:

   | Status | Meaning |
   | --- | --- |
   | *Available after the month closes (UTC)* | The month is still open. Nothing can be published. |
   | *Waiting for this month’s fees or interest to be finalized* | A WP-24 enrolled policy has not finalized through the month end. Run or recover the periodic job (see the [periodic finance runbook](periodic-finance-r1.md)). Do not create an incomplete statement any other way. |
   | *Ready to create* | Choose **Create statement**. |
   | *Version n* | A version exists. The worker may already have published it; it runs every 60 seconds for the latest closed month. |

3. **View or download.** Choose **View** to retrieve the current version through a new single-use link. **Download PDF** requests another link and downloads it once. A link expires after two minutes, and reusing one returns HTTP 410. Each view or download is recorded.
4. **Check the totals.** The figures shown are opening, money in, money out, fees, interest and closing, where closing = opening + money in − money out. Corrections booked after the month are listed separately, and closed-month figures never change.
5. **Find earlier versions.** If a later correction links to a fact in the month, regenerating (by the customer, staff or worker) creates version n+1, *Reissued with a later correction noted*. Earlier versions remain under **Earlier versions**.

## 3. Data-subject requests

1. **Customer submits.** Under **Your data requests**, the customer chooses *Get a copy*, *Correct a statement* or *Delete my statements*, adds optional details and submits. A signed-in session is the verification. The request shows *In review*.
2. **Officer processes.** The records officer opens **Records & retention**. Under **Data-subject requests** they choose **Process request**. Each statement version receives an item:

   | Request | Item outcomes |
   | --- | --- |
   | Copy | `INCLUDED`, or `ALREADY_DISPOSED`. The export manifest lists version identities and digests. |
   | Deletion | `DISPOSED`, `RESTRICTED_RETENTION`, `RESTRICTED_LEGAL_HOLD` or `ALREADY_DISPOSED`. A `CORE_JOURNAL` item is always `RESTRICTED_RETENTION`. |
   | Correction | `RESTRICTED_FINANCIAL_RECORD`. A real financial error goes through the [financial corrections runbook](finance-corrections.md), and the statement is reissued. |

3. **Customer sees the result.** The customer sees the overall outcome and, for each record, "Kept: retention period applies", "Kept: legal hold applies" and similar. Hold details are not shown to the customer.

## 4. Records staff

1. **Open the account.** Open **Records & retention** and enter the full account identifier. The table shows each month's versions, current, superseded or disposed state, closing, *Until* retention date and any *Legal hold* badge.
2. **Retrieve.** **Retrieve** consumes a workforce single-use link and shows the totals and content digest. It is recorded as workforce access evidence.
3. **Place a hold.** Choose the scope (*This account* or *Owner … (all accounts)*), then enter a reason and a case or matter reference. Choose **Place legal hold**. A retried submission with the same idempotency key returns the same hold.
4. **Request disposal.** Enter a reason and choose **Request disposal**. The result is one of:
   - *Disposal restricted: retention until YYYY-MM-DD.*
   - *Disposal restricted: legal hold.*
   - *Object disposed; disposal evidence retained.*

   After disposal the row shows *Disposed* with no figures. Retrieval and regeneration are refused, and the evidence digest is kept.
5. **Release a hold.** Sign in as a **different** records approver, open the same account and choose **Release hold** with a reason. A second release with the same reason by the same approver returns the original release.
6. **Review access evidence.** The table lists the last 50 grants, retrievals, denials (`ACTOR_MISMATCH`, `ALREADY_USED`, `EXPIRED`, `DISPOSED`), not-ready refusals, holds, releases and disposals. Records are append-only.

## 5. Verify and recover

| Situation | Action |
| --- | --- |
| Generation returns `NOT_READY` | Expected until the month ends and periodic finality completes. Nothing was published. |
| An object store or database error during generation | Nothing is published: version metadata and object share one transaction. Retry, or wait for the worker cycle. |
| A failure during disposal | No disposal evidence remains and the object is intact. Retry after fixing the cause. |
| Version `SOURCE_CHANGED` appears | An owner-path backdated fact entered a closed month. Investigate the Core journal. The earlier version remains for comparison. |
| More than 1,000 facts in a month | Generation is refused rather than truncated. Production needs a checkpoint design. |

Integration evidence: `make test-integration` runs the `statements::*` cases in `crates/documents/tests/cases/statements.rs` and the periodic-readiness assertion in `crates/finance/tests/cases/periodic_execution.rs`. The browser journey is the final test in `tests/browser-consumer/consumer.spec.ts`.
