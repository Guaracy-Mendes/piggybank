# WP-28 security acceptance manifest

| Scenario branch | Executable evidence |
| --- | --- |
| AT-14 / FR-IAM-06: actual issuer revocation of browser and native sessions on lost device; security notice | `tests/acceptance/security/security.py` |
| Staff credential reset: recorded verification, independent approval, issuer `UPDATE_PASSWORD`, no credential disclosure | `security.py`; `crates/documents/tests/cases/security.rs` `recovery_verifies_ownership_scores_risk_and_requires_independent_approval` |
| Suspicious recovery (contact changed, prior recovery) is high risk and reviewable; rejection path | `security.rs` (database) |
| SEC-02: executor secret rotated through a file, old secret rejected, no restart | `security.py` |
| SEC-02: session tokens sealed at rest (AES-256-GCM, row-bound, key ring rotation) | `crates/identity/src/seal.rs` unit test; `tests/integration/identity.py` (`tokens_sealed_at_rest`) |
| AT-19 / SEC-05: seeded sensitive values absent from all service logs | `security.py` |
| AT-16: restriction honored over preferences; own-subject personal export; evidence restricted | `security.rs` `privacy_restriction_and_personal_export_honour_purpose_and_subject_boundaries` |
| FR-CTRL-06 / SEC-10: access recertification with no self-review and immediate revocation | `security.rs` `access_review_and_findings_need_independent_attributable_decisions` |
| SEC-07 / SEC-10: findings register; high and critical cannot be accepted; independent disposition | same test, plus the browser journey |
| AT-27: malformed, oversized and over-quota requests | `tests/integration/ingress.py` (existing, re-run) |
| Clients: Sign out everywhere, privacy request options, staff Security workspace with axe WCAG 2.2 AA | `tests/browser-consumer/consumer.spec.ts` "security lifecycle: …" |

Pending: real MFA and phishing-resistant factors, production KMS or HSM key custody, deployed network policies, CI dependency scanning (WP-29, WP-31, DEC-06/10/12).
