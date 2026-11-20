# Event replay, data quality and monitoring runbook

This runbook covers WP-26 [operations monitoring](../implementation/operations-monitoring.md). TST has no monetary value. Rules, thresholds and message templates are synthetic fixtures.

## 1. Prepare

1. Start the environment ([local development](local-development.md), or `python3 scripts/local_cx.py prepare && python3 scripts/local_cx.py start`). Migrations must report schema 63. The worker runs the monitoring and notification loop every 5 s and the data-quality scan every 30 s.
2. Staff users from `scripts/seed.py`:

   | User | Roles | Use for |
   | --- | --- | --- |
   | `reviewer` | `case_reviewer` and `data_steward`, controls queue REVIEW | Scans, proposals, repairs, acknowledgements |
   | `approver` | `case_approver`, controls queue APPROVE | Independent approval of dispositions |
   | `security` | `operations_replay` | Channel outage drills and event replay |

3. Open **Operations monitor** (`#/operations`) in the staff portal.

## 2. Operations snapshot (FR-CTRL-05)

1. Choose **Record operations snapshot**. The tiles show:
   - payments awaiting controls or the provider
   - reconciliation breaks and recovery requests
   - pending and overdue checks
   - open, overdue and escalated cases
   - undelivered and quarantined events
   - open monitoring and data-quality alerts
2. Every count comes from one database snapshot. The digest identifies the stored snapshot, and overdue cases link to **Review queues**.

## 3. Data-quality alerts and replay (FR-DATA-05, AT-13)

| Alert | Action |
| --- | --- |
| *Missing financial event* | First check *Undelivered events* for `core`. If the relay is down, restore the worker; the event is delivered and the alert clears. If the event was delivered but the projection is still missing, choose **Repair projection** (data steward). Repair inserts read-model rows only, from the immutable journal, and never writes a balance. |
| *Quarantined event* | A conflicting identity or a version gap. Inspect it with the original event ID. Replay it through the existing controlled replay (`operations_replay`), or leave it quarantined with a case note. Never edit the projection by hand. |
| *Broken reference* | Replay the account or party events for that aggregate. |
| *Stale projection* | The watermark lag exceeds the objective. Restore the Data consumer and watch the lag fall. Customer spending decisions are unaffected because they use Core. |
| *Provider backlog* | Use **Payment inquiry** and recovery with the original instruction (see the [payment recovery runbook](payment-recovery.md)). Do not resubmit under a new identity. |
| *Aged reconciliation break* | Follow the [Finance reconciliation runbook](finance-reconciliation.md). |
| *Notification failed* | Restore the channel, contact the customer through support if needed, then acknowledge. |

When the next scan no longer finds the condition, the alert shows *Cleared*. Enter a resolution note and choose **Acknowledge** to close its case. An alert that is still open cannot be acknowledged.

## 4. Transaction monitoring (FR-CTRL-04, AT-30)

1. Alerts appear under **Transaction monitoring**, each with its rule, account, window, fact count and total. One alert covers one economic episode: duplicate or replayed events and repeated scans never create another.
2. To investigate retrospectively, enter **Scan from** and **Scan to** (31 days or less) and choose **Run retrospective scan**. The result reports how many facts were evaluated and how many alerts are new or existing.
3. As the reviewer, choose a disposition (*No further action* or *Refer to financial crime*), enter a rationale and choose **Propose disposition**.
4. As a different approver, choose **Approve disposition**. The case closes with both people recorded. The proposer cannot approve their own proposal.
5. Monitoring cases are never shown to the customer.

## 5. Customer messages

1. Customers see **Notifications → Messages** (subject, safe body and delivery status) and **Message preferences**. Security notices and statements & requests are always on. Payment activity and product announcements are optional.
2. **Outage drill (AT-20).** As `security`, enter a reason and choose **Mark unavailable** for *synthetic email*. New messages fail and retry with backoff (up to six attempts). Choose **Mark available** to resume: each message is delivered once. Messages that exhaust their attempts raise *Notification failed*.

## 6. Correlation trace (FR-OPS-02, AT-25)

Enter any operation, batch, payment, account or case identifier under **Trace an operation**. The trace counts linked journal batches, events in every module, cases, monitoring alerts, notifications and payment instructions. It uses durable identifiers only and contains no identity evidence. Use `GET /api/v1/staff/traces/{id}` for the full record.

Integration evidence: `crates/data/tests/cases/operations.rs`. The browser journey is the final test in `tests/browser-consumer/consumer.spec.ts`.
