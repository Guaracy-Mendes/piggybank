# Incoming and return reconciliation

This WP-24 increment observes incoming and returned principal from independent synthetic clearing files through authenticated movement evidence, Core suspense and customer allocation, original payment receipts and captured GL bookings. It has no money, hold-release, allocation, submission or GL-replay command. The [operator runbook](../runbooks/finance-credit-reconciliation.md), [provider contract](../../contracts/providers/finance/README.md), [C4 components](current-finance-components.puml) and [UML sequence](current-finance-credit-reconciliation.puml) describe the implemented boundaries.

## Separate immutable formats

`INCOMING_RETURNS_V1` has its own API, Finance tables, item format, private Core/Payments snapshots, independent provider file store and request kind. Existing outgoing `OUTGOING_PAYMENTS_V1`, FinancialBatch V1 and ReportSummary V1 serialization/digests remain intact. A captured run seals the parent report ID/digest, original signed file and complete digest, each source snapshot/digest and clock, stable UUID-sorted movement membership, exact controls and immutable classification codes. Reopening or idempotent replay preserves that evidence. New facts require a new run; a later report includes new journal/GL membership.

The current schema is migration 46. Creation/replay requires current Finance adjuster and REVIEW authority; independent Finance approvers can inquire. Customer and security roles do not supply Finance authority. Workforce BFF/CSRF/origin/quota controls apply. Authority is checked before capture and again inside the Finance commit boundary. Actor/client/request key identifies one exact `{reportId,fileId}` request; a changed scope returns 409. Unavailable, invalidly authenticated, bound or oversized files produce 503 with no saved run. Invalid period/page/source bounds fail explicitly.

## Read ports and financial matching

`FinanceRecords.credit_accounting` reads Core-owned receipts in a repeatable-read, read-only transaction under the Core principal. It includes movement/evidence identity, exact amount and digest, original signed packet, value date, target account, suspense receipt/date, optional allocation receipt/date/digest and source times. Return records additionally capture the original outgoing instruction/digest, payer, settled principal/journal/date, authenticated finality and lifetime returned principal sum. That sum includes all linked returns, including funds still in suspense and movements outside the requested report period. Core remains the authority that enforces the principal cap during booking.

The Payments FinanceReader independently captures immutable signed credit receipts, identity/digest, target, workflow state and observation times under its owning principal. Finance discovers Core identities, captures Payments, then confirms Core with explicit file/workflow IDs. The resulting sources have separate clocks; no distributed atomic snapshot is claimed. A newly discovered native booking whose prior-day provider identity was outside the original Payments capture is a visibility timing observation, rather than a fictional missing bank instruction. A workflow success flag cannot replace a native allocation receipt.

Matching verifies archived HMAC integrity and typed event scope, compares Core and Payments evidence identity/digest/amount/value date/destination, then checks independent provider rows for kind, original payment or receiving reference, evidence ID/version, principal, currency/scale, separate provider fee and value date. Original return evidence must prove a settled SIMULATED payment, the original payer, original instruction digest and principal, a return date at or after settlement, and a canonical positive lifetime return sum within original principal. The original outgoing bank fee is never refunded or charged by reconciliation.

Each movement has one suspense posting, `INBOUND_SUSPENSE` or `RETURN_SUSPENSE`, from clearing to suspense. An allocation is a separate `CREDIT_ALLOCATION` posting from suspense to the actual target customer account, linked to the suspense batch. Returns' suspense batches link to the original settled batch. Both captured postings must have exact amounts, accounts, operation/kind/original lineage, dates and balanced two-leg journals. Captured parent-report GL mapping/receipt classifications remain applicable independently for both postings. Extra or unlinked journals are exceptions.

## Dates, suspense and classification

Provider value date, provider `occurredAt`, file `generatedAt`, bank suspense/allocation business dates and source clocks have distinct meanings. Current Core books receipts on its actual UTC booking date. A valid earlier provider value date remains visible as `CREDIT_BOOKING_DATE_DIFFERENCE`; reconciliation never rewrites a journal date. A valid native batch absent from the parent report is `CORE_AFTER_REPORT`, or `CREDIT_OUTSIDE_REPORT_PERIOD` if its actual date is outside that period. A missing provider row is `SETTLED_AFTER_FILE` only when authenticated provider event time is later than the declared file cutoff; later bank processing alone cannot excuse a missing row. A prior-period provider value date booked in the current bank period is `CREDIT_VALUE_OUTSIDE_FILE_PERIOD` when absent from that provider-period file. A supplied row for an event occurring after the declared cutoff is `PROVIDER_EVENT_AFTER_FILE_CUTOFF`, an exception.

