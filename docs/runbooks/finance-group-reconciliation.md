# Reconcile a closed group of clearing files

Use the [local runtime](local-development.md) or a newly built GCP/AWS evaluation release containing migration 47. Retain original installation keys. This workflow observes evidence and assigns cases; it has no financial execution command. Single-file [outgoing](finance-reconciliation.md) and [incoming/return](finance-credit-reconciliation.md) artifacts remain independently inspectable.

## 1. Start the current code and dependencies

```sh
make dev-up
make migrate
make identity-setup
make dev-seed
make dev
```

Expected: schema 47 or later and healthy API/worker/issuer/provider. Reviewer has current Finance adjuster/REVIEW; independent approver can inquire. For an existing cloud installation follow its stopped-backup and forward-upgrade procedure with a fresh release built from current source. Historical migration-44/46 images do not contain grouped reconciliation.

## 2. Obtain a provider-declared snapshot

Export raw outgoing or incoming/return row objects independently from provider records, following the [clearing contract](../../contracts/providers/finance/README.md). Retain original operation/movement, evidence, reference/original-payment, amount, currency/scale, fee and value date. Do not build provider claims from bank balances, a projected workflow flag or a reconciliation result.

Establish the provider's common declared cutoff and half-open UTC period. All chunks must belong to that same snapshot. Different original cutoffs or periods require separate sets or a newly issued provider snapshot; do not edit old signed evidence to make it fit. Save the raw JSON array in `.local/provider-rows.json`. Preserve duplicates and differences.

Expected: one explicit namespace (`outgoing` or `credits`), source period/cutoff and raw provider rows. Original bank payment fees remain separate from provider clearing fees; the synthetic provider policy expects zero clearing fee.

## 3. Pack a bounded immutable fixture dataset

Set these variables from the provider export: `CLEARING_KIND` to `outgoing` or `credits`, `FROM_DATE` / `TO_DATE` to ISO UTC dates, and `PROVIDER_CUTOFF` to its timezone-aware RFC3339 cutoff.

```sh
: "${CLEARING_KIND:?set outgoing or credits}"
: "${FROM_DATE:?set provider period start date}"
: "${TO_DATE:?set exclusive provider period end date}"
: "${PROVIDER_CUTOFF:?set original provider snapshot cutoff}"
python3 scripts/prepare_settlement_set.py \
  .local/provider-rows.json .local/clearing-set \
  --kind "$CLEARING_KIND" --from-date "$FROM_DATE" --to-date "$TO_DATE" \
  --cutoff "$PROVIDER_CUTOFF"
python3 -m json.tool .local/clearing-set/settlement-set.json
```

Expected: a new private directory, up to ten `file-NNN.json` files and `settlement-set.json`. Each file has a unique file ID, at most 100 rows and 60,000 bytes; every file shares the declared period/cutoff. Amounts remain canonical decimal strings, including values above JavaScript's exact integer range. Row order and duplicates are retained. The tool refuses existing output, oversize data and numeric amounts before creating a partial dataset. It publishes nothing. An empty snapshot still gets one explicit empty member file.

## 4. Publish every member, then seal membership

For local evaluation the publisher uses the fixture Finance key. For cloud evaluation supply the installation's original `BANK_FINANCE_SIGNING_KEY` privately and run the chosen tunnel bundle with `BANK_TUNNEL_PROVIDER=1`; see [cloud access](../deployment/README.md). Do not put signing keys in arguments or logs.

```sh
set -e
for MEMBER_FILE in .local/clearing-set/file-*.json; do
  python3 scripts/publish_settlement.py --kind "$CLEARING_KIND" "$MEMBER_FILE"
done
python3 scripts/publish_settlement.py --kind set \
  .local/clearing-set/settlement-set.json
```

If any member publication fails, resolve that failure before sealing the set. The provider independently rejects a manifest with an unavailable/wrong-namespace member, so an incomplete loop cannot silently become a complete group.

Expected: member and set publication returns 201 new / 200 exact replay / 409 conflicting same-ID content / 422 invalid closure or bounds. The provider computes sorted member digests and row counts from its own immutable records. A reordered list of the same file IDs replays the same manifest. Retain the set UUID, all member UUIDs and the original files.

