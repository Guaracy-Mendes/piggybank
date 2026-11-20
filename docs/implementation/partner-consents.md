# WP-33 consented partner APIs and delegated account authority

WP-33 adds migration `0067_partner_consents.sql` (schema 67). It covers:
- requirements FR-IAM-05, FR-ACC-08 and FR-EXT-02
- the local branches of AT-14 (delegation revocation) and AT-21

It is an R2 capability built and qualified locally with synthetic partners. Enabling it in production still requires
the R1 launch gate (WP-32) plus actual partner contracts and policy (DEC-06, DEC-09, DEC-11).

## Trust model

| Element | Rule |
| --- | --- |
| Partner identity | A separate `partners` issuer realm with client-credentials clients only. Tokens last 30 s and carry the `bank-partner` audience. Partners have no interactive login, no session and no cookie. |
| Registration | A token is accepted only if its `azp` names a client in the bank registry (`identity.partner_clients`) that is `ACTIVE`. A security administrator proposes; a *different* security administrator or security approver approves. Suspension is immediate. Reinstatement returns to approval, and whoever requested it cannot approve it. Every decision is recorded in history and audited. |
| Consent | The partner requests scopes (`accounts:read`, `payments:initiate`, both bounded by its registration) for 1–90 days. Nothing is granted until a customer authorizes the request in a bank channel within 15 minutes. The customer chooses one to ten accounts they own outright: delegated access can never be passed on to a partner. |
| Partner actor | Built server-side from the consent binding. It carries the consenting customer's issuer and subject, so every ownership check still applies, plus the partner `client_id` and `PARTNER` trust, so audit records and payment intents name both. |
| Enforcement | The API limits each endpoint to its scope. Controls re-checks the live grant (`identity.consent_permits`: client, customer, account, scope, expiry, client state) at every decision. Revocation, expiry and suspension therefore apply to the very next call. |
| Disclosure | A missing, foreign, revoked, expired or out-of-scope consent answers `403` without saying which. An account outside the grant answers `404`. The partner view of a consent never names the customer. Recipient listings give labels, never receiving references. |
| Payments | A partner can quote and draft a payment from a consented account, but cannot start or complete its approval. The customer sees the draft in **Payments** and approves it with action-bound step-up. On confirmation the customer's actor is recorded once as `payments.intents.authorized_actor`. Execution, reconciliation and recovery then act for the customer, while `actor_context` keeps the partner as the immutable initiator. A partner reads only the payments it initiated. |
| Revocation | Prospective (AT-21): new partner calls fail immediately, and already-approved payments continue their own lifecycle to settlement. Customers revoke from **Connected apps & sharing**, security administrators from **Partners & consent**, and partners may withdraw their own consents. |
| Quota | `quotaPerMinute` per client, counted in one statement (`identity.partner_quota_permits`). Over-quota calls answer `429` with `Retry-After: 60`. This applies on top of the per-address ingress quota. |
| Callbacks | `consent.authorized`, `consent.rejected` and `consent.revoked` are queued once per consent and event and delivered by the worker. Each is an HMAC-SHA256 signature over `"<timestamp>.<body>"`, sent as `Piggybank-Signature: t=<unix>,v1=<hex>` with a `Piggybank-Event-Id`. The 32-byte secret is generated at proposal, shown to staff exactly once and sealed at rest (AES-256-GCM, bound to the client). Failed deliveries retry with exponential backoff and become `FAILED` after five attempts. Callback URLs are restricted to loopback in this synthetic profile. |
| Versions | Every call needs `Piggybank-Api-Version`. A missing or unknown version answers `400`. A version past its sunset date answers `410` with `Sunset`. A deprecated version is served with `Deprecation: true` and `Sunset`. Fixtures: `2026-11-01` current, `2026-08-01` deprecated (sunset 2027-02-01), `2026-05-01` sunset. The ingress boundary keeps `Sunset`, `Deprecation`, `Retry-After` and `Allow` through its safe error envelope. |

## Delegated account authority (FR-ACC-08)

- **Granting.** The delegate shares a short-lived reference (`DLG-XXXXXXXX`, valid 24 hours). The owner grants
  `VIEW`, or `PAY` up to a per-payment limit (0.01–100,000.00 TST), for 1–365 days.
- **Authority.** Core lists only current delegations in the account authority, and expiry is evaluated at read
  time. Controls lets a delegate read, and with `PAY` start payments, but never service the account (freeze,
  close, restrict). Payments enforces the per-payment limit against the exact amount when quoting, and again at
  intent creation and confirmation.
- **Revocation.** The owner revokes or the delegate renounces, and it takes effect immediately. Grant terms are
  immutable, and the history records `GRANTED` / `REVOKED` / `RENOUNCED` with the acting person. A delegate's past
  intents keep the delegate as their recorded actor.

## Interfaces

| Audience | Endpoints |
| --- | --- |
| Partner | `POST /partner/v1/consents`, `GET`/`DELETE /partner/v1/consents/{id}`, `GET /partner/v1/accounts`, `GET /partner/v1/accounts/{id}/balance`, `GET /partner/v1/accounts/{id}/transactions`, `GET /partner/v1/beneficiaries`, `POST /partner/v1/payment-quotes`, `POST /partner/v1/payments`, `GET /partner/v1/payments/{id}` |
| Customer | `GET /api/v1/consents`, `GET /api/v1/consents/{id}`, `POST /api/v1/consents/{id}/authorization`, `POST /api/v1/consents/{id}/revocation`, `GET`/`POST /api/v1/delegations`, `POST /api/v1/delegations/reference`, `POST /api/v1/delegations/{id}/revocation` |
| Staff | `GET`/`POST /api/v1/staff/partners`, `POST /api/v1/staff/partners/{id}/decision`, `POST /api/v1/staff/consents/{id}/revocation` |

The customer portal adds **Connected apps & sharing** (`#/sharing`), plus the authorization view at
`#/connected/{id}` that partners link customers to. The staff portal adds **Partners & consent** (`#/partners`).

## Models

| View | Source |
| --- | --- |
| Components | [current-partner-components.puml](current-partner-components.puml) |
| Consent and in-flight revocation | [current-partner-consent.puml](current-partner-consent.puml) |

## Deliberate boundaries

- **Partner onboarding.** Real partner contracts, due diligence and certification (DEC-09), along with legal consent
  wording and retention (DEC-06), are not implemented. The issuer clients are synthetic fixtures provisioned by
  `scripts/identity.py`.
- **Payment scope.** Partners can neither add recipients nor approve payments. Variable recurring payments and
  partner-side payment consents are out of scope.
- **Callbacks.** Callbacks cover consent lifecycle only. Partners poll payment status.
- **Network.** Only loopback callbacks are allowed. Production egress control, mTLS and partner key management
  belong to WP-31 and WP-32.
- **Delegation scope.** Delegation is person-to-person on one account. Business mandates, multi-signature rules and
  power of attorney need DEC-06 and DEC-11 policy.
