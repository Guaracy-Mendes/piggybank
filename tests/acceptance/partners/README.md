# Partner consent and delegation acceptance (WP-33)

`partners.py` runs against the real local issuer, API, worker and provider, using the shared browser fixture and a
fresh database. It runs a local callback receiver on 127.0.0.1:4390 that verifies HMAC signatures.

```sh
cargo build --workspace
python3 scripts/identity.py      # provisions the partners realm and the synthetic partner-budget and partner-other clients
make test-partners
```

| Result key | What is proven |
| --- | --- |
| `registry_maker_checker` | A pending client's token is refused (401). The proposer cannot approve its own registration; a second person can. |
| `sunset_version_410` | A missing version returns 400. A sunset version returns 410 with `Sunset`. |
| `deprecated_version_headers` | A deprecated version is served with `Deprecation: true` and `Sunset`. |
| `signed_callback_verified` | `consent.authorized` arrives signed with the once-shown secret, and a modified body fails verification. |
| `AT21_revocation_in_flight` | A partner-drafted, customer-approved payment is in flight when the customer revokes. Every new partner call returns 403; the payment settles once (one journal batch) and records `partner-budget` / `PARTNER` as initiator. |
| `quota_and_client_isolation` | A 3/minute client gets 429 on the fourth call. Another client cannot use a foreign consent id. |
| `delegation_bounded_and_revocable` | A delegate reads and pays within the limit. An over-limit amount and account servicing are refused. After revocation access ends, and the earlier intent keeps the delegate as its actor. |
| `suspension_blocks_client` | A suspended client's tokens return 401, and staff see delivered callbacks. |

The database-level rules (immutability, scope and account binding, expiry and revocation in Controls, delegation
history) are covered by `crates/core/tests/cases/partners.rs`. The browser journey is in
`tests/browser-consumer/consumer.spec.ts`.
