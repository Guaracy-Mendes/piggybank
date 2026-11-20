# WP-34 assisted channels: branch, private banker and call center

WP-34 adds migration `0068_assisted_channels.sql` (schema 68). It covers FR-CH-03, FR-CH-04 (the assisted-channel
part) and the local branches of AT-22, as a **local synthetic R2 capability**. ATM integration is WP-35. Enabling any
operation in a real channel requires DEC-09 (exact operations per channel) and channel certification.

## Rules

| Rule | Implementation |
| --- | --- |
| Explicit actor and customer | A staff member with `assisted_service` opens a session for a named customer, recording the channel, the verification method and the purpose. The session actor keeps the staff identity. It names the customer as `represented_party`, uses `assisted-<channel>` as its client and carries `assisted:<session>`. Authorization audit records therefore name the staff actor and the represented customer, and the session holds the channel. |
| Identity assurance cannot be bypassed | Each channel permits specific verification methods: branch `ID_DOCUMENT_SIGHTED`; private banker `KNOWN_RELATIONSHIP` or `ID_DOCUMENT_SIGHTED`; call center `VERIFIED_CONTACT_CALLBACK`. Nothing is possible until the customer authorizes the session in their own bank channel within 10 minutes. |
| Confirmed operations only | `controls.channel_policies` is an immutable synthetic fixture. Every channel enables `READ_ACCOUNT` and `PROTECTIVE_FREEZE`; **`DRAFT_PAYMENT` is enabled in no channel** until DEC-09 confirms it. Sessions last 30 / 60 / 15 minutes. |
| Common restrictions | Every decision goes through Controls, which calls `controls.assisted_permits` for the live session (state, expiry, staff and customer parties), the channel's operations and the customer's own account, and applies the account's restriction flags. A protective freeze is the same Core restriction a customer's own channel sees, so the customer's payments fail consistently. |
| Recorded authority | Every decision, allowed or denied, is appended to `controls.assisted_actions` with a reason (`CHANNEL_POLICY`, `SESSION_NOT_ACTIVE`, `OPERATION_NOT_ENABLED_FOR_CHANNEL`, `SESSION_PARTY_MISMATCH` or `NOT_CUSTOMER_ACCOUNT`). Session transitions are kept in history, and opening and authorization are audited on the `assisted` stream. |
| Handoff | Another staff member cannot reuse the session, and a forged represented party is refused. The customer can end the session at any time, and staff close their own. |

## Interfaces

| Audience | Endpoints and pages |
| --- | --- |
| Staff | `GET`/`POST /api/v1/staff/assisted`, `POST /api/v1/staff/assisted/{id}/closure`, `GET /api/v1/staff/assisted/{id}/accounts`, `POST /api/v1/staff/assisted/{id}/freeze`. Staff portal: **Assisted service** (`#/assisted`). |
| Customer | `GET /api/v1/assisted`, `POST /api/v1/assisted/{id}/authorization`, `POST /api/v1/assisted/{id}/closure`. Customer portal: **Assistance requests** on **Connected apps & sharing**. |

The model is [current-assisted-session.puml](current-assisted-session.puml).

## Deliberate boundaries

- **Channels.** No real branch, private-banker or telephony system is integrated, and there is no channel certification
  (DEC-09).
- **Verification.** The verification methods are recorded attestations, not integrated document or voice checks.
- **Operations.** Assisted payments, onboarding and servicing changes stay disabled until DEC-09 names them.
- **Lifting a freeze.** A protective freeze applied through a channel is lifted by staff support. Customer
  self-service unfreeze for staff-imposed restrictions follows the existing servicing rules.
