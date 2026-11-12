# Reconcile outgoing payments and clearing files

Use synthetic data and the [local runtime](local-development.md). For tunneled GCP/AWS evaluation retain the five fixed workstation ports and use that installation's keys. This procedure observes and assigns differences; financial correction or recovery remains an independent authorized workflow.

## 1. Start the matching code and dependencies

```sh
make dev-up
make migrate
make identity-setup
make dev-seed
make dev
```

Expected: the current schema (migration 45 or later), API/worker readiness, issuer and provider are available. Local reviewer has Finance adjuster/REVIEW and approver has Finance approver/APPROVE. Keep identities independent. For an existing cloud installation, deploy a newly built release containing migration 45 rather than reusing the earlier migration-44 archive; preserve backups and forward migration discipline.

## 2. Establish original financial facts

Use the customer portal for an internal payment or an enabled SIMULATED external payment. Preserve its original payment ID, quote, principal/bank fee and provider instruction identity. For an external payment, an ACCEPTED/PENDING acknowledgment is not settlement. Apply the supported authenticated synthetic finality flow from the [rail contract](../../contracts/providers/payments/README.md), then verify the original Core journal and GL export receipt in the staff view.

Expected: actual settlement has one journal, exact original bank fee and immutable final evidence. Unknown/submitted outcomes retain their holds; this reconciliation must not release or resubmit them. Existing [payment recovery](payment-recovery.md) and [GL recovery](finance-recovery.md) procedures apply to uncertain outcomes.

## 3. Prepare and publish an independent clearing file

Obtain rows from the provider's own final records. Use the field contract in the [clearing specification](../../contracts/providers/finance/README.md), including original final evidence ID/version. Do not derive provider claims from bank balances or copy a projected success flag. Put the explicit JSON file in `.local/clearing-file.json` with a new UUID, actual rows, the selected UTC period and cutoff. Keep `providerFeeMinor` separate from the original customer bank fee.

```sh
# Local fixture key is the default. For cloud evaluation supply the original
# BANK_FINANCE_SIGNING_KEY through a private environment, not a command-line argument.
python3 scripts/publish_settlement.py .local/clearing-file.json
```

For cloud evaluation, enable the optional provider tunnel in the selected cloud's tunnel terminal, then run the publisher on your workstation with the installation's original Finance key supplied privately:

```sh
# Use one cloud bundle at a time; the existing runbook environment is required.
BANK_TUNNEL_PROVIDER=1 infra/cloud/gcp/tunnel.sh
# Or:
BANK_TUNNEL_PROVIDER=1 infra/cloud/aws/tunnel.sh
```

The default bundle exposes only the five portal/API/issuer ports; it does not include port 4182. The optional provider tunnel is for trusted synthetic-fixture operators. Keep the Finance key out of command-line arguments and logs. See [cloud access boundaries](../deployment/README.md).

Expected: 201 for a new file, 200 for exact replay. Changed content under the same ID returns 409; malformed/bounded inputs return 422. Record the file ID and retain the original file. Publish a corrected file with a new ID rather than editing old provider evidence.

## 4. Capture the parent report

Sign into `http://127.0.0.1:4301` as reviewer and open **Finance & GL**. Save a reporting snapshot for exactly the clearing file's half-open UTC period. Inspect its journal membership and GL controls. Short asynchronous GL work can remain inside its timing window; do not label a pending snapshot as fully matched. The [reporting runbook](finance-reporting.md) explains frozen membership and liquidity independently.

Expected: the parent artifact has a stable ID/digest and explicit Core/Finance clocks. Current cash freshness does not affect whether outgoing payment evidence matches; it remains separately visible.

## 5. Capture payment reconciliation

Open the saved report, enter the published file UUID under **Reconcile outgoing payments**, and choose **Capture payment reconciliation**. Preserve the resulting URL/reference during a network failure and retry with the UI's retained request key. Never invent replacement original payment/evidence identities to get a successful match.

Expected: the run displays `COMPLETE`, `PARTIAL` or `EXCEPTIONS`, exact matched/posted controls, separately partitioned provider totals and source clocks. Inspect every exception/timing/pending row. `NOT_POSTED` describes a terminal instruction with no contradictory financial effect. An empty outgoing period explicitly contains zero rows. This scope does not reconcile incoming funds, returns, liquidity or periodic accruals.

## 6. Review and assign differences

Open **Frozen payment lineage**, the original payment, captured GL export and owned Finance case links. Confirm principal, currency/scale, payer/payee, original bank fee, actual settlement date, provider evidence identity/version, journal legs and captured GL receipt. Provider totals retain duplicate rows. A nonzero provider clearing fee does not become a new bank charge.

| Observation | Expected action |
| --- | --- |
| Missing/duplicate/value/currency/account/fee conflict | Retain source artifacts; assign the owned case; investigate the original identities |
| Core settled after the report | Save a fresh report and capture a new run; retain the old timing result |
| Settlement after file cutoff | Obtain a later independent file; use a fresh report/run |
| Pending original outcome | Use original-identity inquiry/cancellation recovery; preserve uncertain holds |
| GL timing window or lost workflow acknowledgment | Wait for existing durable recovery, then capture fresh evidence |
| Pending case ID | The worker retries up to fifty assignments per pass; lost acknowledgments reuse the same Controls origin |
| Provider file unavailable/unauthenticated | Repair the dependency or source file; no complete reconciliation is saved |

Expected: each durable classified issue acquires a Finance operations case. Bad scope/assignment records do not starve later items. The UI can show pending ownership; use **Refresh reconciliations** to retrieve the latest case links. Case disposition does not mutate the old run. If correction is needed, use its existing independently approved financial workflow; this page has no money command.

## 7. Reopen and verify retained evidence

Reload the saved `/finance/reports/{reportId}/reconciliations/{runId}` URL. Inspect first/next pages and artifact digest. Read-only approver can inquire, with no capture control; security/customer contexts cannot use Finance actions. A later corrected file/run must leave the old report/run digest unchanged.

Expected: old lineage and classification remain stable through replay and corrected later observations. Private authenticated provider packets are not returned in staff API pages. Any hash/authentication damage fails closed rather than returning a misleading complete result.

## Completion checklist

- [ ] Original instruction and actual final evidence retained; no replacement payment submitted.
- [ ] Parent report/file periods agree; every separate source clock/cutoff is understood.
- [ ] Bank/provider fees and currency partitions are reviewed separately.
- [ ] Missing, duplicate and timing classifications have owned next actions or explicit short grace.
- [ ] Reopened run and item pages retain their original digest/membership.
- [ ] Later corrected evidence leaves prior artifacts intact.
- [ ] Financial journals, balances and holds change only through independently authorized Core workflows.

Local verification: `make test-integration` includes the new reconciliation cases; `make test-payments` exercises the staff matching workflow with the existing payment/GL scenarios. Run database and HTTP/browser suites sequentially on the shared local PostgreSQL server.
