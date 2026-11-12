# ADR 0001 — Local Implementation Baseline

**Date:** 2026-10-23. **Status:** Selected planning default for local implementation; production decisions remain open. **Parent:** [Implementation plan](../IMPLEMENTATION_PLAN.md). **Related product decisions:** DEC-05, DEC-07, DEC-08.

## Context

The repository contains a complete proposed requirements/architecture baseline with no application stack. The target C4 views separate logical deployables, but the architecture permits consolidation while preserving authority. A runnable local solution must prove domain integrity and recovery without depending on real-bank or payment-network credentials.

## Decision

Use Rust with Axum/Tokio for API/application execution, SQLx with PostgreSQL for explicit transactional persistence, and a separately runnable background worker. Axum integrates with the Tower middleware ecosystem and Tokio; SQLx provides async SQL and transaction facilities. Their actual compatible versions and supported Rust toolchain will be resolved and locked in WP-01, rather than claiming a lockfile already exists. [Axum documentation](https://docs.rs/axum/latest/axum/), [SQLx documentation](https://docs.rs/sqlx/latest/sqlx/).

Keep domain crates independent of HTTP, broker and provider SDKs through narrow ports. Consolidate runtime instances initially, with module-owned schemas and database roles. Financial transactions remain inside Core. PostgreSQL isolation/locking and bounded retry behavior must be tested for actual concurrent workloads; selecting a database alone does not prove the financial invariant. [PostgreSQL transaction isolation](https://www.postgresql.org/docs/current/transaction-iso.html).

Use React/TypeScript with Vite for the customer and staff browser clients, with a separate Rust BFF/API owning server behavior. Use a JavaScript workspace, one lockfile, generated API contracts, and shared web components. This choice deliberately fits an independently owned Rust API; it is not attributed to a preexisting stack in this repository. React's documentation discusses framework and from-scratch options, and Vite supplies the build/development tooling. [React setup choices](https://react.dev/learn/creating-a-react-app), [Vite guide](https://vite.dev/guide/).

Use React Native/Expo for a dedicated mobile client, sharing API schemas and business vocabulary rather than assuming web components render natively. Implement OIDC authorization code with PKCE and platform-protected credentials. The local environment must report iOS/Android tooling and device limits honestly; an unavailable native runner is not a passed test. [Expo authentication guidance](https://docs.expo.dev/guides/authentication/).

Use a local Keycloak issuer for reproducible customer/workforce identity and client configuration. Keep the OIDC discovery/issuer/audience boundary configurable so the product can integrate a selected production issuer without changing account ownership or merging identities. Native, browser, workforce and partner grants remain distinct. Keycloak authentication is not the bank's entitlement, screening or financial decision authority. [Keycloak OIDC endpoints and flows](https://www.keycloak.org/securing-apps/oidc-layers).

Use NATS JetStream as the local durable event bus with explicitly acknowledged consumers. Domain changes/outbox rows commit atomically in owned PostgreSQL transactions; consumers commit receipt/local effect before acknowledgement. Retain durable business deduplication independently of broker delivery or transport deduplication windows. Do not infer exactly-once financial execution from messaging semantics. [JetStream consumers](https://docs.nats.io/learn/jetstream/pull-consumers).

Use a restricted filesystem object adapter for synthetic development evidence/documents. Object writes use safe opaque paths, integrity digests, immutable versions and recovery of unreferenced objects. A selected production object-store adapter must supply access, encryption, versioning, retention/legal-hold and recovery evidence in WP-31. Filesystem development storage is not evidence of production storage readiness.

Use a separate deterministic provider-simulator process for identity/screening, payment submission/inquiry/settlement, GL acknowledgements and notifications. Persist simulator-side instruction identities across restarts so timeout-after-acceptance tests exercise a real duplicate-execution risk. Never connect a simulator profile to real customer accounts or treat its evidence as provider certification.

Use Podman-compatible Compose for local dependencies and portable OCI application images. Keep provider credentials outside tracked configuration; fixture secrets/accounts are explicitly synthetic. Use domain tests, real PostgreSQL integration tests, browser acceptance tests and separate native device tests. Playwright supplies browser automation; native acceptance remains independently required. [Playwright documentation](https://playwright.dev/docs/intro).

## Consequences

This baseline gives one reproducible development stack and explicit future extraction points. It adds Rust/native tooling setup to an environment that does not currently expose Rust on its command path. It introduces issuer/broker operations, but exercises the real asynchronous and authorization contracts early.

Local physical co-location simplifies startup and does not supply production isolation, high availability or recovery guarantees. Provider/core build-or-buy selection, residency, certification and legal policies remain external decisions. Exact versions, image digests, CI runner capabilities and compatibility are validated during implementation. Revisit this ADR if those facts require a different stack or physical boundary; preserve domain contracts and financial invariants during any change.
