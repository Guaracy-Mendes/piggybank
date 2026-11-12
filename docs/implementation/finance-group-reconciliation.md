# Grouped clearing-file reconciliation

This WP-24 increment matches a closed provider settlement snapshot across multiple bounded files. It supports outgoing payments and incoming/returned principal, preserving native money authority and the earlier single-file V1 formats. See the [operator runbook](../runbooks/finance-group-reconciliation.md), [provider contract](../../contracts/providers/finance/README.md), [C4 components](current-finance-components.puml), [domain UML](current-domain.puml) and [group sequence](current-finance-group-reconciliation.puml).

## Closed provider membership

An independently published `SettlementSet` declares one namespace (`OUTGOING` or `CREDITS`), immutable set identity, half-open UTC period, common provider cutoff and complete file membership. Each member binds its file UUID, full typed file digest and original row count. The provider computes those facts from its own immutable files, never from bank balances or projections. Publication accepts explicit existing file IDs and seals their sorted membership; it does not accept caller-authored member hashes or row counts.

The set supports 1–10 files, each retaining its existing 100-row/60,000-byte bound, for at most 1,000 raw provider rows. Empty snapshots still have at least one explicit zero-row file. Members must share the exact period and cutoff and the selected namespace. The protocol rejects duplicate/nil file identities, unknown or wrong-namespace members, changed same-ID publication, mixed cutoffs/periods and inconsistent bounds. Canonical membership is UUID-sorted; a reordered publication request is an exact replay of the same set.

The manifest is a provider assertion that these files constitute the complete declared snapshot. It does not prove that a real external provider has supplied every possible event. Completeness applies to the declared signed membership and captured bank source union. Real provider export/completeness policy remains a qualification gate.

## Authenticated capture and source ownership

Creation takes `{reportId,setId}` and a UUID Idempotency-Key under current Finance adjuster/REVIEW authority. Inquiry requires current Finance inquiry authority; independent approvers can read without capture controls. Customer and security-administration roles supply no Finance authority. Existing workforce BFF, origin/CSRF, safe-error and quota boundaries apply. Actor/client/key binds one exact parent report and set; a changed scope is 409.

The bank verifies a fresh signed set-ID/request-nonce inquiry, then obtains every member through its existing authenticated file namespace. Each response remains bounded to 64 KiB and 1.5 seconds, with no redirects or non-loopback transport. At most ten sequential member reads occur; no bank transaction spans provider HTTP. Each typed file must match the sealed member identity, digest, row count, period and common cutoff. An unavailable, invalid or altered member returns dependency-unavailable and saves no observation. A member is never silently dropped or replaced with an empty file.

The service verifies the selected immutable parent report and its journal/GL archives, discovers native accounting identities through the Core read port, captures the owning Payments snapshot, then confirms native Core receipts with explicit provider/workflow identities. Source clocks remain distinct. No distributed atomic snapshot is claimed. Existing source/union bounds remain 1,000 bank identities and 1,000 parent journal batches; a source union beyond the limit fails 422 rather than publishing a truncated result.

## Matching, controls and row provenance

The group reuses the existing pure [outgoing inspection](finance-reconciliation.md) and [credit inspection](finance-credit-reconciliation.md). Sharing a read-only row/period/cutoff view does not create a fictional signed aggregate file. Original instruction, quote, provider finality, native principal/bank fee, journal legs, original return/cumulative cap, suspense/allocation and captured GL checks retain their existing meaning.

All declared rows participate together. Duplicate line, operation/movement and evidence identities across files remain visible; there is no deduplication. Duplicate evidence is explicitly classified in the outgoing group as well as the credit group. A supplied row whose authenticated settlement event is after the declared cutoff is an exception in both scopes. An absent row with a later authentic event remains a timing observation; an event exactly at the cutoff is included. Raw provider totals retain every duplicate and remain partitioned by currency code and scale. Canonical per-row i64 amounts and i128 aggregate amounts remain decimal strings. Provider fees remain separate from original bank fees and cannot become a new financial charge. Native totals describe the captured identity union, including explicit out-of-period facts, rather than period cash flow.

