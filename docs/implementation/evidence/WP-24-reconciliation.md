# WP-24 checkpoint: outgoing payment reconciliation

Date: 2026-11-06. Package status: **in progress**. This checkpoint completes the bounded internal/SIMULATED outgoing matching increment and a related payment-creation replay repair. It does not complete WP-24 or qualify real providers/cloud production deployment.

## Implemented behavior

- Native read-only Core accounting and Payments lineage ports retain their owning SQL principals. Original financial/report format V1 digests remain unchanged. The service captures separate source clocks, then confirms Core receipts after the Payments snapshot to avoid false missing-money observations during concurrent commits.
- Independent immutable clearing files use a fresh signed kind/file/nonce inquiry. Files can authentically contain missing, duplicate, currency, scale, principal, provider-fee, date or evidence differences. Structural/authentication/response bounds fail closed. Provider-file cutoff matching uses authenticated provider event time rather than later bank processing time.
- Immutable outgoing runs bind exact parent report/file identities to original instruction, Core receipt, journal legs and captured GL proof. i64 per-source amounts and i128 aggregates remain decimal strings; provider controls are partitioned by currency and scale. Customer bank fees and provider clearing fees remain separate. The matching workflow never books funds, releases holds, submits payments or replays GL.
- Summary/item/private-source hashes, signed file archives, current Finance role/queue checks, actor/client/key replay, stable ordinal pages and case wrappers are implemented in 3 new path patterns, bringing the actual API inventory to 91.
- Owned cases recover through stable issue origins, a bounded wrapping cursor and one-time acknowledgment links. Tests pass fifty conflicting scopes without starving a later record and recover a case committed before its Finance acknowledgment.
- The staff portal captures/reopens runs by URL, displays separate clocks/controls and safe payment/GL/case lineage, retains uncertain request keys, distinguishes read-only approvers and supports narrow/mobile views. Private authenticated packets are not included in the public item DTO.
- A pre-existing concurrent payment-creation race was exposed by the full suite: an accepted intent could be replayed before challenge linkage. Same-key eligible drafts now recover the original unique Controls challenge. Current authority and actual issuer step-up remain required; recovery does not fabricate proof or post money.

Migration **0045_finance_reconciliation.sql** was applied to persistent local PostgreSQL and is now forward-only. All prior applied migrations remain unchanged. The provider contract, fixture publisher, operator runbook and C4/UML models accompany the implementation.

## Executed verification

| Command / evidence | Actual result |
| --- | --- |
| `make test-integration` | All 58 cases passed across 9 workspace targets: 57 database-backed cases and one exact control-total case |
| Finance reconciliation subset | All 9 new cases passed, including authentic negative files, concurrent replay, large exact principal, authority revocation, archive damage, Core fee inconsistency, lost case acknowledgment, cursor fairness, late native commit and provider-cutoff semantics |
| Payment replay subset | New deterministic lost-link/concurrent retry case and the original financial approval/posting case passed; the full 17-case Payments target also passed |
| `make test-payments` | All 5 real-issuer payment/Finance browser flows passed; the added flow checks matching, original replay/digests, authentic duplicate/provider-fee exceptions, owned cases and customer/security/approver restrictions |
| `make app-check` | Rust formatting, warning-free locked Clippy, real OpenAPI validation, generated client drift and all TypeScript checks passed |
| `make test-customer` / `make test-staff` | One real-issuer customer lifecycle and one workforce regression passed |
| `make test-ingress` | All 91 implemented patterns passed authentication/error/body/quota/address-spoof boundary checks |
| `make check` | 55 Markdown documents, valid local references, all requirement/acceptance mappings and 38 diagram syntax checks passed |
| `make test-unit` | All 8 library cases passed |
| `python3 tests/integration/finance_publisher.py` | All 4 actual HTTP helper checks passed, including exact string bytes, redirect credential containment, loopback-only destinations and bounded input |
| `make cloud-check` | Existing 5 credential/configuration checks passed; no cloud deployment performed |
| `make web-build` | Both compiled browser bundles passed |
| `make diagrams` equivalent rendering | All 38 target/implementation/deployment sources rendered to XML-valid SVG |

The full browser fixture owns disposable databases, isolated broker streams, object storage and process groups. Database and HTTP/browser suites run sequentially on the shared local PostgreSQL instance. Functional browser quota stays at 5,000/minute for the fixture's shared loopback IP; application default 500 and provider 1.5-second deadlines remain unchanged.

The recorded mobile and desktop screenshots are `.local/finance-reconciliation-mobile.png` and `.local/finance-reconciliation-workbench.png`. The mobile capture was visually inspected; accessibility and no-horizontal-overflow checks passed. Local diagnostic logs use the `.local/wp24-reconciliation-*` prefix; the final integration and browser logs are distinct from initial failed diagnostics.

## Reproduce and inspect

Start with the [implementation specification](../finance-reconciliation.md), [operator runbook](../../runbooks/finance-reconciliation.md), [clearing-file contract](../../../contracts/providers/finance/README.md), [current C4 components](../current-finance-components.puml) and [UML sequence](../current-finance-reconciliation.puml). `make test-payments` builds the workspace before starting its browser fixture, so it does not use stale API/schema binaries.

The historical 2026-11-03 cloud archive contains migration 44. Cloud runbook examples now use a fresh release ID and build current source; deploying this increment requires a new image release containing migration 45 and its forward migrations. No real cloud account or provider was exercised by these local tests.

## Remaining work

WP-24 remains open for incoming/return-file reconciliation, grouped/larger file ingestion, versioned periodic fee/accrual commands, explicit calendar/cutoff rules and independently approved linked financial corrections for those commands. Original bank payment fees are already Core-posted and must not be charged again. Cross-authority snapshots are explicitly separate observations. These results establish local synthetic behavior, not statutory accounting, certified rail-file processing, capacity, production availability or full product completion.
