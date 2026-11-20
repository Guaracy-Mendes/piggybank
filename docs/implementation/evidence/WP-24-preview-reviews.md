# WP-24 checkpoint: retained periodic preview reviews

Date: 2026-11-13. WP-24 remains **in progress**. This increment retains exact preview information with attributable customer review and native receipt recovery. It does not complete financial consent/enrollment, activation or periodic posting.

Six focused native cases passed: daily and exact large monthly material equivalence with V1 rendering independent of DateStyle; concurrent keys/canonical aliases with one original receipt/event; explicit digest/true acknowledgment/current-owner and narrow-grant checks; original retained material and retry recovery after live source corruption; actual lock-wait ownership revocation; and immutable/private-attribution integrity checks. Initial fixtures tried a cross-schema read with the Documents role and relied on restricted `pg_stat_activity` details; corrected fixtures use Core's own read role and database-scoped `pg_locks` without widening production grants.

The [specification](../periodic-preview-reviews.md), [runbook](../../runbooks/periodic-preview-reviews.md), [C4](../current-periodic-review-components.puml), [sequence](../current-periodic-review.puml) and [domain](../current-periodic-review-domain.puml) describe the exact boundary. All broader browser, regression and fresh release checks passed. Actual logs use `.local/reviews-*`.

## Executed verification

The real-issuer browser deliberately aborts an actual successful POST response after the native commit. Retrying the original retained key returns the same receipt UUID, reviewed clock and hashes, one request mapping and one original published Documents outbox event. It verifies initial GET absence, unchecked/disabled action, false/stale-material refusal, foreign-owner/workforce denial, private metadata redaction, saved URL/reload, mobile accessibility/overflow and exact large amount. Trusted fixture-only live policy corruption makes current inquiry fail while independently retained historical material remains valid. Subsequent private-attribution damage makes saved inquiry fail and clears previous receipt/material. Native positions/journals remain unchanged throughout. The mobile receipt was visually inspected.

| Check | Executed result |
| --- | --- |
| Formatting / locked Clippy / OpenAPI / generated client / web types and builds | Passed |
| Library tests | All 19 passed |
| Integration suite | All 119 across nine targets passed; 86 Finance cases |
| Focused review native cases | All six passed |
| Payment/Finance browser suite | All thirteen passed |
| Customer / staff / issuer browsers | All three independent flows passed |
| Focused review browser | Actual post-commit response loss and original receipt recovery passed |
| Foundation / identity / events / lifecycle HTTP | Passed persistent dependency restart, issuer readiness, PKCE/rotation/step-up, fenced event recovery and original native lifecycle/GL invariants |
| Ingress | All 114 implemented route patterns passed |
| Cloud configuration/archive checks | Six passed |
| Terraform format / validate / mock plan | Both providers valid; one mock plan passed each |
| PlantUML / documentation | All 53 sources rendered and syntax/XML checked; plan, links and traceability passed |

Migration 0053 is applied forward; all 53 persistent source checksums match. Earlier migration files and ten prior release/checksum pairs were preserved. Two failed disposable review fixtures were cleaned selectively after their setup/observability corrections; unrelated old fixtures were retained. Application/provider/browser fixtures were stopped, and developer dependencies remain available.

## Sealed deployment package

The exact `cloud-20261113-01` archive (84,460,279 bytes, SHA256 `36511fff59a8dbf086b952470310e277cdc529bafb6a2046da7a95e543c93706`) passed local acceptance with fresh installation credentials and its actual packaged images/deployment files. Nested image archive SHA256: `a843fd8a5f9b98d00b4fcbf84f15f0ccadd67c1dce92cc7faafde4bb33e646be`. Both preview feature gates passed actual OpenAPI, served bundle and channel checks alongside prior policy, correction, basis, audit and clearing acceptance. Full authenticated native behavior remains separately evidenced above. [Cloud validation](../../deployment/VALIDATION.md) records exact image IDs and historical seals.

## Remaining scope

Review acknowledgment is historical information, not financial consent. Legal/material terms publication where required, explicit financial acceptance/enrollment, activation and eligibility, authoritative economic-period finalization, guarded fee/accrual jobs and linked periodic corrections remain open. WP-24 remains in progress; no live AWS/GCP deployment or provider/production qualification was performed.
