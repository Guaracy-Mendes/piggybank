# Reconcile incoming and returned payments

Use the synthetic [local environment](local-development.md), or an upgraded GCP/AWS evaluation installation reached through its documented loopback tunnels. Preserve original installation keys. This runbook observes evidence and assigns cases; it does not allocate funds, return principal, change a ledger or replay a payment.

## 1. Start the current runtime

```sh
make dev-up
make migrate
make identity-setup
make dev-seed
make dev
```

Expected: migration 46 or later and healthy API/worker/identity/provider. Reviewer has Finance adjuster/REVIEW; approver has independent Finance approver/APPROVE. For an existing installation, use a newly built release from current source and its backup/forward migration procedure. Earlier migration-44/45 archives do not contain this capability.

## 2. Establish authentic movement evidence

For incoming funds, issue a real synthetic receiving reference through the customer payment view or its authorized receiving-reference endpoint. References are random opaque values, not account IDs. For returns, retain the original SIMULATED outgoing payment ID and actual settled Core receipt. Publish the supported signed incoming/return event through `/admin/rail-credits` and deliver its signed packet to `/api/v1/providers/simulated/payments`, following the [rail contract](../../contracts/providers/payments/README.md).

Expected: an accepted receipt first posts to Core suspense. Current controls and destination restrictions determine whether a separate allocation credits the customer. Unknown/restricted receivers retain owned suspense. Returns link to a settled original and cannot cumulatively exceed original principal. A rejected over-cap provider claim can exist at the provider without a bank financial posting; reconciliation must expose that conflict. Workflow success alone is insufficient.

## 3. Prepare an independent credit clearing file

Obtain original movement/evidence identity, kind, reference/original payment, amount, version and date from provider records. The [credit file specification](../../contracts/providers/finance/README.md) lists fields. Create `.local/credit-clearing-file.json` with a new file UUID, actual independent rows and exactly the intended report's half-open UTC period. Preserve the declared file cutoff. Amounts are decimal strings; `providerFeeMinor` is separate from the original outgoing bank fee and is zero under the synthetic policy.

```sh
# The local fixture key is the default. In cloud evaluation provide the
# installation's BANK_FINANCE_SIGNING_KEY through a private environment.
python3 scripts/publish_settlement.py --kind credits .local/credit-clearing-file.json
```

For cloud evaluation, enable the optional provider tunnel in the selected cloud's tunnel terminal, then run the publisher on your workstation with the installation's original Finance key supplied privately:

```sh
# Use one cloud bundle at a time; the existing runbook environment is required.
BANK_TUNNEL_PROVIDER=1 infra/cloud/gcp/tunnel.sh
# Or:
BANK_TUNNEL_PROVIDER=1 infra/cloud/aws/tunnel.sh
```

The default bundle exposes only the five portal/API/issuer ports; it does not include port 4182. The optional provider tunnel is for trusted synthetic-fixture operators. Keep the Finance key out of command-line arguments and logs. See [cloud access boundaries](../deployment/README.md).

Expected: 201 new publication, 200 exact replay, 409 changed content under an existing ID, 422 malformed/bounded data. Up to 100 rows/60,000 canonical bytes per file; use narrower periods within the current limits. A single UTC day exceeding these bounds is unsupported until grouped/larger-file ingestion is implemented. Never remove a provider discrepancy to force a complete result. Corrected evidence uses a new file ID.

## 4. Capture the journal and GL snapshot

Sign into `http://127.0.0.1:4301` as reviewer, open **Finance & GL**, and save a snapshot for exactly the file's period. Inspect both suspense and allocation journals and their GL receipts. Keep the original return batch link. The [reporting runbook](finance-reporting.md) explains immutable membership and independent liquidity status.

Expected: a stable parent report ID/digest with separate Core/Finance clocks. Provider value dates can precede actual bank booking dates. Frozen reports do not automatically acquire later allocations or GL receipts. Liquidity freshness remains a separate concern.

