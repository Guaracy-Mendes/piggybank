# Runbook: independently approve and execute a financial correction

Use this runbook for a full reversal of an eligible original `SYNTHETIC_FUNDING`, `PERIODIC_FEE` or `PERIODIC_ACCRUAL` journal. Periodic originals require schema 56 and the exact native period receipt; see the [periodic financial runbook](periodic-finance-r1.md). It creates one new linked Core financial journal. It preserves the original, respects held funds/restrictions and requires separate current maker/checker people. See the [implementation specification](../implementation/finance-corrections.md).

## 1. Prepare matching runtime and identities

Follow [local development](local-development.md), or the complete [GCP](../deployment/gcp/RUNBOOK.md)/[AWS](../deployment/aws/RUNBOOK.md) deployment runbook and normal workstation tunnels. Use matching binaries and migrations at schema 50 or later; the current release uses schema 56. Older sealed schema-49 releases retain policy previews but do not contain this correction workflow.

Confirm API/worker/provider readiness and the workforce portal at `http://127.0.0.1:4301`. Prepare separate Finance maker/checker identities with current `finance_adjuster`/REVIEW and `finance_approver`/APPROVE relationships. Local fixtures use `reviewer` and `approver`; use installation credentials. Security administration alone does not grant Finance inquiry or execution.

Expected result: **Financial corrections** appears for Finance inquiry users. The maker has request controls; the checker can inspect and approve an independent source.

## 2. Identify the exact original financial journal

Use an existing mistaken synthetic funding journal's original Core UUID. The Finance/GL workbench shows source batch identity, exact native source and amounts. Funding originals have one credited CUSTOMER account and one debited SETTLEMENT account. Native periodic originals instead have the exact approved CUSTOMER/CONTROL fee or accrual legs. Zero accruals have no financial journal to reverse.

For a disposable demonstration, first open a synthetic customer account through the normal customer portal. Record its Core account UUID and balance. On the local workstation, after matching builds, create a native synthetic funding fixture:

```sh
OPERATION=$(python3 -c 'import uuid; print(uuid.uuid4())')
ACCOUNT=replace-with-the-owned-synthetic-account-uuid
scripts/cargo.sh run --locked --bin bank-fund -- "$OPERATION" "$ACCOUNT" 2500
```

The command prints the **original funding journal UUID**. Preserve it. It credits exactly 2,500 TST minor units (TST 25.00) and records the balanced settlement leg. This is synthetic fixture funding, not real money. Do not run it merely to resolve a live uncertain payment or hold.

For a cloud evaluation, enter the installed VM using the selected cloud's operator access and run the packaged guarded CLI without copying credentials out of the installation:

```sh
sudo -i
cd /srv/piggybank/current/deployment
OPERATION=$(python3 -c 'import uuid; print(uuid.uuid4())')
ACCOUNT=replace-with-the-owned-synthetic-account-uuid
docker compose --env-file /srv/piggybank/state/.env --env-file release.env \
  -f compose.yaml exec -T api bank-fund "$OPERATION" "$ACCOUNT" 2500
exit
```

Expected result: one original journal, exact customer increase and unchanged prior history. Packaged runtimes from schema 50 onward include `bank-fund`. The CLI cannot choose arbitrary posting kinds, mutate journal history or create real currency.

## 3. Stage the original request

Sign in as the Finance maker. Open **Financial corrections**, enter the original eligible financial journal UUID and a precise 4–240-character correction reason. Select **Save financial correction request**.

Expected result: a durable saved request URL, original execution reference, frozen source, native fingerprint, typed source digest and exact reversing legs. For a 2,500-minor-unit source, the customer reversing leg is `-2500` and settlement reversing leg `2500`. Registration progresses from STAGED to PENDING; automatic recovery does not execute a financial posting.

Preserve the original reference/key after uncertainty. The portal retains the same actor/material-derived key for retries. A changed reason is a different proposal and still cannot correct an already corrected source.

## 4. Independently review and approve

