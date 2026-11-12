# Finance report revision runbook — synthetic local profile

Use **Finance & GL → Controlled report revisions** to link a later observation as the approved replacement for an earlier report. Keep the original snapshot URL and digest. Publication preserves both frozen artifacts; it records the approved relationship and reason.

## Prepare a concrete revision

1. Save and inspect the original report and a later replacement for the same UTC period. Verify source pages, exact controls, captured cash status and any owned snapshot issues. The replacement must retain all original source batches.
2. As the Finance adjuster, select both snapshots and enter the specific reason. Review the artifact IDs/digests and exact control changes shown after acceptance. A `202` response means the request is durable; it does not mean approved or published.
3. Preserve the idempotency key if the acceptance response is uncertain. The browser retains it until successful acknowledgment. Replay must use the same reports and exact reason. Changed input conflicts.
4. Wait for registration to reach PENDING. The worker recovers a lost Controls acknowledgment under the accepted request ID. Registration metadata explains pending current-authority/dependency failures. The background cursor advances past blocked requests and wraps for later retry.

## Check independently and publish

1. A different canonical person signs into workforce with the current Finance approver role and Finance APPROVE queue membership. Open both frozen artifacts, inspect the reason, digests and deltas, confirm review, then choose **Approve report revision**.
2. A second login alias for the maker cannot serve as the checker. An expired request requires a fresh accepted request and approval. Restoring a role does not extend the original expiry or change its scope.
3. The original canonical maker refreshes the request and chooses **Publish approved revision**. The current maker/checker grants and required queue relationships must still permit the action.
4. Reopen both report URLs. Their published-lineage wrappers must show the same approved link and reason; each artifact digest and captured membership must remain unchanged. Retain the independent checker and publication receipt.
5. If the publication response is lost, retry the same request and digest or read its receipt. Replays recover the original publication time. Competing proposals for an already revised original conflict; further revision must continue from the published successor using a later snapshot.

## Completion checklist

- Both complete artifact digests and the exact approved reason match the accepted scope.
- The checker is a distinct canonical human with current approval role and queue authority.
- The published receipt identifies the original maker, independent checker, both reports and original publication time.
- There is one successor and one predecessor per published link, with no cycle or cross-period change.
- Both report digests, frozen source membership, Core journals, balances and held funds are unchanged by publication.
- Registration and publication recover their original IDs after lost acknowledgments; audit/history is retained.

`make test-integration` covers concurrent acceptance/publication, aliases, wrong-purpose proof, revocation/expiry, immutable artifacts, acknowledgment recovery and registration pagination. `make test-payments` exercises separate real issuer/workforce identities and the complete review/publication browser flow. These synthetic checks do not approve a statutory filing or execute a financial adjustment.
