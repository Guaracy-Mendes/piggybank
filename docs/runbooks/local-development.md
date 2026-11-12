# Local development

This environment uses synthetic data and is bound to loopback. Its credentials are fixtures and cannot enable a production profile. The API, leased worker, signed control simulator, Core ledger, customer journeys and browser portals are implemented locally. Current package completion and evidence are recorded in the [execution board](../implementation/WORK_PACKAGES.md).

Use the pinned Rust toolchain in `rust-toolchain.toml` and Node version in `.node-version`. `scripts/cargo.sh` resolves the selected Rust toolchain even when the shell omits the Cargo directory. Install it through rustup if unavailable; `rustup toolchain install 1.97.0 --profile minimal --component rustfmt --component clippy` installs the required components. Use `npm ci` with the selected Node/npm environment after checking its version.

```sh
make dev-up
make migrate
make identity-setup
make dev-seed
make dev
# In another terminal:
curl --fail http://127.0.0.1:4180/health/ready

# Stop make dev with Ctrl-C before running a separate runtime smoke:
make dev-smoke
make dev-down
```

`make dev-down` stops only the `piggybank` Compose project and preserves volumes. PostgreSQL is at `127.0.0.1:15432`, Keycloak at `127.0.0.1:18280`, NATS at `127.0.0.1:14222`, and its local monitoring port is `18222`. API/worker health ports are `4180`/`4181`. No unrelated containers are changed. Override `CONTAINER_RUNTIME` to use a compatible Docker runtime. Inspect occupied ports before overriding these defaults.

`/health/live` reports process responsiveness. `/health/ready` checks PostgreSQL, JetStream account availability and issuer discovery concurrently with a bounded 1.5-second deadline and returns `503` when degraded. These checks do not imply financial/provider/launch readiness. A request never exposes dependency credentials. Runtime health uses the unprivileged platform principal; `make migrate` uses the separate migration principal.

The existing logical C4 views remain the target architecture. The initial implementation co-locates their application components in `bank-api` and `bank-worker`; PostgreSQL provides owned logical schemas, Keycloak provides the local identity boundary, and JetStream is a separate transport process. Domain authority and transaction ownership remain unchanged by co-location. Production deployment and recovery have not been verified.

See the [current implementation models](../implementation/README.md), plus the historical [initial implementation deployment](../implementation/local-deployment.svg) and its [PlantUML source](../implementation/local-deployment.puml). `make test-foundation` verifies durable PostgreSQL restart and a paused-broker failure with bounded degraded readiness and continued liveness.

`make test-integration` creates an isolated disposable PostgreSQL database and verifies the full ordered migration registry, repeated migration, runtime schema privileges, denied DDL/escalation and transaction rollback. Required database tests fail with an explanatory error if their explicit connection configuration is absent; they are not silently ignored. Module principals receive only their own schema's DML, while migrations own schema definitions. SQLx embeds the registry and the platform build script watches the migration directory.

`make test-events` verifies an abandoned job lease and durable projection across actual worker restarts. The worker runs owned outbox relays, the Data operation consumer, registered job handlers and bounded control polling/rescreen/escalation. Unknown job handlers require recovery/review. The provider simulator listens on `4182`, requires its synthetic key for checks/scenario controls and preserves provider-side request identities in its own schema.

`make test-ingress` exercises every implemented route, signed callback duplicate/conflict handling, request bounds and shared quotas across API instances. `make test-integration` also builds the simulator and runs isolated real database/broker/provider tests, including Core journal/hold concurrency, product approvals, party/account lifecycle, control faults and restricted case evidence. Runtime Core cannot directly edit financial records. Synthetic funding is a guarded balanced ledger fixture, never an unrecorded balance override. The public implemented contract is `/openapi.json`; the generated TypeScript client reflects only those actual routes.

`make dev-seed` explicitly bootstraps synthetic workforce roles/queue relationships, a zero-valued settlement account, one TST product/terms pair and a versioned GL chart. Product, terms and chart publication use the real independent maker/checker service paths; initial fixture grants use the local administrative bootstrap. The fixture script cannot run a nonlocal profile. It creates no customer account and provides no unrecorded funding.

