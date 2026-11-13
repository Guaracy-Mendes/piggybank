# WP-24 checkpoint: durable Finance audit delivery

Date: 2026-11-12. WP-24 remains **in progress**. This increment implements durable audit progress and native receipt-based acknowledgment. Periodic enrollment, economic finalization and financial posting remain open.

The initial real PostgreSQL regression demonstrated that a valid event never progressed past fifty malformed pending records. The new seven-case focused suite passed durable progress/retention; original native receipt recovery with append permission temporarily revoked; same-ID/different-stream and invalid-record isolation; forged flags/receipt/attempt protection and native-link damage rejection; preserving noncanonical raw records; forward legacy upgrade with valid evidence and transactional refusal without it; and concurrent workers with exact native deduplication plus late lower-ID cursor wrapping.

Migration 0052 is applied locally; applied migrations 0001–0052 are immutable. [Specification](../finance-audit-delivery.md), [runbook](../../runbooks/finance-audit-delivery.md), [C4](../current-finance-components.puml), [sequence](../current-finance-audit-delivery.puml) and [domain UML](../current-finance-audit-domain.puml) describe the boundary. All broader checks, staff browser and fresh archive acceptance passed. Actual logs use `.local/audit-delivery-*`.

## Executed verification

The before-fix regression failed because a valid event never advanced past fifty malformed records. All seven final native cases passed after the fix, including concurrent receipt/status reads and a late zero-UUID event at the wrapping boundary. A test temporarily revokes Controls append permission after its native commit to prove receipt recovery uses independent evidence. Forward-upgrade tests create actual schema-51 databases, preserve original delivery time with matching native evidence and roll back schema-52 migration when that evidence is missing.

The real-issuer browser passed 51 retained malformed events, independent native recovery of a later valid event, one native event per original ID, unchanged Core positions/journals, safe response redaction, saved URLs and read-only checker access, customer/security denial, no mutation endpoint, receipt confirmation without optional attempt history, mobile/desktop layout and accessibility. A trusted fixture-only receipt-link corruption returns 409 and clears prior delivered UI proof. Native and browser corruption tests never rewrite financial facts. The mobile native-receipt capture was visually inspected. All 52 persistent migration checksums and prior release seals were independently rechecked.

| Executed check | Actual result |
| --- | --- |
| `make migrate` | Forward migration 0052 applied; migrations 0001–0052 are immutable |
| `make app-check` plus final web build | Formatting, locked Clippy with warnings denied, real OpenAPI validation, generated client, all web types and both portal builds passed |
| `make test-unit` | All 17 library cases passed |
| `make test-integration` | All 108 cases across nine targets passed, including 75 Finance cases |
| Focused latest native audit suite | All seven passed after the final consistent-read and zero-UUID boundary changes |
| `make test-payments` | All eleven real-issuer payment/Finance flows passed |
| `make test-customer`, `make test-staff`, `make test-browser` | All three independent browser regressions passed |
| Foundation / identity / events / lifecycle HTTP | Passed real restart/readiness, issuer/refresh/reauthentication, durable events and stable original journey/GL identity |
| `make test-ingress` | All 111 implemented patterns passed contract/safe-error/body/quota/address-spoof checks |
| `make cloud-check` | Six configuration/archive checks passed |
| Project-local Terraform fmt / validate / test | GCP and AWS valid; one mock plan passed per cloud; no resources applied |
| Diagram rendering / `make check` | All 48 rendered, XML-validated and syntax checked; plan/catalog/local references resolve |

## Deployment refresh

The sealed schema-52 `cloud-20261112-01` archive (84,334,644 bytes, SHA256 `6940b976458323249bcc9aa987a2e07664f65303349e3a09af7b767726063109`) passed actual archive/image acceptance with fresh installation credentials and its exact packaged deployment files. Nested image SHA256: `8c559118aa91685fe85520c8768549ca8ce85de14fcddc3e6b79521f8c2d052b`. The native audit channel/compiled-portal guards passed alongside prior policy, basis, correction and clearing checks. This image smoke does not claim the full authenticated workflow; that was tested separately by the host browser and PostgreSQL cases above.

Both complete cloud runbooks and Terraform examples point to the fresh identifier. Previous sealed archives remain unchanged. Exact image IDs are retained in the [cloud validation record](../../deployment/VALIDATION.md). Actual build/runtime/check logs use `.local/audit-delivery-*`; screenshots use `.local/finance-audit-*.png`. All disposable application/image fixtures were removed and developer dependencies retained. No live cloud deployment was performed.

## Remaining scope

Audit delivery receipts establish acceptance into the native Controls audit ledger. Financial completion remains authoritative only through Core's original receipts. Periodic enrollment/material terms, authoritative economic uniqueness/finalization, fee/accrual posting jobs/counter-account mappings and corresponding periodic corrections remain unfinished in WP-24.
