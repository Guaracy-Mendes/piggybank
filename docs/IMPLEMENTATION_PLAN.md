# Digital Bank — Implementation Plan

| Field | Baseline |
| --- | --- |
| Plan ID / version | DB-IMPL-001 / 1.0 |
| Date / status | 2026-10-23 / ready for execution; application work packages are planned |
| Product baseline | [PRD v1.0](PRD.md), [architecture](ARCHITECTURE.md), [C4/UML models](diagrams/README.md) |
| Implementer | Codex, working in this repository; product/control decisions remain with their accountable roles |
| Execution source of truth | [Machine-readable backlog](implementation/backlog.json) |
| Detailed task instructions | [Work packages and requirement allocation](implementation/WORK_PACKAGES.md) |
| Technical baseline | [Local implementation decision](decisions/0001-local-implementation-baseline.md) |

## 1. Outcome and current starting point

Implement the full capability landscape already specified, in its R0/R1/R2/R3 order. The first executable outcome is a local, persistent banking environment using synthetic data and deterministic external-provider simulators. It must support a customer moving from application to active account, controlled money movement, statements and staff exception handling. The same domain contracts will then support real providers, a limited launch, additional channels/rails and optimization.

At this baseline, the repository contains requirements, diagrams, traceability and documentation tooling. It has no application, schema, API implementation, verified provider integration or deployment. No work package below is complete. The existing 77 functional requirements and 30 acceptance scenarios remain the acceptance baseline; no requirement is silently removed to simplify the first build.

The environment check on 2026-10-23 found Node/npm, Podman/Docker, Java and Graphviz on the command path; Rust/Cargo/rustup and `psql` were not found there. No `.git` directory is present. WP-01 must establish a reproducible toolchain and repository state rather than assuming another project's setup applies. It must inspect existing files before making changes and record whether a Git repository will be initialized or supplied by the surrounding environment.