| Classification | Meaning |
| --- | --- |
| MATCHED | Original signed movement, independent file, native suspense/allocation and captured GL agree |
| SUSPENSE | Authenticated receipt is fully accounted but customer allocation remains unresolved |
| PENDING | An authenticated accepted workflow awaits native receipt, with no contradictory journal or terminal claim |
| TIMING | Only source visibility, booking/value date, file cutoff, GL grace or workflow acknowledgment differences remain |
| EXCEPTION | Evidence, original/cap, scope, amount, currency, fee, date, identity, duplicate, journal or GL conflict |

An authentic provider claim absent from both bank receipt sources remains in the union as an exception. A quarantined workflow without native receipt remains an exception; it does not become a pending or settled financial fact. Suspense always keeps completeness PARTIAL and acquires a Finance case. Classification counts are exclusive; an exception can also retain suspense principal. The staff list therefore displays actual native suspense principal independently of classification counts. No financial completion is implied for a customer while funds remain in suspense. EXCEPTIONS takes precedence over PARTIAL; COMPLETE applies only to this credit scope, including an explicitly empty period.

Pending processing has a five-minute ownership grace measured from the original workflow creation. Native allocation acknowledgment has a thirty-second grace; GL timing uses the captured report's existing grace. Frozen classifications never age into different results on read. Capture new observations to assess maturity. Durable differences acquire owned Finance cases; worker recovery processes fifty pending case links per pass with a wrapping cursor, advances past failures and reuses the original issue origin after a lost case acknowledgment. Refresh the view to retrieve current case links. Case disposition never changes the artifact.

## Exact controls, bounds and privacy

Per-movement amounts use canonical i64 decimal strings; aggregates use i128 decimal strings. Core received, allocated and still-in-suspense principal are distinct candidate-inventory controls. They describe the captured union, which includes explicitly identified out-of-period records; they are not a statement of period cash flow. Provider controls retain duplicates and partition by currency code and scale. The synthetic provider's explicit zero clearing fee policy treats any nonzero provider fee as an exception, independently of bank fees.

One file supports up to 100 rows and 60,000 canonical serialized bytes, with a signed response bounded to 64 KiB and a 1.5-second deadline. Each bank source/union supports at most 1,000 identities and the parent report at most 1,000 journal batches. Oversized sources fail rather than truncate results. Periods have a one-day minimum. [Grouped clearing](finance-group-reconciliation.md) raises provider capacity to ten files / 1,000 rows while retaining bank/report bounds; a single day exceeding 1,000 bank identities or 1,000 journal batches remains unsupported. Files are immutable: new ID returns 201, exact replay 200, same ID/changed content 409. No redirects or non-loopback transport are enabled.

Staff DTOs expose safe lineage and exact amounts, without HMAC packets. Run/item tables reject update/delete/truncate. Issue identity/code/membership is immutable with a one-time case link. Reads verify summary, private snapshot, signed file and parent hashes, complete item membership and row identities. Archived evidence uses signature integrity verification without imposing the original live transport freshness window.

## HTTP and UI

| Method | Route | Behavior |
| --- | --- | --- |
| GET / POST | `/api/v1/staff/finance/credit-reconciliations` | Latest 100 summaries, optional `reportId`; capture with UUID Idempotency-Key |
| GET | `/api/v1/staff/finance/credit-reconciliations/{id}` | Verified saved summary/digest and current ownership wrapper |
| GET | `/api/v1/staff/finance/credit-reconciliations/{id}/items` | Stable ordinal `after`, limit 1–100 |

The saved report view includes incoming/return capture, exact received/allocated/suspense controls, source clocks, currency-separated provider controls, owned cases, paged movement and original-payment/GL lineage. The URL `/finance/reports/{reportId}/credit-reconciliations/{runId}` reopens the saved run. An uncertain browser request retains its key in session storage. Read-only approvers do not receive capture controls.

[Grouped clearing sets](finance-group-reconciliation.md) now support up to ten files / 1,000 provider rows under unchanged bank/report limits. Remaining WP-24 work includes versioned periodic fee/accrual execution, calendars/cutoffs and independently approved linked financial corrections. Production file formats, actual operating policy, provider/cloud qualification and statutory accounting remain separate activation work.