`make test-lifecycle` runs the real issuer/BFF/customer HTTP flow through independent provider and worker processes, including server restart and purpose withdrawal. Its PostgreSQL database, object directory and JetStream stream/consumer are isolated and removed afterward. The identity HTTP suite now uses the same disposable database discipline, so synthetic approval-person fixtures cannot interfere with developer customer links. Domain integration tests likewise isolate their event streams.


`make dev` starts both browser portals: customer at `http://127.0.0.1:4300` and workforce at `http://127.0.0.1:4301`. Customer fixture logins are `alice` and `bob`; workforce fixture logins are `reviewer`, `approver` and `security`. Their local-only password is `Synthetic-Local-123!`. The reviewer proposes configuration and case decisions, the approver checks independently, and security proposes/revokes role grants. The approver has separate role-approval authority. Queue membership, canonical identity and current resource relationships are checked separately from token roles. Existing fixture grants remain explicit bootstrap data; rerunning the seed is not an administrative production workflow.

`make web-check web-build` checks and bundles both browser clients. `make test-customer` completes real Chromium customer onboarding, resume, account history, support, freeze, contact verification and issuer logout. `make test-staff` exercises separate authenticated people, signed control review, scoped evidence and account context, independent configuration publication and entitlement approval/revocation. Both browser suites own disposable databases, broker streams, private evidence directories and their child processes. Run HTTP/browser suites sequentially because they share loopback runtime ports. Do not run them beside `make dev`. Rust integration suites use their own databases and namespaces.

The browser clients use generated API types and exact decimal money strings. The BFF holds issuer access/refresh/ID tokens; browser storage contains only nonsecret navigation/idempotency references. Local HTTP session cookies use HttpOnly and SameSite=Lax; production HTTPS/Secure hosting is outside the enabled local profile. Browser traces and automatic authentication screenshots are disabled. Explicit screenshots contain only synthetic banking views and are saved under ignored `.local/`.

Staff evidence access rechecks current queue membership and scoped case authority. Original signed provider decisions are separate references from downloadable private evidence objects. Read-only support account context is limited to the currently assigned reviewer of an open support case; it creates no journal edit authority. Product/pricing and exact terms drafts are immutable, approved by a different canonical person and published through the owning domain's guarded port. New role grants require independent approval; revocation takes effect on subsequent server authorization checks. Later payment/reconciliation/reporting screens are added with their domain packages.


`make test-payments` runs the real browser payment journey. Two distinct synthetic customers open accounts through current controls; the payer funds through a balanced fixture posting. The recipient shares a receiving reference, the payer reviews an exact quote, completes fresh issuer reauthentication and explicitly confirms. The test checks current ownership, malformed/altered confirmation rejection, exact source/destination balances, one original journal reference after retries and fresh login, staff inquiry, accessibility and mobile layout. Other payment rails remain outside this internal path until their work packages are completed.

Co-located services share one bounded PostgreSQL pool for an exact module role/database URL, rather than multiplying a new pool for each application service. Roles and databases remain distinct; closing a pool invalidates its clones and a later lookup can reopen it. The disposable database tests verify both reuse and authority/database separation. Run browser and database acceptance suites sequentially on the default shared PostgreSQL instance; their isolated databases/streams do not provide independent server connection budgets.

## Simulated outgoing payment submission

After `make migrate identity-setup dev-seed`, the reviewer has `payment_operator` and the approver has `payment_approver`. The Rail controls page shows the current effective synthetic policy and submission pause. Pause applies immediately to new work. Resume requires a maker proposal, a distinct canonical approver and application against the same current revision. A policy publication retains an existing emergency pause. All changes use current server-side authority, CSRF/origin protection and durable audit intents.

The customer can choose SIMULATED when saving the synthetic recipient `SIM00000000000000000000000000000001`. Review and real issuer action verification remain required before explicit confirmation. The provider acknowledgment is pending: Core reserves the total, booked balance stays unchanged, and availability decreases by held funds. Signed finality callbacks or original-identity inquiry drive Core settlement or definitive release. Incoming and returned funds enter suspense before current checks and destination restrictions permit allocation. Customer and workforce views expose actual receipts and linked return history. The [payment contract](../../contracts/providers/payments/README.md) describes fixture callbacks. Do not interpret provider acknowledgment, callback acceptance or workflow completion as a booked payment.

