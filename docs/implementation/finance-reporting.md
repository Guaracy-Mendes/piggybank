# Finance observation snapshots and liquidity

Current status: essential synthetic WP-24 is complete. This document retains the narrower increment it introduced; the [financial lifecycle](periodic-finance-r1.md), [scope decision](wp24-essential-scope.md) and [completion evidence](evidence/WP-24-complete.md) describe the completed execution and deliberate deferrals. Earlier references to remaining work below describe that checkpoint.

This WP-24 increment adds immutable period reports and an independent synthetic liquidity feed to the existing [GL export slice](finance-execution.md). It implements Core/GL controls and owned snapshot observations. A subsequent [controlled revision workflow](finance-revisions.md) links independently approved replacements without changing either artifact. Full instruction/provider/fee reconciliation and periodic fee/accrual automation remain open within WP-24.

## Snapshot contract and consistency

`FinanceRecords` now exposes bounded `snapshot(range)` and `positions()` read capabilities. Core reads account stocks, held balances, active outgoing principal commitments, journal membership and complete journal legs in one PostgreSQL REPEATABLE READ, READ ONLY transaction. A report range is a half-open UTC interval of at most 366 days. A period containing more than 1,000 batches fails with `422`; it never publishes truncated totals. Customer held funds include fees and other holds, while reserved/submitted outgoing liquidity principal includes only active external commitments.

Finance captures the corresponding export, chart and receipt records in a separate consistent read transaction. It then closes that transaction before calling the liquidity provider. Core, Finance and provider observations retain separate as-of times: this is not a globally atomic snapshot across independent sources. Newly received/backdated sources after the Core snapshot belong to later reports.

The saved format-version-1 artifact seals its range, immutable Core membership, source digests, full account legs, chart/export/receipt references, period account controls, exact debit/credit totals, matched GL totals, independent cash observation and completeness. The Core snapshot and ordered item list have separate aggregate digests. Detail retrieval reconstructs and checks membership; paged lineage retrieval verifies captured matching GL receipts. Archived liquidity proof integrity and evidence lineage are checked without treating historical proof as a new live message.

Report creation requires current Finance adjuster authority and REVIEW queue membership. Inquiry permits current Finance adjuster/approver authority with the matching Finance queue relationship. The request UUID is scoped to issuer, subject and client. Concurrent identical requests select one artifact. A lost response replays that exact artifact without refreshing membership, GL status or provider data. A changed period with the same key returns `409`; revocation denies replay. Explicitly creating a new observation uses a new key and retains all previous snapshots.

## Exact controls and classified observations

| Control | Interpretation |
| --- | --- |
| Core debit/credit | Exact period journal controls, equal for every balanced source |
| Per-account debit/credit/net | Frozen Core account movement; net is credit minus debit |
| Matched GL controls | Include only sources whose original identity, date, legs, mapping and authenticated receipt agree |
| GL completeness | All captured sources matched and Core aggregate ledger control is zero |
| Overall completeness | GL completeness plus fresh independent liquidity evidence at capture |
| Current captured-evidence status | Derived freshness of the captured evidence; the saved artifact and its digest remain unchanged |

Per-leg money remains the existing signed 64-bit minor-unit string type. Aggregates use 128-bit arithmetic and canonical decimal strings. No browser amount is converted to a floating-point number.

Sources classify as `MATCHED`, `GL_MISSING`, `GL_PENDING` or `GL_CONFLICT`, with explicit missing-export, date, value, mapping, identity or receipt reason codes. Expected missing/pending work within the selected synthetic 30 second processing window remains visible as partial with `GL_TIMING_WINDOW`. Later missing/pending sources and conflicts create immutable snapshot issue records with owned Finance cases. Missing/stale/invalid liquidity produces a whole-report issue. Case assignment commits separately and is recovered in pages of 50, including after a lost acknowledgment. Replaying an old partial report does not reopen a newer operational reconciliation result: these cases describe the retained historical observation.

Only authenticated matching GL receipts count toward confirmed GL totals. Reports never book Core money, change a hold, add a fee or turn provider acknowledgment into settlement. A new complete report does not rewrite an earlier partial report. This increment provides observations, not statutory filing formats or approved financial corrections.

## Independent liquidity evidence

The simulator stores immutable, versioned cash observations in its own schema. It selects the greatest version, signs a reply bound to the current request nonce, currency and `SYNTHETIC_TST_CLEARING` scope, and preserves the original evidence ID/as-of/expiry. There is no default cash amount. Administrative fixture publication requires the separate liquidity key; changed content for the same identity conflicts. A later stale or invalid version cannot silently fall back to an earlier fresh observation.

The provider control is `settled = available + reserved`, using bounded exact aggregates. Expected incoming and outgoing amounts remain separate uncertain observations. The selected local availability rule is conservative: `max(0, provider available - all Core active outgoing principal)`. It can reserve a commitment again when the provider already included it; it provides no spending or financial command authority. Core settlement ledger balances are never substituted for a missing provider feed.

| Status | Public current cash values |
| --- | --- |
| `FRESH` | Authenticated scope/date controls pass and provider evidence is unexpired; Core observation is at most five seconds old |
| `STALE` | Current available liquidity is unavailable; provider expiry or Core age explains the result |
| `MISSING` | No published provider observation; cash values are unavailable |
| `INVALID` | Authentication, request binding, currency/scope or cash controls failed; cash values are unavailable |
| `UNAVAILABLE` | Transport failed or exceeded bounds; cash values are unavailable |

Live envelopes have the existing 60 second authentication window; the evidence lifetime itself is at most 60 seconds. HTTP retains the 1.5 second deadline, loopback restriction, redirect rejection and 64 KiB reply limit. `BANK_LIQUIDITY_SIGNING_KEY` is separate from GL and payment-rail keys. The staff view also expires cash values locally between polls. Saved reports label liquidity as captured and show its derived current freshness separately from live liquidity.

## Interfaces and recovery

| Interface | Behavior |
| --- | --- |
| `GET /api/v1/staff/finance/reports` | Most recent 100 saved summaries |
| `POST /api/v1/staff/finance/reports` | `{fromDate,toDate}` plus UUID `Idempotency-Key`; creates or replays one immutable artifact |
| `GET /api/v1/staff/finance/reports/{id}` | Verified controls, source digests, safe captured liquidity, issues and case links |
| `GET /api/v1/staff/finance/reports/{id}/items?after=0&limit=25` | Stable ordinal pages, limit 1–100, of frozen source/GL lineage |
| `GET /api/v1/staff/finance/liquidity` | Current Core commitment observation and independently authenticated provider cash status |
| Simulator `POST /liquidity/inquiry` | Separate signed nonce-bound inquiry |
| Simulator `POST /admin/liquidity-positions` | Key-authenticated publication of synthetic evidence; preserves version/identity |

The workforce portal saves and reopens snapshot URLs, displays exact controls and source drill-down, links owned exceptions, and distinguishes current liquidity from captured evidence. See the [snapshot sequence](current-finance-reporting.puml), [Finance components](current-finance-components.puml) and [operating runbook](../runbooks/finance-reporting.md).

The separate [outgoing payment reconciliation artifact](finance-reconciliation.md) binds an existing report digest to an independent clearing file and bounded Core/Payments accounting snapshots without changing report format V1.
