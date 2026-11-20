# Periodic preview review runbook

Use synthetic accounts and the current review-capable release. This workflow saves reviewed information; it creates no financial enrollment, fee or interest credit.

## Prepare

1. Follow [local setup](local-development.md) or the [GCP](../deployment/gcp/RUNBOOK.md)/[AWS](../deployment/aws/RUNBOOK.md) runbook and workstation tunnels. The review increment requires schema 53 and its API/customer bundle.
2. Open an account through the customer journey. Retain the account UUID and exact product version.
3. Follow [policy publication](periodic-accounting-policies.md) with independent current Finance maker/checker identities. Publish an exact product-bound PREVIEW_ONLY rule; a staged proposal alone is not published material.
4. Sign in as the current customer owner and follow [disclosure review](customer-periodic-disclosures.md). Keep the account/native policy URL and displayed disclosure digest.

## Save and recover reviewed material

1. Open **Accounts**, select the account, then the exact periodic preview. Expected: verified amount/rate, calendar and material digest.
2. Locate **Saved preview review**. Expected: the acknowledgment checkbox is unchecked and **Save review acknowledgment** is disabled.
3. Read the entire fixed statement, including its explicit absence of enrollment, financial consent, charges, interest credits or activation. Check it only when you want to retain this reviewed preview.
4. Select **Save review acknowledgment**. Expected: original receipt reference, server UTC reviewed time, exact material digest, receipt digest and attribution digest. The response is historical review evidence, not a financial receipt.
5. Expand **Exact saved preview material**. Compare its rule/version, amount/rate, account and text with the original information. Store the URL/digests in the normal private evidence location.
6. Reload the saved account/native policy URL. Expected: the same receipt ID, clock, material and hashes. No second acknowledgment is created by GET.
7. If the save response is lost, retry with the original retained key/body or select **Refresh saved review**. Expected: recover the same native receipt. Do not change the digest to bypass an error.
8. A second login alias for the same canonical owner may recover the same receipt. A different person must have current ownership and their own review; sharing a URL does not grant access.

Current owner revocation denies both new creation and recovery through that owner. Another customer receives 404; workforce access does not become customer authority. Explicit false acknowledgment, missing/nil key or changed digest fails without creating a review.

## Direct protocol

Use an authenticated customer session without printing bearer/session/CSRF credentials:

```http
GET /api/v1/accounts/<owned account UUID>/periodic-disclosures/<native policy UUID>/review

POST /api/v1/accounts/<owned account UUID>/periodic-disclosures/<native policy UUID>/review
Idempotency-Key: <non-nil request UUID>
Content-Type: application/json

{"disclosureDigest":"<exact displayed SHA256>","acknowledged":true}
```

Expected: GET before a review returns 404; a valid save returns 201 and its immutable receipt. Preserve the original key/body through unknown responses. Browser POST also requires the normal Origin/CSRF session protection. The API accepts no actor, material text, clock, receipt ID or enrollment flag.

## Diagnose failures

- **401:** sign in again and reopen the saved account/policy URL.
- **404:** check current canonical ownership, exact IDs and whether a review was ever saved. A foreign customer is intentionally indistinguishable from a missing resource.
- **409:** preserve original IDs/key/digests. Investigate a scope conflict, changed live material or damaged retained proof. Do not edit native tables or regenerate a hash to force success.
- **400/422:** provide the original non-nil UUID key, exact lowercase digest and explicit true acknowledgment.
- **Live source failure with intact saved review:** the portal may show historical material while current disclosure inquiry fails. The saved record does not establish current policy validity or financial activation.
- **Saved-proof integrity failure:** previous receipt/material must disappear after its refresh fails. Escalate through existing evidence/audit operations; preserve the original record.

Completion checks: exact verified preview; explicit checked statement; one original receipt; stable retry/time/hash; private actor metadata absent from responses/events; unchanged native money; no financial enrollment or activation inferred. Documents outbox delivery is an evidence event, not a financial command.
