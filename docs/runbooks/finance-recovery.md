# GL export recovery — synthetic local profile

The workforce **Finance & GL** view exposes the current export slice. Core journal entries remain the financial source. A completed Platform job, an accepted recovery command or an HTTP response alone does not establish GL booking. The [execution specification](../implementation/finance-execution.md) records the protocol, exact-money rules and the remaining WP-24 scope.

## Diagnose and recover

1. Open the original export. Record its Core source batch, operation, business date, chart ID/version, debit/credit control totals and provider journal reference. Both totals must match exactly. Keep the original export/source identities.
2. Inspect transport history and **Finance exceptions**. `SOURCE_MAPPING_MISSING` requires an effective independently approved chart with complete account mappings. `GL_EXPORT_PENDING` requires receipt inquiry or investigation of the retained conflicting packet. Follow the owned case in `finance-operations`; matching preserves the case for review.
3. For `REVIEW_REQUIRED`, an authorized Finance adjuster enters a specific reason and chooses **Recover original GL batch**. A `202` response confirms durable acceptance. If the response is lost, retry with the same UUID idempotency key and identical reason. The browser retains the key while the result is uncertain.
4. Keep the original source and mapping. Recovery first inquires; it replays only the exact sealed GL request after authenticated NOT_FOUND. Each request permits eight further transport attempts. A live transport lease must finish or expire before another sender acts. Earlier recovery generations are fenced.
5. Confirm BOOKED using the authenticated receipt, original provider reference and original exact totals. Confirm that Core balances, held funds, fees and original journal count are unchanged by GL recovery. Retain the owned case/audit trail and escalate unexplained conflicts.

Inquiry requires current Finance adjuster/approver authority plus the Finance REVIEW/APPROVE queue relationship. Recovery requires the current adjuster role and REVIEW relationship. Revoked membership denies inquiry and replay. Financial-case closure and direct journal/provider edits are not recovery actions.

## Local provider configuration and fault injection

The worker's `BANK_FINANCE_PROVIDER_BASE` defaults to `http://127.0.0.1:4182`. `BANK_FINANCE_SIGNING_KEY` has a separate synthetic local default and must agree between the worker and simulator. These endpoints bind loopback and operate only on synthetic TST. The local seed publishes the chart through the same independent maker/checker path as the service.

For a known original source batch, the simulator accepts `{sourceBatchId, mode}` at `POST /admin/gl-scenarios` with `X-Finance-Key`. ACCEPT books normally; LOST_RESPONSE commits before delaying the reply; BEFORE_ACCEPT_TIMEOUT delays without booking; CONFLICT signs a mismatching receipt while preserving the original provider booking. Changing the scenario to ACCEPT permits subsequent original-identity inquiry. This simulator administration is a test fixture, not a product money API.

| Check | Expected result |
| --- | --- |
| Repeated original export or concurrent discovery | One sealed export, one provider batch and original receipt |
| Lost response and provider restart | OUTCOME_UNKNOWN followed by inquiry of the same source and BOOKED receipt |
| Authenticated NOT_FOUND | Recorded observation followed by exact original replay; one GL effect |
| Altered source content or replacement export ID | Rejected; original provider batch remains intact |
| Wrong acknowledgment scope | REVIEW_REQUIRED, immutable quarantine observation and owned case; no claimed receipt |
| Expired transport lease | Original inquiry resumes under a greater fence; no new financial identity |
| Exhausted budget | Owned review; no silent new job identity or counter reset |
| Revoked Finance role/queue | Inquiry and idempotency replay denied |

## Completion checklist

- Original Core batch, export ID, sealed chart and request digests are preserved.
- Exact debit/credit totals balance; BOOKED has the original authenticated reference.
- One provider batch exists for the original source despite timeout/restart/replay.
- Recovery has a stable command/job identity, exact reason and retained audit/history.
- Core journals and customer/held/fee balances were not changed by GL export recovery.
- Remaining Finance breaks have an owner and next action; a matched observation does not erase the investigation.

Use `make test-integration` for disposable PostgreSQL/provider cases and `make test-payments` for the real issuer/customer/workforce browser flows. The [reporting/liquidity](finance-reporting.md) and [controlled revision](finance-revisions.md) runbooks cover immutable observations, independent cash and approved replacement links. Full multi-source reconciliation and accrual automation remain tracked by WP-24.
