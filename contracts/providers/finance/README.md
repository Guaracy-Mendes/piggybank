# Synthetic clearing-file contracts

This is a bounded JSON fixture protocol, independent of the bank's ledger/projections and separate from real rail file formats. The provider uses its own immutable `simulator.settlement_files` store. A file is an accounting observation, never financial execution authority. See the [matching specification](../../../docs/implementation/finance-reconciliation.md) and [runbook](../../../docs/runbooks/finance-reconciliation.md).

## Publish a fixture

`POST /admin/settlement-files`, JSON body `SettlementFile`, header `X-Finance-Key: <BANK_FINANCE_SIGNING_KEY>`. The default key is the repository's synthetic Finance fixture key; cloud-hosted evaluation generates its own key. The provider binds loopback and this admin route is not exposed through either portal.

Required fields:

| Field | Contract |
| --- | --- |
| `fileId` | Non-nil UUID; stable immutable file identity |
| `formatVersion` | `1` |
| `provider` | `SYNTHETIC_RAIL_SETTLEMENT` |
| `feePolicy` | `SYNTHETIC_PROVIDER_ZERO_FEE_V1` |
| `range` | `fromDate`, `toDate`; half-open UTC, 1–366 days, upper bound at most tomorrow |
| `generatedAt` | File's declared cutoff; not more than five seconds in the future |
| `rows` | 0–100 rows, whole file at most 60,000 serialized bytes |

Each row requires `lineId`, original `paymentId`, original final `evidenceId`, positive `providerVersion`, `principalMinor`, `providerFeeMinor`, `currencyCode`, `currencyScale` and actual settlement `businessDate`. UUIDs must be non-nil; amounts are canonical signed decimal i64 strings. Code is three uppercase ASCII letters; scale is 0–18. Wrong values/scales/dates and duplicate line/payment identities are deliberately retained for classification. Bad numeric encoding, unsupported policy or structural bounds are rejected with 422.

An exact same-ID/content replay returns 200; a new file returns 201; same-ID/different-content returns 409. Files cannot be edited, deleted or truncated. Corrections need a new file ID. This does not automatically supersede or rewrite any bank observation.

## Authenticated inquiry

`POST /settlement-files/inquiry` takes `SignedDocument` using `BANK_FINANCE_SIGNING_KEY`. Its body is `{kind: "SYNTHETIC_OUTGOING_RECONCILIATION_V1", fileId, requestId}`. Both IDs are non-nil UUIDs. The signature binds the complete canonical query plus its envelope timestamp. The response is a fresh signed `{request, file}`; the bank verifies the original nonce/query and file ID. A missing file returns 404, never an empty file. Invalid authentication returns 401. There are no redirects; client timeout is 1.5 seconds and response bound 64 KiB.

A saved bank artifact uses signature integrity validation without imposing the ingress sixty-second freshness window on an old immutable archive. Neither bank replay nor inquiry changes the provider file. The caller receives safe file lineage and classified rows; raw authenticated packets stay in private Finance storage.

## Fees, dates and negative fixtures

`providerFeeMinor` represents the provider's clearing charge. It is **not** the customer's original bank fee. This implemented explicit provider policy expects zero; nonzero or negative provider fee claims become `PROVIDER_FEE_UNEXPECTED`. The quote/Core/journal comparison checks the independently posted customer bank fee.

A row's business date is actual provider settlement date, which is compared with finality and journal date. The requested payment date remains a separate field in the bank instruction. A file cutoff before a later authenticated provider event is a timing observation; an actually absent earlier settlement is a missing-file-row exception.

For negative fixtures, publish a new file ID and retain the original operation/evidence IDs while changing one row's principal, currency, scale, date or provider fee; duplicate a row; omit an expected row; or add a provider-only operation. Do not modify Core journals to manufacture those cases. The database tests separately use explicit trusted storage-fault fixtures to verify detection of an inconsistent Core fee receipt and private archive damage.

## Independent incoming and return files

`CreditSettlementFile` uses the same file envelope, provider, fee policy, period and bounds, with a separate immutable `simulator.credit_settlement_files` store. Publish it at `POST /admin/credit-settlement-files` with `X-Finance-Key`, or use `python3 scripts/publish_settlement.py --kind credits <file>`. Same-ID semantics remain 201 new / 200 exact replay / 409 changed content. The outgoing V1 file/store is unchanged.

Each credit row requires:

