# Payment recovery — synthetic local profile

This runbook operates the implemented SIMULATED/TST lifecycle. The workforce payment inquiry contains recovery history, original-instruction inquiry/cancellation commands and reconciliation observations. It does not provide financial override authority. The [payment execution specification](../implementation/payment-execution.md), [rail protocol](../../contracts/providers/payments/README.md) and [recovery sequence](../implementation/current-payment-recovery.puml) describe the underlying guarantees.

## Authority and durable identities

Reading payment/recovery/reconciliation records requires the current `payment_inquiry` role and REVIEW membership in `payments-operations`. Commands additionally require current `payment_operator` authority. The workforce form requires a bounded reason. An owned customer can request cancellation through payment details; another customer's payment/history remains unavailable. BFF mutations enforce origin and CSRF checks.

Every recovery command has an immutable request ID, original payment ID, action, requester, reason and payload fingerprint. The `Idempotency-Key` is scoped to issuer, subject and client. Retry an uncertain command with the same key and identical input. Changed input conflicts. A command is recorded before scheduling; a lost Platform acknowledgment recovers the original accepted job. Its status and append-only history remain separate from financial outcome.

The worker uses the original saved provider instruction, including its full input digest and authority window. It does not create another economic payment. Provider transport remains bounded to 1.5 seconds, with redirects and oversized replies rejected. Each durable job defaults to eight attempts. New operator inquiries have a per-payment 30 second cooldown and one active command of each kind. Exhaustion creates an owned payment case and retains uncertain funds. Another explicit audited action can continue recovery; increasing counters or recreating the payment is not a recovery action.

## Diagnose before acting

1. Open the original payment in workforce **Payment inquiry**. Record its payment ID, quote, workflow ID, Core booking receipt, exact principal/fee/total and financial status. Provider acknowledgment, accepted recovery and a completed workflow do not establish settlement.
2. Inspect **Payment reconciliation**, **Controlled payment recovery** and **Quarantined provider evidence**. Reconciliation is available independently when an inconsistent workflow prevents normal detail retrieval. Its financial snapshot is an observation; use current Core-backed inquiry to confirm recovery.
3. Follow the linked owned case. Claim it through the existing queue controls and retain original request/evidence/journal identities. Use the case escalation workflow when additional evidence or authority is needed. Reconciliation matching preserves the case and its history. Financial-case closure/override is not exposed by this increment.
4. Distinguish absence of evidence from definitive nonexecution. Timeout, NOT_FOUND, expired quote/authority, paused rail and exhausted retry budgets never prove a submitted instruction did not execute.

| Observed break | Safe next action | Required recovery evidence |
| --- | --- | --- |
| `SUBMITTED_OUTCOME_OVERDUE` | Inquire using the original instruction; request cancellation only when appropriate | Matching authenticated finality and Core settlement/release receipt |
| `PROVIDER_WORK_MISSING` | Inquire through the owning payment; recovery reconstructs immutable work from the submitted Core reservation and saved recipient | Same payment/instruction/digest; no new submit |
| `FINALITY_WORKFLOW_GAP` or `PROVIDER_FINALITY_UNBOOKED` | Resume the original recovery command; Core receipt lookup precedes new effects | Same immutable Core outcome and original journal/release identity |
| `SUBMITTED_HOLD_WITHOUT_FINALITY` or `BOOKING_WITHOUT_CORE_FINALITY` | Retain funds and investigate; original inquiry repairs the orchestration claim from financial evidence | Core and provider facts agree before a terminal business outcome is exposed |
| `CORE_RESERVATION_WITHOUT_INTENT` | Escalate with the original Core identity and restore the missing owning-domain data through controlled recovery | Verified original binding; no replacement payment or guessed instruction |
| `RESERVATION_SCOPE_MISMATCH`, `ACCOUNT_HOLD_TOTAL_MISMATCH`, `SETTLEMENT_FINANCIAL_FACT_MISMATCH`, `RELEASE_FINANCIAL_FACT_MISMATCH` or `HOLD_CHANGED_WITHOUT_FINALITY` | Keep the owned case open and escalate the financial inconsistency | Reconciled authoritative records and retained observation history |
| Conflicting or excess-return evidence | Investigate the immutable quarantine record and original-linked movements | Authentic exact-scope evidence; original journal and accepted principal cap preserved |

The selected local overdue threshold is five minutes from hold creation. Expected asynchronous gaps (new submitted work, newly received final evidence and new Core finality awaiting workflow acknowledgment) have a 30 second grace period. The reconciler rechecks Core finality after reading workflow state and defers a position that changed between the independent reads. The background worker scans bounded pages of 50 using persistent cursors for Core positions, payment continuations, credit continuations and recovery commands. It wraps each cursor after a full sweep, so old unresolved items do not starve later records. New mismatches and changed observations append history and durable audit. A subsequent matching observation becomes `MATCHED`; this neither releases money nor closes its Controls case. These thresholds are synthetic policy, not measured production SLAs.

## Inquiry and cancellation

The workforce form sends `POST /api/v1/staff/payments/{id}/recovery-requests` with `{action: "INQUIRE" | "CANCEL", reason}` and an idempotency header. Inquiry is permitted only for an original submitted external commitment. A customer uses `POST /api/v1/payments/{id}/cancellation`. Both return `202` for durable acceptance, not a final cancellation. Scoped GET recovery-history endpoints expose the accepted requests and current outcomes.

Before possible submission, cancellation serializes on the intent lock with execution. An existing bound unsubmitted reservation is released by Core; local cancellation does not call the provider. A crash after that release recovers by the same business key. Once Core has marked possible submission, cancellation uses the original authenticated provider cancellation protocol and the same fenced transport lease as inquiry/submission. An active transport attempt is allowed to finish before a later attempt takes its lease. A cancellation tombstone prevents a delayed submit of that identity from executing.

Only definitive authenticated CANCELLED evidence plus Core release permits a submitted payment to become CANCELLED. If the provider already settled, cancellation preserves settlement and the journal and reports that it was too late. It does not refund funds or release a consumed hold. Any later return is a separate principal-capped linked movement. Emergency pause blocks new submissions while existing inquiry, finality handling and cancellation continue.

## Completion checks

- The original payment, provider instruction and Core business identities are unchanged.
- SETTLED has the original valid Core booking receipt; CANCELLED/DECLINED on submitted work has definitive signed evidence and a Core release receipt.
- Exact booked, held and available balances agree with Core. Principal/fee and gross daily usage follow settlement/release rules; linked returns preserve the original fee and daily usage.
- Duplicate requests/evidence recover the same effect. No generic reversal, new economic payment or manual journal edit was used.
- Relevant observations are matched or remain owned and escalated with a next action. Case and financial histories are retained.

`make test-integration` exercises real disposable PostgreSQL records and the persistent synthetic provider, including lost acknowledgments, cancellation during transport, late cancellation, exhausted budgets, orphan/mismatch diagnostics and pagination. `make test-payments` exercises the real issuer and both browser clients. These checks do not certify an actual rail, settlement file format, Finance/GL process, cloud deployment or production capacity.
