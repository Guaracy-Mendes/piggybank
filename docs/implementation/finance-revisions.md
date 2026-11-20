# Controlled Finance report revisions

Current status: essential synthetic WP-24 is complete. This document retains the narrower increment it introduced; the [financial lifecycle](periodic-finance-r1.md), [scope decision](wp24-essential-scope.md) and [completion evidence](evidence/WP-24-complete.md) describe the completed execution and deliberate deferrals. Earlier references to remaining work below describe that checkpoint.

This WP-24 increment publishes an independently approved link between two existing immutable observation reports. Both artifacts, their source memberships, controls, captured proofs and digests remain unchanged. It builds on [Finance observations](finance-reporting.md). Broader instruction/provider/fee matching and periodic fee/accrual commands remain within WP-24.

## Scope and accepted command

A Finance adjuster with current REVIEW membership chooses the original and replacement report and supplies a bounded reason. Both reports must have the same UTC period, currency and format version. The replacement must be later by creation and source observation times and must retain every original Core batch. The service computes both full artifact digests, exact debit/credit/matched-total changes and source-count change. These details and the exact reason form the immutable approval scope.

The accepted request seals issuer, subject, client, request UUID, canonical maker, full details/digest and a 24 hour expiry before contacting Controls. Concurrent identical requests return one accepted ID. Reusing the key with changed reports or reason conflicts. Current maker authority is required even for acceptance replay. Artifacts are fully verified before staging and before publication.

Controls registration uses the same durable request ID under a distinct `FINANCE_REPORT` approval purpose. It binds the original report resource, exact details digest, canonical maker and expiry. A committed registration whose acknowledgment was lost is recovered through that same ID; it creates no duplicate proposal or proposal audit event. The generic internal registration port compares immutable identity/scope fields and stores timestamps at PostgreSQL microsecond precision.

Registration runs in bounded, persistent UUID pages. It records safe pending/error metadata and advances past temporarily unavailable or revoked makers. The cursor wraps to retry later; expired requests are excluded. A real database test places fifty revoked requests ahead of valid later work and verifies the later request registers on the next page.

## Approval and publication

| Action | Required current authority and guard |
| --- | --- |
| Inquiry | Finance adjuster/approver role and Finance REVIEW/APPROVE relationship |
| Propose | Finance adjuster role and Finance REVIEW relationship |
| Approve | Finance approver role and Finance APPROVE relationship; a different canonical human from the maker; exact displayed revision digest; unexpired request |
| Publish | Original canonical maker with current adjuster/REVIEW authority; approved exact purpose/resource/digest; unexpired proof and current independent checker grant/APPROVE relationship |
| Replay an existing publication | Current original-maker authority and the same sealed digest; recover the immutable published fact |

Controls rechecks the report checker's role and queue after locking the proposal. A login alias linked to the maker remains the same human and cannot approve the request. Another checker cannot claim an already recorded checker's approval. Chart or financial-correction proof purposes cannot authorize report publication, and report approval cannot authorize Core money correction.

Finance serializes publication on the original report. Database uniqueness permits one published successor per original and one predecessor per replacement. Strictly increasing artifact/source times prevent cycles; concurrent competing publications produce one winner. New corrections can continue the resulting linear chain using a later artifact. The publication receipt records both report IDs, exact scope digest, maker, checker and publication time in one Finance transaction with its durable audit intent.

Repeated publication returns the same receipt and publication time, including after a lost acknowledgment. Historical publication is preserved when approval later expires or grants change. New publication remains subject to current authority and proof checks. No revision action books Core money, changes a hold, remaps a GL export or rewrites either report. Existing historical snapshot issues remain historical observations.

## Interfaces and model

| Interface | Behavior |
| --- | --- |
| `GET /api/v1/staff/finance/report-revisions` | Most recent 100 accepted requests and derived registration/approval/publication state |
| `POST /api/v1/staff/finance/report-revisions` | `{originalReportId,replacementReportId,reason}` plus UUID idempotency key; durable acceptance returns `202` |
| `GET /api/v1/staff/finance/report-revisions/{id}` | Bound artifacts, deltas, digest, people, expiry and publication receipt |
| `POST /api/v1/staff/finance/report-revisions/{id}/approval` | `{detailsDigest}`; independent review of the exact accepted scope |
| `POST /api/v1/staff/finance/report-revisions/{id}/publication` | `{detailsDigest}`; original-maker publication or original receipt replay |

Report detail adds a current published-lineage wrapper. This wrapper is separate from the immutable format-version-1 report and its digest. It links the predecessor/successor and approved reason without changing captured financial evidence. The workforce view presents both artifacts/digests and exact deltas, requires the checker to confirm review, and exposes publication to the original maker after approval.

See the [revision sequence](current-finance-revisions.puml), [Finance components](current-finance-components.puml), [domain model](current-domain.puml) and [operating runbook](../runbooks/finance-revisions.md).