`BANK_RAIL_PROVIDER_BASE` defaults to loopback port 4182. `BANK_RAIL_SIGNING_KEY` is independent of the controls signing key; defaults are synthetic fixtures only. See the [provider protocol](../../contracts/providers/payments/README.md) for faults and typed fields. Lost responses, uncertain outcomes, expired quotes and signed NOT_FOUND observations never release a submitted hold. Existing inquiries continue through a submission pause.

Database integration tests use two harness threads on the shared local PostgreSQL server. Individual financial tests still exercise concurrent commands explicitly. Browser and HTTP suites run sequentially; this avoids treating fixture contention as a runtime throughput test. Provider deadlines remain unchanged.

## Finance GL export slice

After migration and seeding, the worker discovers immutable Core batches and schedules unique GL exports. **Finance & GL** in the workforce portal exposes sealed chart mappings, exact totals, authenticated booking receipts, retained transport/recovery history and owned cases. The reviewer has Finance adjuster authority and REVIEW queue membership; the independent approver can inquire using the Finance approver role and APPROVE relationship. Security administration alone does not grant Finance inquiry. See the [Finance execution specification](../implementation/finance-execution.md) and [recovery runbook](finance-recovery.md).

`BANK_FINANCE_PROVIDER_BASE` defaults to `http://127.0.0.1:4182`; `BANK_FINANCE_SIGNING_KEY` is distinct from rail/control keys. The simulator commits one immutable booking per original Core source, then answers exact original inquiry after a timeout or restart. A GL export never posts Core money or creates a second fee.

The [Finance reporting runbook](finance-reporting.md) covers immutable UTC period observations, exact controls, frozen source pages and owned snapshot issues. Independent cash observations use `BANK_LIQUIDITY_SIGNING_KEY`. The simulator has no default cash amount; missing/stale evidence leaves current available liquidity unavailable. The portal separates current cash status from the evidence captured in a saved report. The [revision runbook](finance-revisions.md) adds independent approval and original-maker publication of exact replacement links. The [outgoing reconciliation runbook](finance-reconciliation.md) and [incoming/return runbook](finance-credit-reconciliation.md) cover independent files, native accounting and immutable GL matching. The [grouped reconciliation runbook](finance-group-reconciliation.md) adds closed signed membership across bounded files. Periodic fee/accrual/calendar/correction automation remains in WP-24.

Browser functional fixtures use a separate 5,000-request-per-minute quota (`BANK_BROWSER_RATE_LIMIT`) because multiple independent synthetic people share one loopback IP. The application's default remains 500. The dedicated ingress suite configures and verifies shared quota enforcement independently.

The [outgoing Finance reconciliation runbook](finance-reconciliation.md) adds independent clearing-file publication and immutable instruction/Core/journal/GL matching. Existing bank fees remain already posted; the observation workflow never charges them again.

## Periodic policy previews

The workforce **Periodic policies** workspace supports independent approval/publication of immutable synthetic policies and exact monthly-fee/daily-accrual previews. Follow the [complete policy runbook](periodic-accounting-policies.md) for calendar/cutoff examples and expected amounts. Schema 49 is required for publication clock checks after lock waits. Policies remain PREVIEW_ONLY and no periodic charges or interest are posted. Current product terms and payment fees are unchanged. See [policy evidence](../implementation/evidence/WP-24-periodic-policies.md).

## Controlled financial corrections

Schema 50 adds **Financial corrections** for exact full reversals of original synthetic funding. Follow the [complete correction runbook](finance-corrections.md): capture the original source, obtain a separate current canonical checker, then explicitly execute through the original maker. Core enforces source/account locks, live authority, available funds after holds, restrictions and one linked native receipt. No automatic registration task executes money. The [evidence](../implementation/evidence/WP-24-corrections.md) records validation and remaining scope.

The foundation restart acceptance now waits for fresh customer and workforce issuer login forms after restarting PostgreSQL. OIDC discovery can remain available while the issuer discards stale database connections; dependency readiness alone does not prove a fresh login succeeds. The probe uses the configured customer-web/staff-web clients and no user credentials.
