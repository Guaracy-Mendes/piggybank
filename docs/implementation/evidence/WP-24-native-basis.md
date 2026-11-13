# WP-24 checkpoint: native periodic ledger basis observations

Date: 2026-11-12 (America/Porto_Velho). Implementation and host checks began November 11; this checkpoint was finalized November 12. WP-24 remains **in progress**. Native observations and calculation previews are implemented; customer enrollment, authoritative economic finalization and periodic financial execution remain separate work.

## Implemented behavior

- Forward migration 0051 creates immutable Core basis evidence and guarded actor-key capture, plus a persisted Finance audit cursor. It is applied locally; migrations 0001–0051 are immutable.
- Exact published policy/account-product binding and frozen finite UTC calendar determine period and due date. No operator balance or evaluation clock is accepted.
- Account metadata/current position and committed native account journal prefix share one native statement MVCC view. Business-date opening/change/closing use exact i128 strings; current booked/held/available remain separate i64 strings.
- The entire prefix, including opening history, is limited to 1,000 facts. The 1,001st aborts without a partial artifact. Source items preserve original journal/operation/date/time/fingerprint/account delta and stable ordinal pages.
- Original actor/client/key replay retains native clock, facts, totals and digests. Fresh keys observe later commits without changing prior artifacts. Ended periods remain observations rather than global book close or authoritative economic finalization.
- Derived native-source previews retain the captured clock and explicit provisional status. Negative/out-of-i64 daily basis or account registration after period start suppresses calculation. No value is clamped or replaced by current booked balance.
- Every read verifies raw/typed/source/policy hashes and scope/totals, then independently checks saved native journal facts. Rehashed false stored amounts cannot become financial evidence.
- Current maker/canonical identity/REVIEW authority is rechecked after the native key lock. Audit uses the original basis UUID/person/time; fifty-UUID wrapping recovery repairs missing acknowledgment past damaged records without monetary work.
- The existing policy workbench now includes capture/retry/fresh actions, saved URLs bound to exact policy, source paging, historical versus current positions, exact-string money and UTC clocks. Checkers are read-only. Three new route patterns/four operations bring the implemented inventory to 109.

See [specification](../periodic-native-basis.md), [runbook](../../runbooks/periodic-native-basis.md), [C4](../current-finance-components.puml), [sequence](../current-periodic-native-basis.puml) and [domain UML](../current-periodic-basis-domain.puml).

## Executed verification

All nine focused PostgreSQL cases passed: exact large amounts/holds/holiday due/replay; ended history versus current position; exact scope/role/queue/page guards; rehashed source tampering; 1,000/1,001 source bounds; concurrent actual financial commits; lost audit acknowledgment beyond fifty damaged artifacts; authority expiry during a real key-lock wait; and closed whole-month fee calculation without posting. Account registration dates are fixture metadata only; financial facts are generated through native Core commands and are not backdated or rewritten.

The focused real-issuer browser flow passed original capture/replay, unchanged saved sources after later native funding, fresh observation including the new journal, stable source pages and exact native fingerprints, ended-period zero history versus positive current position, registration-based calculation suppression, role/CSRF/unknown-field/key/page guards, saved URL, checker read-only access, mobile overflow and accessibility. The mobile screenshot was visually inspected. A deliberately held authentic post-commit response reproduced navigation back into a policy after leaving the screen. Invalidating pending work on unmount fixed it; both final policy and native-basis browser regressions passed. Before/after logs are retained separately.

A further November 12 browser regression aborts an authentic successful capture response after the backend commits. The UI shows no unacknowledged observation as captured; retry preserves the original browser key and returns the complete original basis, native clock and digest. Exactly one Core basis and one stable Finance audit event remain, with unchanged journals and account positions. That full native-basis flow passed again, including the prior delayed-navigation regression. Actual log: `.local/basis-browser-lost-response.log`. Application code/images were unchanged, so the existing sealed schema-51 release remains the tested artifact.

A PostgreSQL restart initially exposed stale issuer database connections despite cached OIDC discovery. Foundation acceptance now waits for fresh customer and workforce authorization forms with the configured clients before starting dependent tests. Initial failures and an accidentally overlapping retry were retained separately; the final sequence ran serially. No financial command was retried to address identity readiness. The older lifecycle test also assumed three jobs globally; it now verifies the original three journey commands and stable job identities across restart while independently requiring all three native funding exports to be uniquely booked in GL.

| Executed check | Actual result |
| --- | --- |
| `make app-check` | Formatting, locked Clippy with warnings denied, real OpenAPI validation, generated-client drift and all web TypeScript checks passed |
| `make test-unit` | All 17 library cases passed |
| `make test-integration` | All 101 cases across nine targets passed, including 68 Finance cases |
| Focused native-basis browser flow | Passed authenticated capture/replay/fresh observation, native provenance, role boundaries, mobile and accessibility |
| `make test-payments` | All ten real-issuer payment/Finance flows passed |
| `make test-customer`, `make test-staff`, `make test-browser` | All three independent browser regressions passed |
| Foundation / identity / event / lifecycle HTTP checks | Passed persistent restart/degraded readiness and existing authority/event/journey acceptance |
| `make test-ingress` | All 109 patterns passed contract/safe-error/body/quota/address-spoof checks |
| `make web-build` | Both compiled portals passed |
| `make cloud-check` | Six configuration/archive checks passed |
| Project-local Terraform fmt / validate / test | GCP and AWS valid; one mock plan passed in each cloud, with no resources applied |
| Diagram rendering / `make check` | All 46 rendered, XML-validated and syntax-checked; plan/catalog/local links resolve |

## Deployment refresh

The actual schema-51 `cloud-20261111-01` archive (84,308,685 bytes, SHA256 `02acb63d09a22865dc1a3f4308db7b436f3e2d1291398d56c15dfa546d34461b`) passed checksum-verified packaged-runtime acceptance using randomized installation credentials and its exact archived images/deployment code. The nested image archive SHA256 is `7072318fbaf504ef7cbeb293cebe49fcd331fe0c9b293957dc3e31f8d545d22a`. Native-basis customer/workforce channel guards and actual served compiled controls were checked alongside both signed clearing namespaces, two closed groups, identity/bootstrap/readiness/reseed and packaged funding CLI usage without monetary posting. The full authenticated basis workflow was exercised separately by host browsers and native tests.

Both complete cloud runbooks and Terraform examples use the new identifier. Earlier sealed releases were preserved. The [cloud validation record](../../deployment/VALIDATION.md) retains exact image IDs and historical seals. All disposable app/image-test resources were removed; developer dependencies remain available. Actual logs use `.local/basis-*`, with desktop/mobile captures `.local/finance-basis-*.png`. No live cloud deployment was performed.

## Remaining scope

Captures never enroll accounts, establish customer consent or an amount owed, post fees/interest, consume/release holds or mark an economic period financially complete. WP-24 still requires material customer terms/enrollment, authoritative economic uniqueness/finalization, durable posting jobs, accepted native fee/accrual/counter-account commands and corresponding periodic correction eligibility. Funding-only corrections remain separate. No live GCP/AWS apply or production/provider qualification is claimed.
