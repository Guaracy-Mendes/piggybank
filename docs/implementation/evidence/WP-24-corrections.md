# WP-24 checkpoint: controlled synthetic funding corrections

Date: 2026-11-10. Package status: **in progress**. The new financial capability supports full reversal of original synthetic funding only. Periodic posting and periodic-journal corrections remain open.

## Implemented behavior

- A separate narrow Core CorrectionCommands capability accepts only original balanced SYNTHETIC_FUNDING with positive CUSTOMER / negative SETTLEMENT legs. FinancialBatch/ReportSummary V1 and original journals remain unchanged; reporting/reconciliation remain observation-only.
- Finance persists original actor/client/key, complete source, native fingerprint, typed source hash, exact original/reversing legs, reason, execution UUID, material digest and fixed expiry before Controls registration.
- FINANCE_CORRECTION is independent of configuration, report, chart and legacy correction purposes. Current canonical maker/checker, queue/grants, exact scope and expiry are enforced. Registration recovers in wrapping pages of fifty past failed makers and never executes money automatically.
- The guarded native command holds original-source and ordered account locks before checking live authority. It respects availability after held funds and account restrictions, rejects a second correction of the original, and atomically commits balanced linked journal, native receipt and Core outbox. UTC-midnight-spanning attempts roll back for original-reference retry.
- Lost execution responses recover from the exact original native receipt. Competing approved requests yield one financial winner. Native readback checks actual journal/operation/original/legs; damaged receipt references cannot claim completion. Historical accepted receipts survive later checker expiry while current caller authority is still required.
- Immutable Finance observations distinguish execution request, receipt observed/not observed and inquiry unavailable. An unconfirmed response does not establish financial failure or release a hold.
- The staff workspace adds frozen source/leg/hash/reason review, independent approval, explicit customer-debit execution acknowledgment, native receipt, saved URLs and original/correction GL links. Four patterns/five operations bring the inventory to 106.
- The cloud runtime includes the guarded bank-fund fixture CLI. Both cloud runbooks retain loopback access, module roles and forward-only deployment. The complete correction runbook includes local/packaged fixture commands and expected results.

Migration **0050_finance_funding_corrections.sql** is applied to persistent local PostgreSQL and is immutable. Applied migrations 0001–0049 remain unchanged. See [specification](../finance-corrections.md), [runbook](../../runbooks/finance-corrections.md), [C4](../current-finance-components.puml), [sequence](../current-finance-corrections.puml) and [domain UML](../current-finance-correction-domain.puml).

## Verification so far

All eight focused PostgreSQL cases passed: concurrent exact replay/canonical independence/lost registration and native acknowledgment; rejecting payment/correction sources, wrong purpose and altered legs; held funds/restrictions with unchanged reservation; one native winner across competing approvals and immutable privileges; checker expiry after a financial account wait; cursor recovery past fifty revoked makers; trusted receipt-reference damage; and approval expiry while waiting for the original-source lock.

The focused real-issuer browser flow passed independent review/execution, deliberately aborted post-commit response recovery, exact customer booked/held/available restoration, one native linked reversal, stable key/scope/source hash, role/CSRF denial, original/correction GL booking links, saved URL, mobile layout and accessibility. Expanding staff navigation exposed an overflowing desktop sidebar; making it scrollable fixed the footer contrast, with the visible scrolled footer rechecked by accessibility tests. Initial failure and final passing logs are separate.

All 92 integration cases across nine targets passed, including 59 Finance cases (91 database-backed cases and one pure exact-total case). All nine payment/Finance browser flows plus customer and staff regressions passed. All 15 library cases, 106 ingress paths, current Rust/OpenAPI/TypeScript checks, both portal builds and six cloud configuration/archive tests passed. All 44 PlantUML sources rendered and passed XML/syntax validation. Final focused browser validation also passed the explicit recovered-receipt message after the final UI clarification. The mobile capture was visually inspected.

| Executed check | Actual result |
| --- | --- |
| `make app-check` | Rust formatting, locked Clippy with warnings denied, real OpenAPI validation, generated client drift and web TypeScript passed |
| `make test-unit` | All 15 library cases passed |
| `make test-integration` | All 92 cases across nine targets passed, including eight financial correction scenarios |
| `make test-payments` | Nine real-issuer payment/Finance browser flows passed |
| `make test-customer` / `make test-staff` | Both independent regressions passed |
| `make test-ingress` | All 106 patterns passed safe-error/body/quota/address-spoof checks |
| Final focused correction browser flow | Passed explicit receipt recovery, exact balances, held funds, one linked native correction, both GL exports, role/CSRF boundaries, saved URL, mobile and accessibility |
| `make web-check` / `make web-build` | All web types and both compiled portals passed after the final UI clarification |
| `make cloud-check` | Six configuration/archive checks passed |
| Diagram rendering/checks | All 44 rendered, XML-validated and syntax checked |

The actual sealed schema-50 archive passed packaged-runtime acceptance. Actual logs use `.local/corrections-*`; desktop/mobile captures use `.local/finance-correction-*.png`.

## Deployment refresh

The fresh release `cloud-20261110-01` contains schema 50, both compiled portals and the guarded native funding CLI. The actual archive is 84,171,340 bytes, SHA256 `a035111d51588da87f60e69961d29951c921387a8d92a24a6f95f5d5f33c141c`, with 21 deployment members and nested image SHA256 `fd3a93f4890fd54d6251a83de179216033fe6eb96ae5a75d45a556be24774d7c`. Actual checksum-verified archive acceptance passed fresh credentials/identity/bootstrap, health/portals, both clearing namespaces, two closed groups, new correction channel guards, compiled policy/correction/recovered-receipt controls and the packaged funding CLI usage boundary without a financial posting. Full authenticated financial execution was tested separately by host browsers and native PostgreSQL cases.

All disposable image-test resources were removed; developer services were retained. Both complete cloud runbooks and infrastructure input examples point to this new identifier. The [cloud validation record](../../deployment/VALIDATION.md) retains exact image IDs and historical release seals. No live GCP/AWS resources were applied.

## Remaining scope

This funding-only capability does not authorize provider payment refunds, incoming/return reversals, partial corrections or periodic charges/accruals. Periodic enrollment/material terms, authoritative historical basis, posting jobs, approved counterpart mappings and corresponding periodic correction eligibility remain unfinished in WP-24. Local synthetic checks do not establish live GCP/AWS deployment, production financial policy or provider qualification.
