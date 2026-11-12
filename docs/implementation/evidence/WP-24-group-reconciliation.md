# WP-24 checkpoint: closed multi-file reconciliation

Date: 2026-11-08. Package status: **in progress**. This checkpoint implements bounded grouped clearing snapshots for outgoing payments and incoming/returns. Periodic fee/accrual/calendar/correction execution remains open.

## Implemented behavior

- Provider-issued immutable settlement sets bind one namespace, exact period/common cutoff and sorted closed membership of 1–10 independent files. The provider derives member digests/row counts from its own immutable store. Every original file retains the 100-row/60,000-byte bound; grouped input supports 1,000 raw provider rows.
- Signed nonce-bound manifest and member inquiries retain loopback-only transport, no redirects, 1.5-second per-request deadline and 64-KiB response bounds. Missing/altered/invalid members abort capture before any artifact is saved.
- Separate group V1 storage retains original set/member HMAC packets, typed native/workflow snapshots, source clocks, parent identity/digest, exact i128 currency-partitioned controls and stable ordinal operation items. The earlier single-file, FinancialBatch and ReportSummary V1 DTOs/digests remain unchanged.
- Group matching reuses pure outgoing/credit inspections and retains duplicates across files. Every row has file UUID, original ordinal, line UUID and row digest provenance. Public provider claims are independently bound back to the signed files, even if an incorrect storage repair recomputes public artifact hashes.
- A supplied outgoing or credit row after the authenticated event cutoff is an exception. A missing row for a later authentic event remains timing, and an event exactly at the cutoff is included. Native suspense, allocation, original-return principal caps, late parent membership, GL timing and lost workflow acknowledgment preserve their original meanings.
- Staff APIs add 3 patterns / 4 operations, bringing the inventory to 97. Current Finance roles/queues, BFF/CSRF/origin/quota, actor/client/key binding, original replay while the provider is unavailable, stable pages and private-archive verification are enforced.
- The report workbench displays the closed file list, exact controls/clocks, raw per-row claims/provenance, original payment/return/GL lineage and owned cases. Uncertain keys persist in session storage; saved URLs reopen groups; independent approvers can read without capture controls.
- Case ownership commits as immutable issues with stable origins and recoverable one-time links. The fifty-issue cursor wraps past failures and recovers acknowledgments without duplicating cases. Reconciliation performs no financial posting, allocation, release, resubmission, refund, extra bank fee or GL replay.
- The preparation tool packs independently supplied provider rows into private bounded files and a manifest publication request. It preserves large exact strings, row order and duplicates; refuses existing output, invalid numeric encodings and oversize input before creating partial data; and performs no publication or bank query.

Migration **0047_finance_group_reconciliation.sql** is applied to persistent local PostgreSQL and is now immutable. Applied migrations 0001–0046 remain unchanged. See the [implementation specification](../finance-group-reconciliation.md), [operator runbook](../../runbooks/finance-group-reconciliation.md), [provider contract](../../../contracts/providers/finance/README.md), [C4 components](../current-finance-components.puml), [domain UML](../current-domain.puml) and [matching sequence](../current-finance-group-reconciliation.puml).

## Verification completed

| Executed command | Actual result |
| --- | --- |
| `make test-integration` | All 77 cases passed across 9 targets, including 44 Finance cases: 76 database-backed cases and one exact-control-total case |
| New grouped database scenarios | All 10 passed: 105 outgoing rows with unchanged V1 artifacts, incoming/partial returns/suspense, cross-file differences, canonical closure/replay/bounds, missing/member archive damage, late allocation/report timing/offline replay, ownership beyond fifty bad scopes, 1,000 rows with refusal of a 1,001-identity union, rehashed public-claim damage, and exact outgoing cutoff semantics |
| `make test-payments` | All 7 real-issuer payment/Finance browser flows passed, including both grouped namespaces, cross-file exceptions, row provenance, immutable prior digest, role restrictions, saved URL, pagination, mobile layout and accessibility |
| Focused grouped browser flow | Passed independently as well as in the combined suite |
| `make test-customer` / `make test-staff` | One customer lifecycle and one independent workforce regression passed |
| `make test-ingress` | All 97 patterns passed authentication/contract/safe-error/body/quota/address-spoof checks |
| `make app-check` | Rust formatting, locked Clippy with warnings denied, real OpenAPI validation, generated client drift and TypeScript checks passed |
| `make test-unit` | 10 library cases passed, including live manifest HTTP checks for valid scope, forged nonce, bad signature, invalid membership and oversized response |
| `python3 tests/integration/finance_publisher.py` | 6 real HTTP helper checks passed, including the set endpoint, exact bytes and redirect credential containment |
| `python3 tests/integration/finance_packing.py` | 5 operator-file checks passed: exact large strings/duplicates/order/common cutoff/private modes, bounds/numeric rejection before output, refusal to overwrite, credit linkage and explicit empty membership |
| `make web-build` | Both compiled React portals passed |
| `make cloud-check` | 6 deployment/configuration/archive checks passed |
| Diagram rendering | All 40 target/implementation/deployment sources rendered and were XML-validated |

Final documentation checks passed 61 Markdown files, 526 local references, all requirement/acceptance mappings and all 40 PlantUML sources. Actual sealed-archive acceptance passed schema 47 migration, random credentials/identity, distinct maker/checker bootstrap, portals/health, both signed file namespaces and two closed two-member groups, channel guards and repeated seed. All disposable pods/volumes were removed; developer services were retained.

The full financial/HTTP/browser suites run sequentially on the shared local PostgreSQL service. The functional fixture's quota remains 5,000/minute and production-profile assumptions remain unchanged. The mobile screenshot `.local/finance-group-reconciliation-mobile.png` was visually inspected; accessibility and no-horizontal-overflow checks passed. Desktop capture is `.local/finance-group-reconciliation-desktop.png`. Actual diagnostics use `.local/group-*`; initial compiler diagnostics are separate from the final successful logs.

## Deployment refresh

The fresh release identifier is `cloud-20261108-01`, containing migration 47 and the grouped workbench/protocols. Actual packaged-runtime acceptance verified both closed two-member namespaces with the installation's randomized Finance signing key, in addition to migrations, identity/bootstrap, static portals, health and channel boundaries. The archive is 81,831,187 bytes, SHA256 `3f8cacb6bcc7412b92c44fecfc03100be7bbd296e9b0f4aeb83d988a6dec7e61`, schema 47, with 21 deployment files. Exact image seals are in the [cloud validation record](../../deployment/VALIDATION.md). The immutable migration-46 release remains recorded separately. No live GCP/AWS resources are applied by local validation.

## Remaining work and limits

The provider/file capacity increase does not raise the existing 1,000-identity and 1,000-journal bank snapshot bounds. A day exceeding those bounds remains unsupported; credits commonly consume two journals. Global real-provider completeness, unbounded ingestion and production capacity are not implied by a complete declared group. WP-24 remains open for versioned periodic fee/accrual jobs, explicit business calendars/cutoffs and independently approved linked financial corrections. Original bank payment fees are already Core-posted and must not be charged again.