| Field | Contract |
| --- | --- |
| `lineId`, `movementId`, `evidenceId` | Non-nil UUIDs from the independent provider movement, with a separate clearing line identity |
| `eventType` | `INBOUND_SETTLED` or `RETURN_SETTLED` |
| `originalPaymentId` | Original settled outgoing UUID for a return; null for incoming |
| `receivingReference` | Actual randomly issued PB reference for incoming; null for returns |
| `providerVersion` | Positive integer, matched to original signed movement version |
| `principalMinor`, `providerFeeMinor` | Canonical signed i64 decimal strings; zero clearing fee under the declared policy |
| `currencyCode`, `currencyScale` | Three uppercase ASCII letters, scale 0–18; matched against TST/2 |
| `businessDate` | Provider value date, retained separately from actual bank suspense/allocation booking dates |

Wrong kind/reference/original/amount/currency/date/version/evidence claims and duplicates remain classifiable. Reference strings are structurally bounded to 80 ASCII bytes; optional original UUID, when present, must be non-nil. A valid row shape is not proof of a bank credit. A provider over-cap return claim can exist without a native posting and must remain visible as an exception.

`POST /credit-settlement-files/inquiry` takes a fresh Finance-key SignedDocument whose body is `{kind: "SYNTHETIC_CREDIT_RECONCILIATION_V1", fileId, requestId}`. The signed response binds the entire original request and exact file. Outgoing query kinds cannot be reused here. Missing files return 404, bad authentication 401 and invalid structural scope 422. The bank uses the same loopback, no-redirect, 1.5-second, 64-KiB transport constraints and archived signature-integrity rules. It seals the original response privately; staff pages contain safe movement/file lineage.

See [incoming/return matching](../../../docs/implementation/finance-credit-reconciliation.md) and its [step-by-step runbook](../../../docs/runbooks/finance-credit-reconciliation.md). A supplied event later than the file cutoff is a conflict. A prior-period provider credit booked in the current bank period retains separate value and booking dates. Both native suspense and allocation must match captured journal/GL receipts; unresolved suspense remains partial and owned. Return comparisons include original settlement/payer and the lifetime principal cap, never a second customer bank fee.

## Closed multi-file settlement snapshots

`POST /admin/settlement-sets` publishes a provider-issued closed snapshot, authenticated by `X-Finance-Key`. Its body is `PublishSettlementSet`:

| Field | Contract |
| --- | --- |
| `setId` | Non-nil immutable UUID |
| `kind` | `OUTGOING` or `CREDITS`, selecting the existing file namespace |
| `range` | Exact half-open UTC period of every member |
| `generatedAt` | Common declared provider cutoff, identical to every member's cutoff |
| `fileIds` | 1–10 unique non-nil UUIDs of existing independent files |

The provider reads its own immutable files, validates namespace/period/cutoff and original file hashes, computes each typed file digest/row count, and seals UUID-sorted membership in `simulator.settlement_sets`. It never reads bank tables or accepts caller-authored member hashes. The signed `SettlementSet` has `formatVersion:1`, provider/fee policy, sorted `{fileId,digest,rowCount}` members and exact total `rowCount` (0–1,000). Each original file retains its 100-row/60,000-byte bound. Even an empty snapshot has at least one explicit member. Different original cutoffs cannot be merged under this protocol version.

New publication returns 201 with the sealed manifest, exact replay 200, changed same-ID content 409, invalid/missing/wrong-namespace/duplicate membership 422. Reordering the same requested file IDs replays the same canonical membership. The manifest and files reject update/delete/truncate; corrections require new identities. The provider's closure assertion covers this declared snapshot, not globally unknown external events.

`POST /settlement-sets/inquiry` takes a fresh Finance-key `SignedDocument` with body `{kind:"SYNTHETIC_SETTLEMENT_SET_RECONCILIATION_V1",setId,requestId}`. The signed response is `{request,manifest}` bound to the entire query/nonce and exact set ID. A missing set returns 404, invalid authentication 401 and invalid query scope 422. Bank transport retains no redirects, loopback-only base, 1.5-second per-request deadline and 64-KiB response bound. The bank subsequently fetches **every** member through its original signed file protocol, verifies all sealed identities/digests/counts/periods/cutoffs and saves nothing if any member is unavailable or altered.

The [grouped matching specification](../../../docs/implementation/finance-group-reconciliation.md) and [runbook](../../../docs/runbooks/finance-group-reconciliation.md) cover matching, row provenance, native source bounds and owned differences. The packing helper creates private bounded files from an independent provider row export; it does not publish or derive rows from bank projections:

```sh
python3 scripts/prepare_settlement_set.py --help
python3 scripts/publish_settlement.py --kind set .local/clearing-set/settlement-set.json
```

A grouped observation retains raw cross-file duplicate lines, operations and evidence. Public row claims bind back to the signed member files, while private HMAC packets remain sealed. Exact provider controls include every duplicate and stay separated by currency and scale. Grouping cannot post money, invent a signed aggregate file, overwrite a single-file V1 artifact or bypass the bank's 1,000-identity / 1,000-journal snapshot limits.