## 5. Save a matching parent report

Sign into the staff portal as reviewer, open **Finance & GL**, and capture an immutable reporting snapshot for exactly the set's period. Review captured journal and GL membership. Incoming movements can have separate suspense and allocation journals; returns retain original settled-payment linkage and lifetime principal caps. Liquidity freshness is a separate observation.

Expected: stable report identity/digest and explicit source clocks. Pending GL work or a later allocation cannot silently change this report. Use a new report to capture new journal/GL membership.

## 6. Capture the complete clearing group

Open the saved report. Under **Reconcile a clearing file group**, enter the published set UUID and choose **Capture clearing group**. Preserve the resulting URL and uncertain request key after a network interruption.

Expected: the service verifies the signed manifest and every member before saving. A missing, altered, invalidly authenticated or bound member fails with dependency-unavailable; no partial group is saved. A source union beyond 1,000 identities or parent report beyond 1,000 journal batches fails explicitly. Splitting files cannot bypass bank-source limits.

The saved artifact shows OUTGOING_PAYMENT_GROUP_V1 or INCOMING_RETURN_GROUP_V1, exact controls, common cutoff and separate bank/workflow/report clocks, COMPLETE/PARTIAL/EXCEPTIONS, and the entire closed file membership. COMPLETE applies to this declared snapshot and captured bank union, not global provider completeness or production certification.

## 7. Inspect cross-file lineage and owned differences

Open **Frozen group lineage**. Check original bank facts and GL receipts plus every provider file ID, row ordinal, line ID, row digest and raw value/currency/scale/fee/date/evidence claim. Duplicate rows keep multiple references and remain in provider controls. Review principal, original bank fee or credit allocation/suspense separately.

| Observation | Expected action |
| --- | --- |
| Missing member / member hash or cutoff mismatch | Repair/reissue provider evidence; do not omit the member or substitute an empty file |
| Duplicate line/operation/evidence across files | Review every original row reference and owned case; do not deduplicate away the discrepancy |
| Provider currency/value/fee/date conflict | Retain independent files and assign the owned Finance case; never post an extra bank fee from reconciliation |
| Native suspense | Follow the existing controlled credit workflow; keep partial status and owned cases until actual allocation |
| New native fact absent from parent report | Save a fresh report and new group observation; preserve the original timing artifact |
| Pending processing / lost acknowledgment / GL grace | Recover the original durable workflow and capture new evidence when appropriate |
| Pending case ID | Keep worker recovery running; choose **Refresh clearing groups** to retrieve current links |
| Artifact/archive integrity failure | Retain evidence and investigate; do not edit stored JSON or hashes to force a successful read |

Expected: durable differences acquire Finance cases using stable issue origins. The worker wraps after fifty attempted pending links and advances past conflicting scopes. A case committed before its acknowledgment is reused. Case disposition does not rewrite the original observation.

## 8. Reopen and retain the original artifact

Reload `/finance/reports/{reportId}/groups/{runId}`. Use first/next lineage pages, verify the digest and closed member list, and retain file/set/report/run references. Read-only approver can inquire without capture controls; customer/security contexts cannot use Finance actions.

Expected: actor/client/key replay returns the original artifact, including when the provider is unavailable, without refreshing facts. New evidence needs a new key and new journal membership needs a new report. Older single-file V1 and grouped artifacts retain their original digests. No provider HMAC packets appear in staff DTOs.

## Completion checklist

- [ ] Independent provider rows, common cutoff and period retained.
- [ ] Every declared member published and sealed; none silently omitted.
- [ ] Complete source union within provider/bank/journal bounds.
- [ ] Cross-file duplicates and currency/fee/value differences reviewed with per-row references.
- [ ] Original instructions, native receipts, suspense/allocation, returns/caps and GL evidence inspected.
- [ ] Owned cases or explicit short grace account for all durable differences.
- [ ] Reopened digests/pages and prior V1 artifacts remain unchanged.
- [ ] No financial posting, hold release, resubmission, allocation or fee charge performed by reconciliation.

Reproduce with `make test-integration`, then `make test-payments` sequentially. Operator helpers have standalone checks in `tests/integration/finance_packing.py` and `tests/integration/finance_publisher.py`.
