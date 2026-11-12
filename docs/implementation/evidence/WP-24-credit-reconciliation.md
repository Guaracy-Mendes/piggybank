# WP-24 checkpoint: incoming and return reconciliation

Finalized: 2026-11-08. Application/image tests ran on 2026-11-07; the sealed archive and persistent schema were rechecked on 2026-11-08. Package status: **in progress**. This increment implements bounded incoming/return matching, retained suspense and original-return controls, with a separate V1 artifact format. It does not complete periodic fee/accrual automation or the whole package.

## Implemented behavior

- Added bounded read-only Core credit accounting and Payments credit lineage capabilities under their owning SQL principals. Return snapshots retain original instruction, payer, settled receipt/date and lifetime returned principal, including suspense and out-of-period movements. Existing FinancialBatch, ReportSummary and outgoing reconciliation V1 digests remain unchanged.
- Added separate independent credit file publication, authenticated kind/file/nonce inquiry and immutable provider storage. Original movement kind/reference/payment, exact amount, evidence ID/version, provider fee and value date remain independently classifiable. Files are bounded to 100 rows/60,000 canonical bytes; transport remains loopback, no redirects, 1.5 seconds and 64 KiB.
- Added immutable credit runs, source archives, exact currency-partitioned controls, stable movement membership/pages, one-time case links and a wrapping fifty-issue ownership cursor. Incoming suspense, allocation and original-return/GL lineage are observation-only; reconciliation cannot post, allocate, refund, charge a bank fee or release a hold.
- Captures Core discovery, independent Payments snapshot, then native Core confirmation. Prior-day provider values newly booked between sources remain timing observations. A provider-period omission is distinct from current bank booking inventory; a file cannot match a row whose authentic event occurred after its declared cutoff.
- Added current Finance role/queue and actor/client/key guards, 3 route patterns / 4 operations, generated OpenAPI/TypeScript contracts and safe staff DTOs without signed HMAC packets. The API inventory is now 94 patterns.
- Added the staff report workbench with saved URLs, uncertain-key replay, source clocks, received/allocated/suspense controls, original-payment/GL/case lineage, stable pages and mobile/accessibility checks. Suspense principal is displayed separately from exclusive classification counts, so an exception cannot hide retained funds.
- Added [implementation notes](../finance-credit-reconciliation.md), [step-by-step runbook](../../runbooks/finance-credit-reconciliation.md), [provider contract](../../../contracts/providers/finance/README.md), [C4 components](../current-finance-components.puml), [domain UML](../current-domain.puml) and [matching sequence](../current-finance-credit-reconciliation.puml). All 39 target/implementation/deployment views render to XML-valid SVG.

Migration **0046_finance_credit_reconciliation.sql** is applied to persistent local PostgreSQL and is now immutable. Migrations 0001–0045 remain unchanged. The ingress readiness guard rejected the older persistent schema before the forward migration; it passed after migration 46 was applied.

## Verification

| Executed command | Actual result |
| --- | --- |
| `make test-integration` | 67 cases passed across 9 targets, including 34 Finance cases; 66 DB-backed cases and one exact control-total case |
| Focused credit reconciliation subset | All 9 new database scenarios passed |
| `make test-payments` | All 6 real-issuer payment/Finance browser flows passed |
| Focused incoming/return browser flow | Passed alone with fresh-account setup as well as in the combined suite |
| `make test-customer` / `make test-staff` | One customer lifecycle and one independent workforce regression passed |
| `make test-ingress` | All 94 route patterns passed contract/authentication/error/body/quota/address-spoof boundaries |
| `make app-check` | Locked Rust formatting/Clippy with warnings denied, real OpenAPI validation, generated client drift and all TypeScript checks passed |
| `make test-unit` | 9 cases passed, including live HTTP checks for valid credit files, forged nonces, bad signatures and oversized responses |
| `python3 tests/integration/finance_publisher.py` | 5 actual HTTP helper checks passed, including the distinct credit endpoint and exact large decimal bytes |
| `make web-build` | Both compiled React portals passed |
| `make cloud-check` | 6 configuration/archive tests passed; unsafe/tampered archive subcases are exercised before container execution |
| `python3 tests/deployment/runtime.py cloud-20261107-01` | Actual sealed archive, migration 46, randomized credentials/identity, portals/health, independent publication, both signed clearing namespaces, channel guards and repeated seed passed |
| `make check` and diagram rendering | 58 Markdown documents, traceability/local links and 39 PlantUML sources passed; all 39 SVGs rendered and XML-validated |

The release build and actual packaged-image acceptance also passed. The isolated environment migrated schema 46, provisioned random credentials and both realms, independently published fixtures, started API/worker/provider/static portals, verified both signed clearing protocol namespaces with the installation key, checked channel restrictions and replayed immutable seed commands. All acceptance pods/volumes were removed; developer services were retained.

The nine new Finance database scenarios cover exact large incoming principal and multiple partial returns/caps; simultaneous replay and current authority; unresolved/unknown suspense and lost allocation acknowledgment; independent kind/reference/original/amount/currency/scale/fee/date/evidence gaps and duplicates; unavailable/bounded sources and private archive damage; pending/late captures and provider cutoffs; provider value versus bank booking periods; concurrent prior-period booking between source reads; and owned-case recovery beyond fifty conflicting scopes.

The browser scenario passes both alone and in the combined suite. Its initial combined setup attempted a new profile for an already established customer; Core correctly required canonical identity review. The final fixture reuses an existing active account, or opens a fresh account in isolation, while preserving all financial and issuer checks. The combined flow also retains provider over-cap return claims that were correctly quarantined by the bank, rather than deleting them to force a complete result.

Local diagnostics use `.local/credit-*`. Desktop/mobile captures are `.local/finance-credit-reconciliation-desktop.png` and `.local/finance-credit-reconciliation-mobile.png`. The narrow layout was visually inspected, with no horizontal overflow. Browser/HTTP/database suites run sequentially on the shared PostgreSQL service. Provider deadlines, application quotas and immutable prior artifacts retain their existing boundaries.

## Cloud release and operational files

Image acceptance now verifies the selected release's outer checksum, manifest identity and complete membership, rejects traversal/link/duplicate/undeclared entries, verifies all deployment/image hashes, loads the actual image archive and executes its packaged deployment code. It no longer substitutes deployment files from the checkout. Six configuration/archive tests pass, including explicit tampering and unsafe-member cases.

Both cloud tunnel bundles add an optional trusted-operator provider forward with `BANK_TUNNEL_PROVIDER=1`. Default portal/API/issuer tunnels remain the same five ports. The optional loopback 4182 tunnel makes the fixture publisher usable from the workstation with the installation's privately supplied original signing key. Runbooks and deployment C4 models include this operational path; no public provider listener or cloud ingress rule is added.

The fresh migration-46 release `cloud-20261107-01` is built and locally accepted. Its 81,659,976-byte archive has SHA256 `98a2d1bfcef5f028c869e70264164353b2a91b10b965e21232ad8de7ec027a00`. Exact image IDs and the complete validation boundary are in the [cloud validation record](../../deployment/VALIDATION.md). No live GCP/AWS account was used; no cloud resources were applied.

## Remaining WP-24 work

Grouped/larger-file ingestion, versioned periodic fee/accrual jobs, explicit calendar/cutoff rules and independently approved linked corrections remain open. Existing outgoing bank payment fees are already Core-posted and must not be charged again. Cross-authority snapshots remain separate observations. Real provider formats, statutory accounting, live cloud operation and production qualification remain separate work.
