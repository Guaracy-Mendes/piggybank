# Periodic finance R1 operator runbook

Use this runbook for the [essential synthetic scope](../implementation/wp24-essential-scope.md). TST has no monetary value. The [execution specification](../implementation/periodic-finance-r1.md) explains native authority and financial evidence.

## 1. Prepare a running environment

1. Follow [local development](local-development.md) to start dependencies, apply migrations, provision identities, seed approved products/chart and run the API/worker/provider/customer/staff portals. `make migrate` must show schema 56, with the prior applied checksums unchanged.
2. Run `make dev-seed` with the documented workforce maker/checker subjects. The seed preserves chart v1 and independently publishes chart v2, including explicit fee revenue `4f1a780b-3e31-48ef-a294-000000000004` and interest expense `4f1a780b-3e31-48ef-a294-000000000005`.
3. Keep the worker and persistent provider running. Confirm API readiness and provider health. For cloud-hosted synthetic evaluation, use the complete [GCP](../deployment/gcp/RUNBOOK.md) or [AWS](../deployment/aws/RUNBOOK.md) installation runbook first; then use the same portal steps below through the authorized tunnel.
4. Sign in to operations in two separate browser profiles: an authorized Finance maker with REVIEW and a different canonical Finance checker with APPROVE. Sign the customer into the separate customer portal. A second issuer alias for the same person is not an independent approver.

## 2. Publish a calculation and financial terms

1. Open **Periodic policies**. Supply the exact published product version ID, unique rule code/version, effective dates, UTC calendar/weekdays/holidays/cutoff, fixed monthly minor units or daily annual basis points, and reason.
2. Save the preview policy request. The checker reviews its complete rule/calendar/digest and chooses **Approve preview policy**. The original maker chooses **Publish preview policy**. Record the native policy ID. The preview registry remains PREVIEW_ONLY.
3. Open **Periodic finance**. Enter the published policy ID and financial terms reason under **Prepare financial enrollment**. Choose **Stage financial terms** and retain its saved URL.
4. In the checker's profile open that URL. Review the generated financial text, full policy/calendar, material digest, product/effective period and reason. Explicitly check the review box, then choose **Approve financial terms**.
5. The original maker refreshes and chooses **Publish financial offer**. Retain the offer ID and exact digest. Publication alone changes no balance and enrolls no account.

## 3. Obtain explicit customer consent

1. In the customer portal open the owned active account, then **Periodic financial offers**. Open the financial offer and review its complete text, exact amount/rate, effective period, calendar and digest.
2. The owner explicitly checks the acceptance box and chooses **Request financial enrollment**. An unchecked box must disable the action. Reading terms or saving a preview acknowledgment is insufficient.
3. Confirm `PENDING_ACTIVATION`, the consent ID/digest, native accepted time and earliest whole period. Copy the consent reference to the authorized Finance workflow. Check booked/held/available balances: consent has no financial effect.
4. To cancel before activation, choose **Withdraw pending enrollment**. Confirm WITHDRAWN and retained original receipt. The same offer cannot be used to replace withdrawn consent.
5. If the response is interrupted, choose **Refresh financial offers** or reopen the saved URL. Recover the original consent and current status. Do not infer failure from a network timeout or use a different displayed digest.

## 4. Independently approve and activate

1. As maker, enter the consent ID and explicit counter-account ID in **Prepare account activation**; choose **Stage account activation**. Use revenue control for a fee or expense control for interest. Core requires a current approved exact account mapping.
2. The independent checker opens the activation URL and reviews the accepted financial terms, account, consent digest, counter account and activation digest. Check the review box and choose **Approve account activation**.
3. The original current maker refreshes and chooses **Activate periodic finance**. Confirm ACTIVE and the first whole UTC period. A changed owner/product/restriction, expired authority, withdrawn consent or mismatched mapping must refuse activation.
4. Refresh the customer offer. It must show ACTIVE/ENABLED and the same consent reference; pending withdrawal is no longer offered. No current partial period is charged.

## 5. Complete a due period

