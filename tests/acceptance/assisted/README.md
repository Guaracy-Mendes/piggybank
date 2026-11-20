# Assisted-channel acceptance (WP-34)

`assisted.py` runs against the real local issuer, API and worker on a fresh fixture database: `make test-assisted`.

| Result key | What is proven |
| --- | --- |
| `verification_method_enforced` | A verification method not permitted for the call center is refused (400). |
| `customer_authorization_required` | Before the customer authorizes in the app the agent reads nothing (403). Another customer cannot authorize. |
| `confirmed_operations_only` | The call-center policy lists no payment operation. Another customer's account cannot be frozen (404). |
| `common_restriction_applies_to_customer` | After the agent's protective freeze, the customer's own payment quote is refused. |
| `customer_ends_session` | The customer closes the session and the agent is refused at once. |
| `audit_staff_customer_channel` | Authorization audit records carry the staff subject and the represented customer, and the session records each decision with its reason. |

Database rules (handoff, forged represented party, channel policy, immutability) are covered by
`crates/core/tests/cases/partners.rs::assisted_sessions_need_customer_authorization_and_channel_policy`. The browser
journey is in `tests/browser-consumer/consumer.spec.ts`.
