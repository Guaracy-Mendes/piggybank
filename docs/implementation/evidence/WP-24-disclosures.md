# WP-24 checkpoint: customer periodic rule disclosures

Date: 2026-11-12. WP-24 remains **in progress**. This increment adds customer-owned read-only information derived from exact approved preview rules. It does not implement customer acceptance/enrollment or financial activation.

Two pure renderer tests and five real PostgreSQL cases passed: exact large amount/text/hash stability; owner/privacy/unchanged money; foreign/staff/service/revoked denial; unpublished and wrong-product hiding plus native policy tampering refusal; current/upcoming/ended version retention; and native publication of 100/101 rules with refusal of truncation. An initial fixture incorrectly republished the same product interval and correctly hit the existing overlap guard; the corrected fixture independently publishes another product/policy instead of changing financial facts.

The focused real-issuer customer browser passed exact large-amount rendering and independent canonical SHA256 recomputation, current/upcoming policy versions, unchanged original material, private workforce metadata exclusion, foreign-owner and workforce denial, mutation refusal, saved-link reload and fresh-login return, native policy tampering refusal, cleared stale material and unchanged financial facts. Desktop/mobile accessibility and horizontal layout checks passed; the mobile material capture was visually inspected.

The first browser fixture opened an account through the API after the page had loaded an empty account list; reloading that inventory fixes the fixture setup. A subsequent browser failure exposed an actual duplicate React sibling key shared by disclosure and history components. The disclosure key now has its own prefix. The final focused run verifies exactly one disclosure panel after reload and mobile resizing. Before-fix logs remain under `.local/disclosures-browser-focused*`.

[Specification](../customer-periodic-disclosures.md), [runbook](../../runbooks/customer-periodic-disclosures.md), [C4](../current-periodic-disclosures.puml) and [sequence](../current-periodic-disclosure-read.puml) describe the boundary. All broader native/browser/archive checks passed. Actual logs use `.local/disclosures-*`. No migration is added; schema 52 remains immutable.

## Executed verification

| Check | Result |
| --- | --- |
| `make app-check` and final web checks/builds | Locked Clippy with warnings denied, formatting, implemented OpenAPI, generated client and all web types/builds passed |
| `make test-unit` | All 19 library cases passed, including two pure disclosure render/hash cases |
| `make test-integration` | All 113 cases across nine targets passed, including 80 Finance cases |
| Focused native disclosure cases | All five passed, including actual approved 100/101-policy publication, owner revocation and exact saved ended-version reads |
| `make test-payments` | All twelve real-issuer payment/Finance flows passed |
| Independent customer/staff/issuer browsers | All three regressions passed |
| Focused disclosure browser | Passed saved URL/re-login, stable independently recomputed digest, exact large value, role/privacy/integrity checks, mobile accessibility and one-panel regression |
| Foundation/identity/events/lifecycle HTTP | Persistent dependency restart, real PKCE/rotation/step-up, fenced event recovery and original native lifecycle/GL invariants passed |
| Ingress HTTP | All 113 implemented patterns passed safe-error/body/quota/spoof checks |
| Cloud configuration/archive tests | Six passed, including malformed feature metadata refusal |
| Terraform format/validate/mock test | GCP and AWS valid; one mock plan passed per cloud; no resources applied |
| PlantUML/documents | All 50 sources rendered and syntax/XML checked; documentation, plan and traceability checks passed |

The fixture-only integrity test changes a policy document without its native digest; inquiry refuses it and clears displayed material. It never changes account positions or journal facts. Customer reads and policy publication leave native money unchanged. The 101-policy test refuses list truncation while preserving direct known-version inquiry. All applied migration checksums and earlier archive seals were independently verified.

## Deployment refresh

The exact schema-52 `cloud-20261112-02` archive (84,374,476 bytes, SHA256 `92d2354c50539b4bf3429640cc35712d7f966fd2585a555e63f89c9a48dad611`) passed local sealed-image acceptance using fresh installation credentials and its actual packaged deployment files. Its manifest declares `customer-periodic-disclosures-v1`; actual embedded OpenAPI, served customer bundle and both channel guards passed alongside all prior policy/basis/correction/audit/clearing checks. Nested images SHA256: `711dbc0691e57286fd532280564fb243adbe995224ebde8b1b4a8a10fb03d7ba`. [Cloud validation](../../deployment/VALIDATION.md) records exact image IDs and historical seals.

Both [GCP](../../deployment/gcp/RUNBOOK.md) and [AWS](../../deployment/aws/RUNBOOK.md) runbooks/examples use the new immutable release. No live cloud account or provider qualification was used. The original schema-52 audit archive remains unchanged and valid without the newer feature declaration.

## Remaining scope

Customer disclosure is read-only product information. Attributable acceptance/enrollment, durable legal/material document controls where required, financial activation, authoritative economic-period uniqueness/finalization, native fee/accrual jobs and counter-account mappings, and periodic corrections remain open. WP-24 stays in progress.