## 5. Capture incoming/return reconciliation

Open the saved report, enter the file UUID under **Reconcile incoming and returned payments**, and choose **Capture credit reconciliation**. Preserve its saved URL. The UI retains a request key during uncertain submission; repeat the same scope after network interruption.

Expected: `INCOMING_RETURNS_V1`, COMPLETE/PARTIAL/EXCEPTIONS, separate received/allocated/still-in-suspense principal, provider controls by currency/scale, and source clocks/cutoff. Every native receipt and explicit provider orphan remains in the union. Exact controls are candidate inventory, including explicit out-of-period identities; they do not claim period cash flow.

## 6. Review lineage and owned cases

Inspect **Frozen credit lineage**, both native journal receipts and linked GL exports. For returns, open the original payment and check original settled principal, payer and lifetime returned principal. Match event/evidence ID/version, original reference/payment, exact amount, currency/scale and dates. Review duplicate provider lines and nonzero provider fees independently.

| Observation | Expected action |
| --- | --- |
| FUNDS_IN_SUSPENSE | Open the Finance case and existing payment credit case; resolve through current controlled allocation workflow |
| CREDIT_PROCESSING_PENDING | Let the durable original workflow recover; capture new evidence after the five-minute ownership grace if unresolved |
| CREDIT_ACK_TIMING | Recover the original workflow acknowledgment; do not allocate a second credit |
| CORE_AFTER_REPORT | Save a fresh report and new run, retaining the earlier timing result |
| CREDIT_BOOKING_DATE_DIFFERENCE / CREDIT_OUTSIDE_REPORT_PERIOD | Review provider value date versus actual bank booking date and report membership; never rewrite journals |
| CREDIT_VALUE_OUTSIDE_FILE_PERIOD | Review prior-period provider value date against the current bank booking period; retain both facts |
| SETTLED_AFTER_FILE | Obtain a later independent file with the authentic provider outcome |
| Missing/duplicate/cap/original/scope/value/currency/fee/date conflict | Retain the artifacts, assign the owned case and investigate original provider/bank evidence |
| GL timing or discrepancy | Use the original GL inquiry/recovery workflow and capture a fresh report after resolution |
| Case assignment pending | Keep worker recovery running, then choose **Refresh credit reconciliations** |
| File unavailable/invalid authentication | Repair the source/dependency; no complete run is saved |

Expected: suspense and durable conflicts have owned Finance cases. The worker attempts fifty pending case assignments per pass and skips failed scopes without starving later issues. Lost acknowledgment reuses the original issue origin. Case disposition does not rewrite saved observations. Financial corrections remain independently authorized Core workflows.

## 7. Reopen and retain the evidence

Reload `/finance/reports/{reportId}/credit-reconciliations/{runId}`. Page through movement lineage and verify the digest. Independently authorized approver can read without capture controls; customer/security contexts cannot use Finance authority. Preserve original file and saved report/run IDs before investigating or publishing later observations.

Expected: the old digest, membership and classifications remain unchanged after later allocation, corrected provider files or new reports. Private signed packets are not exposed in staff responses. Damaged stored evidence fails verification.

## Completion checklist

- [ ] Independent file, authenticated movement and original return payment identities retained.
- [ ] Exact file/report periods and separate source clocks/cutoff understood.
- [ ] Received, allocated and suspense controls reviewed separately.
- [ ] Provider value date and actual bank booking dates retained.
- [ ] Return lifetime principal cap and original payer/settlement verified.
- [ ] Suspense and durable exceptions have owned actions; short grace remains explicit.
- [ ] Reopened digest and stable pages verified; original artifacts retained.
- [ ] No duplicate credit, replacement payment, additional fee or manual journal mutation performed.

Reproduce validation with `make test-integration`, then `make test-payments` sequentially against the shared local PostgreSQL service. The latter covers the real issuer/BFF/customer/staff flow and retained financial state.
