# On-call runbook

1. **Roster.** One active **primary** and one **secondary** (local seed: `reviewer` and `approver`). Change it with `POST /api/v1/staff/oncall`, which needs `operations_replay` or `security_admin`. Every change is recorded.
2. **Paging.** High-severity data-quality alerts, open transaction-monitoring alerts and escalated overdue cases page the primary within the 60-second objective (synthetic pager capture). Pages show in **Operations monitor → On-call pages**, with delivery latency.
3. **Acknowledge.** Within 5 minutes, choose **Acknowledge page**. Only a paged person can acknowledge. An unacknowledged page escalates once to the secondary.
4. **Act.** Follow the alert's next action: the [event replay and data-quality runbook](event-replay.md), the monitoring disposition, or case escalation. For a security incident use the [security incident runbook](security-incident.md).
5. **Watch metrics.** `bank_pages_unacknowledged`, `bank_oldest_unacknowledged_page_seconds`, `bank_projection_lag_seconds`, `bank_outbox_oldest_age_seconds` and `bank_unbalanced_journal_batches` (must be 0) at `/metrics`, with the bearer token.