Open the saved URL in a separate session authenticated as the Finance checker. Compare original batch/operation/business date, source kind/fingerprint/digest, both account IDs, original/reversing signed amounts, exact reason, maker identity, approval digest and expiry.

Select **I reviewed the original source, exact reversing legs, reason and digest**, then **Approve financial correction**.

Expected result: APPROVED, a distinct canonical checker and an unchanged financial ledger. A second login for the maker's same canonical person is not independent. Wrong purpose/digest, stale or revoked grants/queue, and expired approval are rejected. The checker has no execution button.

## 5. Execute through the original maker

Return to the original maker's saved URL and select **Refresh corrections**. Review the final source and approved legs again. Select **I intend to reverse this exact synthetic posting and apply the displayed reversing legs to the customer account**, then **Execute approved correction**.

Expected result: Core locks the original source and all affected accounts, rechecks live approval/grants/queues after waits, applies availability/restriction checks and atomically creates one balanced linked CORRECTION journal, receipt and outbox event. For an unspent 2,500-minor-unit demonstration, the customer balance returns to its value before the demonstration funding. Original source entries remain unchanged.

## 6. Recover an interrupted response

If the execution response is lost or unavailable, retain the saved URL and original request. Refresh it before taking another action. `EXECUTED` requires the exact original Core receipt, journal, operation, source link and approved legs. Replaying the original request recovers that receipt and cannot create a second correction.

Expected result: the **Native correction receipt** panel shows one correction journal and original execution reference. An unconfirmed observation or unavailable inquiry is not proof of failure. Do not create replacement financial operation IDs, edit receipts or release held funds to force a result.

## 7. Verify native and GL lineage

Compare the customer booked/held/available balances with the starting fixture and receipt. Use **Original GL export** and, once discovery stages it, **Correction GL export** to inspect the two separate immutable exports. A booked GL receipt is delivery evidence; native financial completion comes from Core's correction receipt.

Expected result: one original eligible journal, one new linked CORRECTION and one correction per original across all competing requests. Original journals, finalized periodic basis, prior reports and reconciliation snapshots remain unchanged. The new financial event is exported by the existing pipeline with its original-source link. Refresh the saved request to pick up a newly staged GL link.

## 8. Handle restrictions and conflicting sources

| Observation | Required next step |
| --- | --- |
| Held funds, insufficient availability or debit/credit restriction | Preserve the approved request and financial evidence; investigate the actual account state. The command leaves the hold and money unchanged when blocked |
| `SOURCE_ALREADY_CORRECTED` | Inspect the existing linked correction; this request cannot post another reversal |
| PENDING/STAGED persists | Check worker/readiness and original maker's current REVIEW authority; registration recovery wraps past failures |
| 403 | Recheck workforce session/Origin/CSRF, current grants/queue, canonical maker/checker and original executor |
| 409 | Reopen exact scope; check approval expiry, digest/purpose, source winner, available funds, restrictions or an original-reference retry after UTC midnight |
| 422 | Verify an original synthetic funding or native periodic source, exact field format and reason; payments, credits/returns and other corrections are ineligible |
| Native receipt query unavailable | Retain the original execution reference and restore the owned dependency before retrying |

Use the periodic posting workflow to assess a fee or credit interest. This correction workflow only reverses an existing eligible native journal; it cannot refund a settled provider payment, allocate suspense or reverse a return. Those actions require their own financial workflows and evidence.

## Completion checklist

- [ ] Exact original source, both accounts, signed legs, reason and digest reviewed.
- [ ] Independent current canonical checker approved the same scope.
- [ ] Original current maker explicitly requested execution.
- [ ] Original native receipt/source/journal references retained after any interruption.
- [ ] Customer availability and held funds verified; no hold was released by correction.
- [ ] Exactly one linked correction and original/correction GL lineage inspected.
- [ ] Saved desktop/mobile URL reopens the accepted immutable evidence.

For reproduction use the [checkpoint evidence](../implementation/evidence/WP-24-corrections.md). Shared PostgreSQL integration and HTTP/browser suites run sequentially; the provider deadline and browser quota remain unchanged.
