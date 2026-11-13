# Periodic disclosure runbook

This synthetic workflow explains approved preview rules for an owned account. It creates no enrollment, accepted terms, fee debit or interest credit.

## Prepare approved material

1. Complete [local development](local-development.md), or the [GCP](../deployment/gcp/RUNBOOK.md)/[AWS](../deployment/aws/RUNBOOK.md) deployment runbook and normal workstation tunnels. Use the current disclosure-capable release; schema 52 alone does not establish that this code/UI is installed.
2. Open an account through the normal customer journey and retain its UUID/product version. Do not use a bare ledger-only UUID.
3. Follow [periodic policy publication](periodic-accounting-policies.md) with independent current Finance maker/checker people. Publish a monthly fee or daily credit policy for that exact product. Expected: original native policy/digest and PREVIEW_ONLY activation.
4. Keep customer and workforce sessions separate. Local synthetic customer credentials are `alice`/`bob`, password `Synthetic-Local-123!`; a VM uses the installation's privately retrieved fixture password.

## Read as the account owner

1. Sign in at `http://127.0.0.1:4300`, open **Accounts**, select the account and locate **Periodic rule previews**.
2. Review the current/upcoming list. An empty successful list means no current/upcoming policies were published for this account product. An error is not evidence that no policy exists.
3. Open a rule. Read its exact amount/rate, covered period, day-count/rounding, declared weekdays/holidays, UTC cutoff and due-date roll. Monthly amounts include exact minor units; daily rules explain the formula without supplying or claiming a captured balance.
4. Open **Version and verification**. Record native rule UUID, policy digest, disclosure digest and format version. The source includes the full approved immutable policy/calendar while omitting private approval people and operator notes.
5. Save the URL `#/accounts/<account UUID>/periodic/<native policy UUID>`. Reload it or open it before a fresh customer login. Expected: return to that account/rule and the same material digest. The observation clock can change without changing the document.
6. If a new future version is published, inspect both versions separately. Expected: the original saved version/digest is unchanged; the new version has its own material and upcoming label. An ended version can remain readable through its saved link.
7. Confirm there is no acceptance/enrollment/activation control in this workflow. Reading must not be represented as consent or a financial receipt.

Expected: another customer receives 404 for the account/rule. Workforce access does not grant customer-channel ownership. Current owner revocation or a damaged native policy causes a failed inquiry and clears prior material.

## Direct inquiry

Use an authenticated customer session/bearer without printing credentials:

```http
GET /api/v1/accounts/<owned account UUID>/periodic-disclosures
GET /api/v1/accounts/<owned account UUID>/periodic-disclosures/<native policy UUID>
```

Expected: owner-bound material and its digest; no balances, approval people/reason or audit payload. All money in structured policy data remains exact decimal strings. The server supplies observation time; there is no caller-supplied basis or clock.

If more than 100 current/upcoming rules exist, list returns 422 with no partial document set. Use a known saved rule URL or contact support; the individual inquiry still applies the same owner/product checks. Do not infer that the first 100 would be complete.

## Diagnose discrepancies

- 401: reauthenticate in the customer channel and retain the saved URL.
- 404: check current ownership, account/product identity and actual native publication. A staged workforce request is not a published rule.
- 409/integrity refusal: preserve original IDs/digests and investigate native policy storage. Do not edit/re-hash material to force a display.
- New observation clock with the same digest: expected. The clock and availability label are outside material hashing.
- Changed policy/template/account: require its own disclosure digest and future explicit acceptance controls; do not reuse a prior acknowledgment.

Completion checklist: approved exact product policy; current owner; full material reviewed; original native policy/disclosure digests retained; saved URL/re-login verified; private workforce metadata absent; no consent/enrollment/charge/credit inferred.