Each stable operation item contains its original outgoing or credit inspection and file UUID / one-based row ordinal / line UUID / row digest references for every matching raw provider row. Two identical lines in different files keep two source references. The staff view shows per-row provider value/currency/scale/fee/date/evidence alongside those references. Native bank-only candidates retain an empty provider-reference list and the applicable missing/timing classification.

Public row claims are verified against the signed member archives, in addition to their own saved hashes and source references. A repair that incorrectly changes a displayed provider amount and recomputes the artifact hashes still fails because the independently signed file remains unchanged.

Group completeness is EXCEPTIONS if any operation conflicts, PARTIAL while pending/timing/suspense remains, otherwise COMPLETE for the explicit group scope. Credit suspense principal remains separately visible even when the same operation has another exception. The common cutoff supports the existing late-event timing semantics without merging incompatible file cutoffs.

## Immutable storage and recovery

Migration 47 adds separate Simulator set and Finance group tables. Existing FinancialBatch V1, ReportSummary V1, outgoing V1 and credit V1 serialization and saved digests remain unchanged. New scopes are `OUTGOING_PAYMENT_GROUP_V1` and `INCOMING_RETURN_GROUP_V1`.

A run seals the manifest archive, every member response packet, separate typed native/workflow snapshots, parent report identity/digest, source clocks, exact controls, stable ordinal items and row provenance. Reads verify manifest/file HMAC integrity and closed membership, all private/public hashes, namespaces, parent range/digest, source clocks, row identity/membership and exact provider claims. Old archives use integrity validation rather than live transport freshness. Private packets stay out of staff DTOs.

Accepted actor/client/key replay returns the original verified artifact, including while the provider is unavailable, without refreshing its source facts. New observations need a new key; new journal/GL membership needs a fresh parent report. A later good set or allocation cannot rewrite an old exception/suspense/timing result.

Durable differences receive immutable issues and Finance cases through stable issue origins. Short existing processing/acknowledgment/GL grace remains explicit. The wrapping fifty-issue cursor advances past failures, reuses a case committed before acknowledgment loss and cannot be starved by fifty bad scopes. Current case links are a separate wrapper; refresh to retrieve updated ownership. Reconciliation cannot post, allocate, refund, resubmit, release holds or replay GL exports.

## API and staff workflow

| Method | Route | Behavior |
| --- | --- | --- |
| GET / POST | `/api/v1/staff/finance/reconciliation-groups` | Latest 100 verified summaries, optional `reportId`; capture one complete set |
| GET | `/api/v1/staff/finance/reconciliation-groups/{id}` | Saved controls/digest and current issue ownership |
| GET | `/api/v1/staff/finance/reconciliation-groups/{id}/items` | Stable `after` ordinal and limit 1–100 |

The report view accepts a published set UUID and exposes the immutable closed member list, clocks, controls, per-file row provenance, original payment/return/GL lineage and owned cases. URL `/finance/reports/{reportId}/groups/{runId}` reopens the artifact. Uncertain capture retains its request key in session storage. Read-only approvers do not receive capture controls. Mobile/accessibility and current financial state are validated through the real issuer/BFF browser flow.

## Operational packing and limits

`prepare_settlement_set.py` packs independently supplied provider row objects into a new private directory, preserving decimal strings, row order and duplicates. It writes bounded member files and a publication request, with a common explicitly supplied UTC period/cutoff. It never queries bank state or publishes data. Invalid sizes, numeric encodings and existing output paths are refused before writing a partial dataset. The provider remains the authority for typed row validation, member digests and signed closure.

The larger provider bound does not raise bank/report limits or the one-day minimum reporting period. A single day exceeding 1,000 bank identities or 1,000 journal batches remains unsupported; incoming allocation commonly consumes two batches per movement. Splitting files cannot bypass these limits. Unbounded ingestion, real provider formats and capacity qualification remain separate work. WP-24 also remains open for versioned periodic fee/accrual commands, calendars/cutoffs and independently approved linked financial corrections.
