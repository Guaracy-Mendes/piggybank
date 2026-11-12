# Contract conventions

Only implemented routes are published as executable API contracts. The PRD route table remains the proposed surface until each resource is implemented and checked against the router.

Money is `{ "minorUnits": "2500", "currency": { "code": "TST", "scale": 2 } }` in the synthetic local profile. `minorUnits` is a canonical decimal string, bounded to signed 64-bit storage. A money-moving command requires a positive amount; fees and journal calculations use separately validated nonnegative/signed values. `TST` is an explicit fixture unit, not a production or legal-currency selection. No actual currency is enabled by a client-provided code.

Opaque UUIDs identify operations, accounts, parties, applications, payments and events with distinct Rust types. Updates carry an expected aggregate version. Domain actor context is assembled from verified issuer/subject and server-resolved relationships; a request body cannot assert its own roles or represented party.

Idempotency is scoped by verified actor/client, command type and key; requests use a recursively canonical JSON SHA-256 fingerprint after DTO validation. Material payment approval includes payer, beneficiary, amount/currency, fee, requested date, rail and quote. Any material change invalidates the digest. Cached response replay still checks current authorization.

Events conform to [the envelope schema](../events/envelope.schema.json). Producers own payload schemas; envelope schema version and aggregate version are different fields. UTC occurrence time is separate from business/settlement date. Delivery can repeat or arrive out of order; consumers must enforce receipts and aggregate ordering. Financial action is never inferred from delivery success alone.

The executable application/account/payment transition rules require trusted application evidence. They do not perform persistence or create authority: subsequent domain packages must obtain proof from committed Core/control/provider outcomes before constructing transition evidence.
