# WP-24 progress — Independently approved report revisions

Verified locally on 2026-11-01. **WP-24 remains in progress.** This increment implements controlled replacement links between immutable Finance reports. Full instruction/provider/fee reconciliation and versioned periodic fee/accrual commands remain open.

Acceptance seals both verified artifact IDs/digests, exact reason, UTC period/currency, canonical maker, control deltas, requester/client/key and a fixed 24 hour expiry. The replacement must be later and retain every original source. Concurrent identical acceptance returns one ID; changed input conflicts. Both reports retain their original digests, captured evidence and membership.

Finance commits acceptance before Controls registration. The distinct FINANCE_REPORT purpose registers under that same durable ID, binding the original resource, exact digest, maker and expiry. Exact replay uses PostgreSQL microsecond timestamp precision. A lost registration acknowledgment recovers the original proposal without duplicating its audit event. The wrapping cursor advances past blocked makers; the database test fills its first page with fifty revoked requests and verifies later eligible work registers on the next page.

Approval requires the current Finance approver role, APPROVE queue relationship, exact displayed digest and a different canonical human. Controls rechecks report approval authority after locking the proposal. A second login alias for the maker cannot approve; report proof cannot authorize a Core correction. Publication requires the original canonical maker and current maker/checker authority, expiry and exact scope. It serializes on the original report and commits one immutable predecessor/successor receipt plus audit intent. Unique edges and increasing artifact/source times prevent forks, merges and cycles. Publication replay returns the same original publication time.

Published lineage is a report-detail wrapper separate from the frozen format-version-1 artifact. The staff view shows both reports/digests and exact deltas, requires independent review confirmation and exposes publication to the original maker after approval. The first browser run found unstable names on the new selectors; explicit label associations fixed them. The complete rerun passed with separate real issuer reviewer/approver sessions, self-approval and premature-publication rejection, exact-scope approval, publication replay, retained lineage and unchanged Core money. Mobile accessibility/overflow checks passed; the published-lineage screenshot was inspected.

| Local verification | Result |
| --- | --- |
| `make test-integration` | All 48 workspace integration cases passed, including 16 Finance cases |
| New revision database coverage | Concurrent acceptance/publication, aliases, lost registration/publication ACKs, wrong-purpose proof, exact scope, current grant/queue revocation, expiry, fork/cycle/cross-period guards, other-maker rejection, immutable artifacts and pagination passed |
| `make test-payments` | All four real issuer/browser payment and Finance flows passed after the selector-label fix |
| `make test-ingress` | 88 actual route patterns passed authentication/error, size, shared-quota and forwarded-address checks |
| `make test-customer test-staff test-unit` | Customer/staff regressions and eight library unit cases passed |
| `make app-check` | Warning-free Rust, actual OpenAPI route/method/client drift, real schema validation and strict TypeScript passed |
| `make web-build` | Both browser clients built successfully |
| `make diagrams` | 35 SVG views rendered and XML checked: 22 target and 13 implementation/historical models |
| `make migrate` | Migration 0044 applied to the persistent local database; earlier migrations remain unchanged |

Local dependencies were stopped at session start. The initial database attempt could not connect; Piggybank project services were started before the successful real database runs. No financial assertion was established by the disconnected attempt.

Logs: `.local/wp24-revisions-integration.log`, `.local/wp24-revisions-database-live.log`, `.local/wp24-revisions-verified-check.log`, `.local/wp24-revisions-verified-browser.log`, `.local/wp24-revisions-verified-ingress.log`, `.local/wp24-revisions-regression.log`, `.local/wp24-revisions-web-build.log`, `.local/wp24-revisions-diagrams.log` and `.local/wp24-revisions-migrate.log`. Screenshots `.local/finance-report-workbench.png` and `.local/finance-report-mobile.png` include the published revision and retained lineage.

See the [specification](../finance-revisions.md), [UML sequence](../current-finance-revisions.puml), [C4 components](../current-finance-components.puml), [domain model](../current-domain.puml) and [runbook](../../runbooks/finance-revisions.md). These links approve report metadata within the synthetic system; they do not execute financial adjustments, certify statutory reports or verify real providers/cloud/production operation.