1. Keep the worker running until the first complete period closes and the approved UTC cutoff/business-day rule is due. The worker discovers due active enrollments automatically while the original maker retains current authority.
2. In **Periodic finance**, refresh **Period runs**. A queued operation is not financial completion. Inspect the native finalization, original source facts/digest, exact amount, posted flag and actual Core batch reference.
3. A positive fee/accrual must have one balanced journal. A zero accrual must show POSTED, amount `0`, and no batch. Monthly fees respect available funds/holds; daily interest respects credit restrictions and native booked basis.
4. Open the actual batch in **Finance & GL**. Wait for BOOKED with authenticated simulator acknowledgment, unique source reference and exact debit/credit totals. Repeated exports must recover the original GL receipt.
5. In the customer's account, refresh balance/history. Confirm the native fee or interest movement. Reconciliation/report capture remains an observation and does not post money.
6. Never edit financial facts or replace `core.periodic_clock` in a deployment. Only isolated automated test databases use a migration-owner controlled clock to exercise closed dates without waiting months. There is no API clock override.

## 6. Recover exceptions and interrupted jobs

| Condition | Action and expected proof |
| --- | --- |
| Intent committed, scheduling response lost | Leave the worker/recovery scan running. Its wrapping cursor schedules the original intent. Reopen the run and verify its original operation/native receipt. |
| Duplicate runs / lost posting acknowledgment | Inspect native receipt by activation/period. Concurrent jobs must share one batch; balances must not change again. |
| Insufficient available funds or hold/restriction | Inspect the owned Finance case. Resolve the actual cause through authorized controls. Keep the original finalized basis. If bounded retries are exhausted, queue that activation and the same closed period with a fresh manual request; the native economic identity stays unchanged. |
| Maker revoked, ownership changed or source damaged | Preserve the exception and receipts. Restore only legitimately authorized state or stop the enrollment. Do not alter stored actors, source hashes, consent or period dates. |
| More than 1,000 source facts | Finalization refuses. Escalate for the deferred scale/checkpoint design; do not truncate history or supply a current balance. |
| Provider receipt arrives after a period closed | Preserve provider value-date evidence and the actual current bank booking date. Capture a new report/reconciliation observation; never backdate into closed facts. |
| GL response unknown / conflicting | Follow the existing [GL runbook](finance-recovery.md) for original inquiry and owned recovery. Do not create a replacement source journal. |

For manual resumption, enter **Activation ID** and **Closed period start**, then **Queue due period**. Only the next sequential closed due period is allowed. Preserve the Idempotency-Key across an unanswered request; after an acknowledged command, a later explicit retry can use a fresh key. Job state never proves financial completion.

## 7. Stop or correct

1. To stop future effects, open the activation, supply a reason and choose **Stop periodic finance**. Confirm PAUSED in both portals. This scope has no resume command. Existing financial receipts remain historical facts.
2. To reverse a mistaken original fee/accrual, open **Financial corrections**, enter its original native Core journal ID and reason, then save the request.
3. Independently review/approve the exact source/legs/digest, then explicitly execute as the original current maker. Confirm a new `CORRECTION` batch linked to the original and one native correction receipt. Existing holds/restrictions still apply; an interest reversal may require available funds.
4. Confirm separate original/correction GL exports and updated customer history. Repeated execution must recover the same reversal. The original period digest and source facts must remain unchanged. Reversals affect later basis/report observations, not past finalizations.

## 8. Reproduce acceptance

Run shared-database suites sequentially:

```sh
make app-check
make test-unit
make test-integration
make test-payments
make test-customer
make test-staff
make test-browser
make test-foundation
make test-identity
make test-events
make test-lifecycle
make test-ingress
make web-build
make cloud-check
make check
```

The financial browser flow uses real local issuer sessions, a disposable database, actual worker/provider/GL and native money. Focused enrollment/execution cases live in `crates/finance/tests/cases/enrollment.rs` and `periodic_execution.rs`. See the [completion evidence](../implementation/evidence/WP-24-complete.md) for the recorded results and remaining qualification boundaries.
