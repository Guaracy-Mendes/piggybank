# Security incident runbook

This runbook is for the local synthetic profile (WP-28). It names roles, not people. A real incident policy, regulator notification duties and the on-call rota are DEC-06 and DEC-12 inputs and remain pending.

## Roles

| Role | Seeded local user | Responsibilities |
| --- | --- | --- |
| Incident lead | `security` (`security_admin`) | Opens the incident, records findings, opens access reviews |
| Independent approver | `approver` (`security_approver`) | Approves recoveries and disposes findings, never their own |
| Support | `reviewer` (`account_support`) | Verifies customer ownership and requests recovery |

## 1. Detect and triage

- Watch **Operations monitor** (WP-26) for data-quality alerts, monitoring alerts, provider backlog and failed messages.
- Review recent recovery requests flagged **High risk** in **Security**. Their signals are a contact changed within 7 days, an open monitoring alert, or a recovery within the last 30 days.
- Use **Trace an operation** for durable identifiers. Never copy tokens, passwords or evidence into tickets.

## 2. Contain

| Situation | Action |
| --- | --- |
| Compromised customer device | Ask the customer to use **Profile → Sign out everywhere**. Otherwise support requests a *Lost device* recovery after verifying the contact. This revokes every issuer session (browser and native refresh tokens) and sends a security notice. |
| Compromised customer credential | Support requests a *Credential reset*. An independent security approver approves it, and the customer must set a new password at the next sign-in. Nobody sees or sets the password. |
| Compromised staff access | The security admin opens an access review. A different approver revokes the grant, which takes effect immediately on every database-checked action. |
| Compromised recovery executor secret | Rotate the `bank-identity-admin` client secret in the issuer and write the new value to `BANK_IDENTITY_ADMIN_SECRET_FILE` (mode 0600). The API reads it on next use, with no restart, and the old secret is rejected. |
| Session key exposure | Add a new `kid:base64(32 random bytes)` as the **first** line of `BANK_SESSION_KEYS_FILE` and keep the old line, so existing sessions still open. New sessions and refreshes seal with the new key. After 30 minutes (the session lifetime) remove the old line; any remaining old-key session then fails closed. |
| Payment rail compromise | Use the existing emergency rail pause (WP-21). Unknown outcomes keep their holds. |

## 3. Eradicate, record and recover

1. **Record.** Enter a finding in **Security → Security findings** with a reference (`SF-NNN`), severity, area and a sanitized description.
2. **Fix and dispose.** Fix it, then have a different security approver record **Resolved** with an evidence reference. Only low and medium findings may be recorded as **Accepted risk**. Any open high or critical finding blocks release.
3. **Recertify access.** Re-run the access review if privileged access may have been involved.

## 4. Tabletop exercise record (2026-11-17)

Scenario: a customer reports a stolen phone, and an attacker attempts a credential reset through support.

1. **Customer revokes the device.** The customer signs out everywhere from the web portal. All browser and native sessions are revoked and a security notice is sent. Verified by `tests/acceptance/security/security.py`.
2. **Support requests a reset.** It is recorded with `VERIFIED_CONTACT` and enters `PENDING_APPROVAL`. Support cannot approve it.
3. **Independent approval executes.** A different approver approves; the issuer logs out every session and requires `UPDATE_PASSWORD`. Login stops at the password update.
4. **High-risk path.** A contact changed yesterday makes a staff lost-device request high risk. The approver rejects it and asks for a call-back on the old contact (database test).
5. **Secret rotation.** The executor secret is rotated through its file during the exercise; the old secret is rejected and execution continues without restart.
6. **Post-incident access review.** Run by `security`. A grant was revoked, and self-review was refused (database test).

Closure: no open high or critical findings remain. SF-001 (plaintext server-held tokens) was resolved during WP-28.
