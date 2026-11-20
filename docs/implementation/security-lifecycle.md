# WP-28 security, privacy, account recovery and audit lifecycle

WP-28 was implemented on 2026-11-17 as migration `0065_security_lifecycle.sql` (schema 65). It covers:
- requirements FR-IAM-06, SEC-02, SEC-03, SEC-07 and SEC-10, with supporting SEC-05 and SEC-06 evidence
- acceptance scenarios AT-14, AT-16, AT-19, AT-27 and AT-29, for the local branches implemented here

See also the [threat model](../security/threat-model.md) and the [incident runbook](../runbooks/security-incident.md).

## Account recovery (FR-IAM-06, SEC-03)

| Path | Behavior |
| --- | --- |
| Customer **Sign out everywhere** (`POST /api/v1/security/device-revocation`) | Recorded with `AUTHENTICATED_SESSION` verification and executed at once, because it is protective. It revokes every issuer session (browser and native refresh tokens), deletes server-held browser sessions, and sends a SECURITY notice. If risk is HIGH, a reviewable `SECURITY` case is opened. |
| Staff-assisted (`POST /api/v1/staff/security/recoveries`) | `account_support` records how ownership was verified (`VERIFIED_CONTACT` or `VERIFIED_EVIDENCE_REVIEW`) and must target an onboarded customer. A *Lost device* request at LOW risk executes immediately. A *Credential reset*, or any HIGH risk, goes to `PENDING_APPROVAL` with a `SECURITY` case. A `security_approver` who is neither the requester nor the customer approves or rejects it; approval executes. |

**Risk signals** come from durable facts:
- the contact changed within 7 days
- an open monitoring alert on an owned account
- an executed recovery within 30 days

**Execution.** The decision commits first; then the issuer is called with no database transaction held. The executor uses the least-privilege `bank-identity-admin` service client (`manage-users` and `view-users` in the customers realm only). Its secret is re-read from `BANK_IDENTITY_ADMIN_SECRET_FILE` on every use. A reset sets the issuer's `UPDATE_PASSWORD` required action: staff never see or set a credential, and the customer's next login stops at the password update. A failed issuer call leaves the request `APPROVED` so it can be retried.

Requests are immutable apart from forward transitions, and every transition is written to `recovery_history`. Audit entries go to the `recovery` stream.

## Protected session secrets (SEC-02, resolves SF-001)

**Sealing.** Server-held BFF access, refresh and ID tokens are sealed with AES-256-GCM (`ring`). The associated data binds each ciphertext to its row and column.

**Key ring.** Keys are read from `BANK_SESSION_KEYS_FILE` on every use:
- The first key seals; all listed keys open, so rotation needs no restart.
- Removing an old key makes its sessions fail closed.
- Legacy plaintext rows are re-sealed on their next refresh.
- The local synthetic profile uses a documented synthetic key; production custody (KMS or HSM) is WP-31.

## Privacy requests (AT-16, SEC-05)

The WP-25 data requests gain two kinds.

**`RESTRICTION`** records `MARKETING` and `ANALYTICS` restrictions. A restricted purpose stays disabled even if a preference later re-enables it. Service and security notices are `RESTRICTED_LEGAL_BASIS`, and financial records are `RESTRICTED_RETENTION`.

**`EXPORT`** produces a `DOCUMENTS_PERSONAL_DATA_EXPORT_V1` manifest of the subject's own records only:
- application profiles
- verified contact
- terms acceptances
- preferences and restrictions
- statements

Identity evidence content is `RESTRICTED_EVIDENCE`; only its count is shown. The manifest is kept under the `DATA_EXPORT` retention policy and can be disposed through the existing retention path.

## Access recertification (FR-CTRL-06, SEC-10)

1. A `security_admin` opens a review that snapshots every active grant.
2. A `security_approver` decides **Keep** or **Revoke** for each item, with a reason. Self-review is refused.
3. **Revoke** takes effect immediately on every database-checked action.
4. Decisions are final, and the review closes when every item is decided.

## Security findings register (SEC-07, SEC-10)

A `security_admin` records findings: reference `SF-NNN`, severity, area and a sanitized description. A different `security_approver` disposes each one as **Resolved** with an evidence reference, or as **Accepted risk**. Accepted risk is allowed for low and medium severity only. Open high or critical findings are counted as release-blocking. Dispositions are final.

## Sensitive-data inspection (AT-19, SEC-05)

`tests/acceptance/security/security.py` captures every service log from a full journey (`BANK_FIXTURE_LOG_DIR`) and asserts that none of the seeded values appears:
- the password
- the access, refresh and ID tokens
- the rotated client secret
- the full name, email and evidence text

## API and clients

The 11 routes are listed in the OpenAPI contract under `/api/v1/security/*` and `/api/v1/staff/security/*`.

- **Customer portal:** **Profile → Lost a phone or shared a device? → Sign out everywhere**, with recovery history. **Statements & documents** offers **Export my personal data** and **Stop marketing and analytics use**.
- **Staff portal:** **Security** has the recovery queue (request, approve or reject), access recertification and findings.

## Models

| View | Source |
| --- | --- |
| Recovery sequence | [current-security-recovery.puml](current-security-recovery.puml) |
| Domain | [current-security-domain.puml](current-security-domain.puml) |

## Deliberate boundaries

- Real MFA and phishing-resistant staff factors, a production recovery policy, regulator notification and the on-call rota are DEC-06, DEC-10 and DEC-12 inputs.
- Network policies for deployed trust zones belong to the WP-29 and WP-31 deployment packages. The local profile documents the zones in the threat model.
- Key custody in KMS or HSM, certificate rotation and dependency scanning in CI are WP-29 and WP-31.