This plan selects reversible engineering defaults for local implementation. It does not select a bank license, launch market, production provider, actual payment rail, cloud subscription, currency policy or retention obligation. Those decisions are managed explicitly in [section 9](#9-external-decisions-and-provider-gates).

## 2. Execution rules for Codex

1. Read this plan, the current backlog, the relevant PRD rows, and their C4/UML models before implementing a package. Inspect workspace changes and retain unrelated work.
2. Use `make plan-next` to find the earliest package whose implementation dependencies are complete. Start with WP-01. Follow the package's implementation steps, planned artifacts and completion conditions.
3. Implement a working vertical slice with persistence and observable behavior. Add modules and infrastructure when the slice needs them; an empty crate, sample screen or mocked success path does not complete a capability.
4. Keep domain authority explicit: only Core commits balances, holds and journals; Payments owns instruction/provider state; Controls owns distinct decisions and cases; channels and analytics never become financial authorities.
5. Build and run the checks appropriate to the change. Financial, authorization and durable workflow changes require real PostgreSQL integration and failure/concurrency cases. Do not substitute in-memory tests for database transaction evidence.
6. Update the backlog with status, implemented artifacts, actual commands/results and remaining limitations. Keep requirement, route, event, state and diagram changes synchronized. Mark a package complete only after its stated exit conditions have evidence.
7. Continue to the next ready package without routine confirmation. If provider credentials, jurisdictional policy or another external fact is missing, record that exact dependency and continue independent local work. Never invent settlement evidence, customer clearance or stakeholder decisions.
8. Keep local simulation, provider sandbox, certified integration and production evidence separate. A successful local demo is a local result. A built pipeline definition is not proof of a successful remote pipeline.

The user has authorized autonomous continuation within the requested scope. Work packages are execution units, not recurring approval checkpoints. Product launch authority and external factual inputs remain distinct from routine implementation choices.

## 3. Implementation architecture

### 3.1 First deployment shape

Start with a **modular monolith plus a separately runnable worker**, preserving the logical boundaries in the C4 model. This keeps early delivery manageable while providing explicit module APIs and owned persistence. Extract a service only when measured deployment, isolation or scaling needs justify it.

| Runtime / component | Initial responsibility |
| --- | --- |
| `bank-api` | Axum HTTP boundary, channel BFF/session handling, domain command/query routing, and authenticated callback ingress |
| `bank-worker` | Durable workflows/timers, outbox publication, event consumers, payment submission/inquiry, reconciliation and record generation |
| PostgreSQL | Owned schemas for Core, Journey, Payments, Controls, Finance, Documents, Identity/session integration, Projections and platform workflow metadata |
| Keycloak | Local OIDC issuer with separated customer/workforce/client trust contexts; production issuer remains configurable |
| NATS JetStream | Durable event transport with explicit acknowledgements; local outboxes/inboxes enforce business deduplication |
| Customer web | React/TypeScript/Vite customer journeys; financial rules stay on the server |
| Staff web | Separate client/session/audience for reviews, approvals, reconciliation, product configuration and support |
| Mobile | React Native/Expo application sharing generated API contracts; native authentication, secure credential storage and device lifecycle handling |
| Provider simulator | Separate synthetic identity/screening/payment/GL/notification endpoints with deterministic faults and durable provider instruction identity |
| Object storage adapter | Restricted local filesystem implementation for synthetic objects; deployable object-store adapter and retention behavior validated before live use |

Separate schemas do not authorize arbitrary cross-module writes. Give migrations a dedicated principal; use module-specific database roles/pools and explicit service interfaces. A local physical database is acceptable; cross-domain financial transactions remain prohibited. Core's financial unit of work is its own transaction boundary. No worker may hold a database transaction open across a provider HTTP call.

The consolidated runtime is a physical implementation choice, not a change to the image's capabilities. Before WP-01 closes, add an implementation deployment view and ADR linkage to distinguish the existing target C4 containers from their initial co-location. Preserve target logical views and annotate the implementation mapping rather than relabeling all domains as already deployed services.

### 3.2 Proposed repository layout

Paths below are **planned artifacts**, not files already implemented. Create each as its work package needs it.

```text
apps/
  bank-api/                 # Rust HTTP executable and channel BFF
  bank-worker/              # Rust worker executable
  provider-simulator/       # Synthetic provider endpoints and fault scripts
  customer-web/             # React / TypeScript / Vite
  staff-web/                # Separate workforce browser client
  mobile/                   # React Native / Expo
crates/
  platform/                 # IDs, money, clocks, tracing, errors, operations, outbox/inbox
  identity/                 # Issuer validation, sessions, revocation, grants; no homegrown passwords
  core/                     # Parties, products, accounts, restrictions, holds, immutable journal
  controls/                 # Entitlements, check decisions, cases, audit, retrospective monitoring
  journeys/                 # Acquisition, onboarding, opening and servicing orchestration
  payments/                 # Intent, approvals, execution, evidence, recovery, returns
  finance/                  # Reconciliation, GL mapping, reporting and liquidity
  documents/                # Terms, acceptance, statements, retrieval and retention
  data/                     # Query projections, alerts, mastered links and governed ingestion
  adapters/                 # Explicit provider/object-store/transport implementations
packages/
  api-client/               # Generated TypeScript contracts/client
  ui/                       # Shared web components and accessibility foundations
contracts/
  openapi/                  # Reviewed API specification synchronized with actual router
  events/                   # Versioned envelope/payload schemas and compatibility fixtures
  providers/                # Adapter contracts and rail supplements
migrations/                 # One ordered registry; each migration declares its owning schema
infra/
  compose/                  # Local environment, persistence, health checks, synthetic seeds
  containers/               # Runtime images and ingress configuration
  identity/                 # Local realm/client configuration without production secrets
tests/
  integration/              # Real PostgreSQL and local issuer/broker tests
  acceptance/               # Executable AT scenario entry points and fixtures
  browser/                  # Customer/staff Playwright journeys
  mobile/                   # Device/emulator journeys and lifecycle checks
  resilience/               # Crash, replay, outage, restore and concurrency harnesses
  load/                     # Reproducible workload definitions
docs/
  implementation/           # This execution backlog and evidence index
  decisions/                # Versioned implementation decisions
  runbooks/                 # Operational procedures added alongside working features
```

Versions and image digests must be resolved and pinned in WP-01 using a compatible supported toolchain. Commit `Cargo.lock`, the JavaScript lockfile, a Rust toolchain file, Node version policy and an image/version manifest. Avoid floating container tags and unrecorded global tools. The [technical decision](decisions/0001-local-implementation-baseline.md) gives the rationale and primary documentation references.

## 4. Ordered delivery stages

The backlog contains 41 packages. Dependencies in its JSON are authoritative; table ranges below explain the intended sequence, not permission to skip prerequisites.

| Stage | Packages | Demonstrable outcome | Exit boundary |
| --- | --- | --- | --- |
| S0 — Reproducible foundation | WP-01–WP-09 | Locked build, database ownership/migrations, issuer/session integration, authorization, audit, durable operations and events | Restart-safe durable operations; denied unauthorized calls; outbox replay demonstrated |
| S1 — Core and customer lifecycle | WP-10–WP-19 | Authoritative journal/holds, products, parties/accounts, distinct controls, case handling, onboarding/terms and customer/staff inquiry | Synthetic applicant reaches one active account; reviews and failed gates are operable |
| S2 — Money, records and all channels required for R1 | WP-20–WP-27 | Internal transfers, simulated external/inbound/returns, unknown-outcome recovery, finance, statements, alerts and native mobile | End-to-end synthetic customer/staff/mobile journeys with reconciled records |
| S3 — R1 engineering qualification | WP-28–WP-30 | Privacy/security lifecycle, container/release tooling, load/fault/restore evidence | Local qualification report; provider and production evidence still separate |
| S4 — Provider integration and controlled R1 | WP-31–WP-32 | Selected real-provider adapters, certification, deployment-specific validation and launch evidence | External decisions and accountable launch gates satisfied; otherwise explicitly pending |
| S5 — R2 capability expansion | WP-33–WP-38 | Partner consent/delegation, assisted channels/ATM, additional rails, enterprise analytics and consented journeys | Each enabled channel/rail has its own evidence; production enablement depends on R1 gate |
| S6 — R3 optimization | WP-39–WP-41 | Advanced routing/approvals, liquidity forecasting and measured outcome/scale improvements | Evidence of benefit with unchanged financial/control invariants |

WP-16 (terms) precedes integrated onboarding in WP-15 despite its numeric identifier. Use the dependency graph rather than numerical sorting alone. WP-01–WP-30 can reach local qualification without real bank/provider credentials. Later packages can build ports and simulators independently when their engineering dependencies are complete, while activation remains gated.

### 4.1 First working slice

Do not begin with all screens or payment rails. Complete the foundation and financial authority first, then deliver a small coherent slice:

1. Start the environment with persistent PostgreSQL, local issuer, broker, API, worker and deterministic provider simulator.
2. Sign in as a customer through the intended channel trust context; deny an attempted workforce action.
3. Discover a versioned synthetic deposit offer, submit/resume an application, obtain distinct identity/screening/risk decisions, and handle a manual-review variant through the staff workbench.
4. Accept the exact terms version and create one account despite retries or a lost response.
5. Show authoritative/projection freshness and zero opening balance; seed funding only through an explicit synthetic inbound movement, never by setting a balance field.
6. Execute a controlled internal transfer, then a simulated external payment whose response is lost; recover the same provider instruction, reconcile the posting, and issue an authorized statement.

This slice spans multiple packages and is an integration milestone. A successful happy path alone does not close all packages involved.

## 5. Persistence and financial implementation

### 5.1 Minimum invariants

Implement these before exposing spendable accounts or payment execution:

- Exact money uses signed checked integer minor units, validated currency/scale and explicit range bounds. Serialize large integers in a client-safe representation; never let JavaScript precision alter money. Amounts accepted by the API must be positive and bounded for their command type.
- Journal batches balance by currency, are immutable after posting, and have a unique `(operation_id, posting_kind)` business reference. Financial corrections add a linked balanced batch. Enforce posting through the authorized transaction path and database constraints, including deferred balance validation at commit where needed.
- Core owns posted balances, active holds, restrictions and spend decisions. Check availability and restrictions in the same transaction as reserve/post/consume/release. Lock multiple accounts in stable order; retry serialization/deadlock failures with bounded policy and the same business key.
- An internal transfer commits both customer liability sides in one Core transaction. Opening an account does not create funded value. Funding fixtures use balanced inbound/suspense operations with traceable references.
- Persist the payment instruction and submission work before calling a provider. A lost provider reply retains the same provider instruction identity. Unknown submission/settlement keeps the safe financial restriction and enters inquiry/reconciliation.
- Mark a payment settled only after certified finality evidence and the required unique Core booking. Provider evidence and Core commits are separate recoverable steps.
- Record received provider evidence durably before acknowledging it. Verify authenticity, replay identity, amount/currency/ref and permitted transition; quarantine contradictions instead of overwriting history.
- Safe cancellation or definitive non-execution releases a hold once. A potentially submitted instruction cannot lose its hold through an automatic timer. Returns and partial/multiple corrections use linked movements with aggregate amount limits.

These rules implement BR-01 through BR-15 alongside the existing state machines. Tests must exercise both the application path and unauthorized/direct persistence attempts that would bypass invariants.

### 5.2 Migration and record strategy

Use a single ordered migration registry to avoid collisions across domain crates, with module ownership documented per migration. Separate the database's IAM provider tables from bank-owned subject links/session integration. Use opaque IDs and `(issuer, subject)` identity links; never merge on email. Store UTC instants separately from business date, settlement date and rail calendar.

Core records include accounts, relationship versions, products/pricing, restrictions, holds, journal batches/entries and Core outbox. Payments records include instruction/version, scoped idempotency fingerprint, provider references, evidence receipts, submission/inquiry work and linked return movements. Controls records keep each check's policy/evidence/expiry, review cases and audit. Journey records keep application/workflow versions and correlation. Finance/Document/Projection schemas own their respective derived and operational records.

Every accepted financial operation must be recoverable by durable business key independently of a short-lived API response cache. Define non-destructive migration, backward-compatible deployment and restoration verification before live data. Retention, legal holds and disposal apply to objects, metadata, exports, logs and backups under explicit policy.

## 6. Identity, APIs and client delivery

Use a configured OIDC provider rather than implementing password storage. Keep customer, workforce, service and partner audiences/trust contexts separate. Browser channels use a BFF with opaque, protected session cookies, CSRF protection and server-held tokens. Native clients use authorization code with PKCE and OS-protected credential storage; do not treat embedded web credentials as native authentication. A changed amount/beneficiary invalidates an action-bound approval even if the OIDC session remains authenticated.

Build the PRD's proposed `/api/v1` resources incrementally. Publish OpenAPI from a reviewed contract and test it against the actual router, response codes and authorization. Generate TypeScript DTOs/client and check drift in CI. Use separate public/owned-channel/staff/provider ingress policies with limits, safe errors and request correlation. Same-key response replay rechecks identity and resource permission.

Customer web, staff web and mobile must share operation/account identities and behavior. Reuse API contracts and appropriate UI foundations, while keeping workforce sessions distinct. Mobile needs its own build, redirect handling, secure storage, app suspension/resume, lost-network recovery and device evidence; a responsive browser preview does not complete FR-CH-01's planned native delivery. Do not queue offline financial commands for later automatic execution.

## 7. Verification and evidence

### 7.1 Required verification by layer

| Layer | Minimum proof |
| --- | --- |
| Pure domain | Money bounds/scales, allowed/forbidden transitions, fee rounding, balanced journal proposals, material-detail digest and correction limits |
| PostgreSQL integration | Atomic debit/credit, concurrent reserves, transaction rollback, unique business references, append-only restrictions, schema grants, migrations and restore |
| Identity/authorization | Real local issuer validation, issuer/audience rejection, customer/workforce separation, resource ownership, revocation, maker/checker and recovery abuse |
| Workflow/event | Crash after state commit/before publish, duplicate/out-of-order event handling, consumer receipt/effect atomicity, job leases/fencing, DLQ recovery and replay |
| Provider contract | Signed/authenticated evidence, durable receipt, timeout-after-acceptance, duplicate callbacks, pending/final/return distinctions, inquiry and conflicting evidence |
| Client | Customer and staff browser journeys, keyboard/screen-reader checks, native device lifecycle, consistent pending states and document authorization |
| Operations | Paused submissions with callback/reconciliation continuity, aging/escalation, redacted telemetry, credential rotation, controlled adjustment and business-day completeness |
| Resilience / capacity | NFR workload and burst, slow dependencies, fenced failover, backup restore, acknowledged-gap inventory and reconciliation before reopening |

Map actual executable tests back to AT-01 through AT-30 in test names or an acceptance manifest. The backlog's scenario references define intended coverage; an AT ID remains planned until all applicable branches have evidence. Use local PostgreSQL for its transaction semantics; mocks are for provider responses, not substitute databases.

### 7.2 Planned development commands

Only documentation/plan commands currently exist. WP-01 and later packages must implement these targets before the plan can claim they ran:

```sh
make dev-up             # Start isolated, persistent local dependencies
make migrate            # Apply the ordered schema registry
make dev                # API, worker and required clients
make app-check          # Locked Rust checks, frontend types/lint and contract drift
make test-unit          # Domain invariants and authorization logic
make test-integration   # Disposable real PostgreSQL/issuer/broker; required tests cannot silently skip
make test-acceptance    # Current AT manifest against the synthetic complete environment
make test-browser       # Customer and staff Playwright journeys
make test-mobile        # Native/emulator cases; unsupported platforms reported explicitly
make test-resilience    # Crash, replay, concurrency and recovery cases
make test-load          # Approved reproducible workload/latency evidence
```

Retain existing `make check`, `make diagrams` and `make docs` semantics. Add application aggregates explicitly rather than making existing documentation commands silently require a database or phone SDK. Locked commands such as `cargo test --workspace --locked` and `npm ci` apply after their manifests/lockfiles exist. If a PostgreSQL suite is ignored in a default test run, invoke it explicitly or redesign the target so required integration tests cannot be reported as passed when skipped.

### 7.3 Completion evidence

Each work package stores evidence entries in the backlog: environment, command/action, actual result, artifact reference and timestamp. Add remote pipeline URL/provider certification reference when available. Sensitive evidence stays in an approved restricted store; the repository carries references and sanitized summaries only.

Use `planned`, `in_progress`, `blocked` or `done`. `done` requires completion notes and evidence; downstream implementation dependencies must also be done. `blocked` requires a specific reason and missing input. Do not mark a local package done merely because a corresponding diagram renders. Do not mark a provider/launch package done using simulator results.

## 8. Release gates

| Gate | Required evidence | Packages |
| --- | --- | --- |
| Local foundation | Locked clean build, owned migrations, authenticated bounded API, audit and restart-safe workflow/event contracts | WP-01–WP-09 |
| Local financial slice | Exact/invariant-safe Core, current controls, idempotent opening and observable customer/staff lifecycle | WP-10–WP-19 |
| Local R1 qualification | Money lifecycle, inbound/returns, finance/records, web/native clients, security/operations and measured applicable NFR checks | WP-20–WP-30 |
| Provider-qualified R1 | Chosen providers/rail, sandbox and certification evidence, finality/calendar rules, production storage/GL/notification/identity integration and deployment checks | WP-31 |
| Controlled R1 launch | Required decisions, accountable owners, staffed operations, real terms/policies, provider evidence, recovery and remaining PRD P0 R1 acceptance | WP-32 |
| R2 enablement | Qualified R1 plus per-partner/channel/rail/analytics control evidence and required external decisions | WP-33–WP-38 |
| R3 enablement | Production baselines, approved optimization policy and evidence that financial/control invariants still hold | WP-39–WP-41 |

The original R0 exit includes market/provider decisions. “Local foundation complete” does not close that full business gate. Similarly, local R1 qualification is an engineering milestone, not a claim of limited-launch readiness. No destructive database reset is a routine production rollback; use forward-compatible migrations, fenced recovery and explicit financial reconciliation.

## 9. External decisions and provider gates

| Decision | Local work can proceed with | Required before activation / final qualification |
| --- | --- | --- |
| DEC-01 | Synthetic adult individual customers in a noncommercial demo environment | Named legal entity, market, sponsor/license model and permitted segment |
| DEC-02 | Documented money-movement interpretation and consolidated journey adapters | Product/architecture confirmation of source ambiguity |
| DEC-03 | Versioned synthetic single-currency/no-overdraft product; fixture chart and exact fees | Approved real terms/pricing, currency/scale, tax/accrual and chart/mapping policy |
| DEC-04 | `SIMULATED` rail identity, deterministic provider API, calendars and fault scripts | Selected R1 rail/provider, settlement accounts, finality, returns, limits and certification |
| DEC-05 | The reversible local stack and physical consolidation in the linked ADR | Production build/buy, issuer/core/provider boundaries, storage, cloud, residency and deployment architecture |
| DEC-06 | Synthetic required-field/control/retention policies explicitly labeled as fixtures | Applicable legal/control/reporting inventory, retention, holds, disclosures and dispute rules |
| DEC-07 | Reproducible local benchmark and fault harness using proposed PRD targets | Ratified workload, recovery/availability budget, regional financial RPO risk and production measurements |
| DEC-08 | Separate transaction command façade and query projection; Core remains operational authority | Confirmed naming/ownership and master-data operating policy |
| DEC-09 | Assisted-channel/ATM/partner contract harnesses with recorded actor/customer context | Actual supported operations, scheme/network contracts, delegation/consent and certification |
| DEC-10 | Explicitly documented local browser/device/language matrix | Supported production devices, translations, service coverage, accessibility and communication channels |
| DEC-11 | Versioned fixture retry/expiry/replay rules preserving unknown-payment restrictions | Approved business retention/replay/hold-aging and escalation rules |
| DEC-12 | Codex executes technical packages and records measured effort | Named business/control owners, pilot limits, cost envelope, staffing and launch authority |

Selecting local engineering defaults does not close all of DEC-05. Keep unsupported external adapters disabled outside the simulator profile. Production profiles fail startup when mandatory provider/policy configuration is missing or a synthetic adapter is selected.

## 10. Planning and estimating work

Work-package size is relative complexity: S, M, L or XL. It is not elapsed time, a price or an SLA. Split an XL package into reviewable implementation increments if needed while preserving its exit criteria. Re-estimate after the first funded synthetic account/payment slice, using actual throughput and provider lead times. Do not manufacture a launch calendar from the architecture image.

The principal engineering sequence is foundation → financial authority → controlled onboarding/accounts → payments/recovery → finance/records → channel/operations qualification. UI integration and test harnesses can progress as their dependencies become available. External certification, legal policy, mobile platform tooling and staff readiness are separate lead-time risks.

## 11. Resume and reporting protocol

At the start of an implementation session, run the existing checks needed to establish the baseline, inspect the backlog and run `make plan-next`. Resume any in-progress package before starting unrelated work. If the workspace or requirements changed, update the package's impact/dependencies before editing code.

At each completed package, record behavior delivered, important changed artifacts, tests actually run and outstanding external inputs. Update requirement/API/event/diagram traceability only to reflect implemented behavior. Keep planned coverage alongside evidence rather than replacing it with an unsupported “complete” label. Continue to the next eligible package unless the active user instruction changes scope.

Current next action: **WP-01 — Establish repository, toolchain and local runtime baseline**. The [detailed work-package board](implementation/WORK_PACKAGES.md) contains its exact steps and exit conditions.
