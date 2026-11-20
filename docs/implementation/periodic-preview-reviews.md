# Retained periodic preview reviews

Current status: essential synthetic WP-24 is complete. This document retains the narrower increment it introduced; the [financial lifecycle](periodic-finance-r1.md), [scope decision](wp24-essential-scope.md) and [completion evidence](evidence/WP-24-complete.md) describe the completed execution and deliberate deferrals. Earlier references to remaining work below describe that checkpoint.

Date: 2026-11-13. WP-24 was in progress at this checkpoint. Customers can explicitly retain an acknowledgment that they reviewed the exact synthetic preview disclosure. This is historical review evidence. It does not enroll an account, accept financial terms, activate a policy or authorize a financial posting.

## Material ownership and native creation

Core owns the approved policy, account/product binding and retained V1 disclosure renderer. Documents owns the immutable review receipt, private identity attribution, request-key mappings and transactional event. Finance retains policy proposal/approval/publication ownership. No money authority is added to Documents.

Forward migration [0053](../../migrations/0053_periodic_preview_reviews.sql) adds the native V1 material function and Documents records/functions. Earlier migrations 0001–0052 remain unchanged. The Documents runtime role has SELECT on its two new tables and EXECUTE on narrow functions; it cannot insert, update, delete or truncate those tables directly. The Core renderer is not directly exposed to the Documents runtime role through Core schema access; the guarded Documents function invokes it under its fixed native owner.

The POST input contains only `disclosureDigest` and explicit `acknowledged: true`, with a UUID `Idempotency-Key`. It accepts no material text, actor, canonical person, observation time, receipt ID, amount or financial command. Core's native function independently derives the full original V1 material from the exact account/product/published policy, verifies the original native policy hash and requires the displayed material digest to match. Integer-only amount/rate rendering, frozen calendar fields and source labels match the existing V1 endpoint. Explicit ISO formatting and UTC clocks avoid session DateStyle/timezone changes to material.

The application verifies the complete native receipt and V1 material before committing its Documents transaction. The record includes the exact material, material hash, original receipt UUID, server UTC time, fixed acknowledgment statement and a public attribution digest. Private canonical person and original authenticated actor context remain stored outside the public response; the attribution digest binds them to the original receipt ID. Native inquiry independently verifies that link and the complete retained hashes.

## Attribution, concurrency and recovery

Current customer trust, canonical identity and account-read ownership are required. Staff/service/foreign customers do not gain ownership through an inquiry role. Native creation serializes first on issuer/subject/client/request key and then on canonical person/account/policy. It rechecks and locks current identity and active OWNER records after both potentially blocking locks. A revocation committed while a request waits prevents creation, including replay.

There is one V1 reviewed-material receipt per canonical person/account/native policy. Separate keys or login aliases for that same person recover the original receipt rather than creating duplicate proof. Each successful new request key is retained with its exact scope. Reusing a key for different material, policy or account fails; an existing receipt cannot be rebound to a changed material digest. A different person must review independently under their own current account ownership.

Receipt creation, request mapping and the redacted `documents.periodic.preview.reviewed` outbox fact commit atomically. The fact uses the receipt UUID as event/aggregate identity and version 1. It contains receipt/account/policy IDs, material digest and `PREVIEW_REVIEW_ONLY`; it omits the original actor, canonical person and full material. Existing durable Documents publishing delivers it. Broker delivery does not create enrollment or financial completion.

After a lost response, retry the exact original key/body or inquire by account/native policy. Receipt UUID, reviewed time, material and hashes remain unchanged. Core positions/journals and GL exports are unaffected. Native request replay and known receipt inquiry verify the retained record without regenerating it from a current policy.

## Historical material and integrity failures

The original V1 text is retained verbatim, including its statement that reading does not accept terms or activate anything. The explicit review statement says:

> I acknowledge reviewing this exact synthetic preview disclosure. This does not enroll my account, accept financial terms, authorize a charge or interest credit, or activate a policy.

A damaged live policy causes current disclosure inquiry to fail. An intact prior review can still return its independently verified saved material and original proof. That historical record does not certify current policy validity. If the saved document, digest, account/policy link, original timestamp or private attribution is damaged, inquiry fails and the portal clears the previous receipt. It never recomputes/re-hashes damaged material to repair proof.

Database immutability applies to UPDATE, DELETE and TRUNCATE, including ordinary migration-owner statements. Trusted fixture-only corruption tests temporarily disable a specific immutable trigger on disposable records; they never alter financial facts. Retention scheduling, legal holds and financial consent publication remain separate requirements.

## API and customer workflow

| Method and path | Behavior |
| --- | --- |
| `GET /api/v1/accounts/{id}/periodic-disclosures/{policyId}/review` | Current customer owner and own canonical receipt; 404 before any review; retained historical material after source changes; 409 on damaged proof |
| `POST /api/v1/accounts/{id}/periodic-disclosures/{policyId}/review` | Explicit true acknowledgment and exact displayed digest; mandatory non-nil request UUID; original receipt on retry; 201 after commit |

POST uses the existing customer Origin/CSRF boundary. Invalid input is 400/422; scope/material conflicts are 409; absent or foreign resources are hidden. No endpoint exposes private identity attribution or accepts a financial activation flag.

The customer Accounts disclosure screen embeds **Saved preview review**. The checkbox starts unchecked, and the save action is disabled until the verified material has loaded and the user checks the explicit statement. The portal retains the request key after uncertain failures. The original receipt shows IDs, reviewed clock, material/receipt/attribution digests and expandable exact saved material. A separate refresh can recover historical proof even when current disclosure loading fails. Replies are scoped to account/policy and invalidated on unmount; failed saved-record inquiry clears previous proof.

See the [step-by-step runbook](../runbooks/periodic-preview-reviews.md), [checkpoint evidence](evidence/WP-24-preview-reviews.md), [C4 components](current-periodic-review-components.puml), [sequence](current-periodic-review.puml) and [domain](current-periodic-review-domain.puml).

## Remaining financial scope

This increment establishes retained product information and attributable review. It must not be used as periodic financial consent. Material legal terms publication where needed, explicit enrollment/financial acceptance, current activation/eligibility, authoritative economic-period finalization, guarded fee/accrual jobs and their corrections remain open. Future consent must bind its own exact material and activation scope; existing preview reviews cannot be promoted silently.
