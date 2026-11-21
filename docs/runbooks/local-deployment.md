# Local deployment runbook

This runbook takes a Linux workstation from nothing to a running Piggybank stack: the API, the worker, the provider simulator, the customer and workforce portals, and their PostgreSQL, Keycloak and NATS dependencies. It also covers checks, daily operation, upgrades, tests, troubleshooting and teardown.

**This is not a production deployment.** Everything runs on `127.0.0.1` with synthetic data, synthetic TST money and fixture credentials published in this repository. The binaries refuse any profile other than `local`. Real providers, production hosting and launch qualification are separate work, tracked in [COMPLETION_STATUS.md](../implementation/COMPLETION_STATUS.md). For cloud packages see [docs/deployment](../deployment/README.md).

| If you want to… | Go to |
| --- | --- |
| Deploy for the first time | [§2 Prerequisites](#2-prerequisites), then [§3 First-time deployment](#3-first-time-deployment) |
| Start or stop an existing deployment | [§6 Daily operation](#6-daily-operation) |
| Update after `git pull` | [§6.4 Upgrade to new source](#64-upgrade-to-new-source) |
| Fix a problem | [§10 Troubleshooting](#10-troubleshooting) |
| Wipe everything and start again | [§11.2 Full reset](#112-full-reset-destructive) |

## 1. What gets deployed

```
 Browser ──► 127.0.0.1:4300  customer-web (Vite) ──┐  proxies /api, /auth/customer
 Browser ──► 127.0.0.1:4301  staff-web    (Vite) ──┤  proxies /api/v1/staff, /auth/workforce
                                                   ▼
                              127.0.0.1:4180  bank-api ──────────┐
                              127.0.0.1:4181  bank-worker ───────┤
                              127.0.0.1:4182  provider-simulator ┤
                                                                 ▼
   Containers (Compose project "piggybank"):
     127.0.0.1:15432  PostgreSQL 18.4   (databases: piggybank, keycloak)
     127.0.0.1:18280  Keycloak 26.5.0   (realms: customers, workforce, partners)
     127.0.0.1:14222  NATS 2.12 JetStream, monitoring on 127.0.0.1:18222
```

| Component | Port | Runs as | Notes |
| --- | --- | --- | --- |
| PostgreSQL | 15432 | container `piggybank-postgres-1`, volume `postgres-data` | Admin `dev_admin` / `synthetic-admin-only`. Migrator `bank_migrator` / `synthetic-local-only`. One login role per module: `bank_<module>` / `synthetic-<module>-only`. |
| Keycloak | 18280 | container `piggybank-keycloak-1` | Admin console at `http://127.0.0.1:18280/admin/`, login `local-admin` / `synthetic-keycloak-admin-only`. Stores its data in the `keycloak` database. |
| NATS JetStream | 14222, 18222 | container `piggybank-nats-1`, volume `nats-data` | Event stream `BANK_EVENTS`, subjects `bank.events.*`. |
| bank-api | 4180 | host process `target/debug/bank-api` | Public API, BFF sessions, OpenAPI at `/openapi.json`, health at `/health/live` and `/health/ready`. |
| bank-worker | 4181 | host process `target/debug/bank-worker` | Outbox relays, jobs, consumer and savings ticks, control polling. Health on 4181. |
| provider-simulator | 4182 | host process `target/debug/provider-simulator` | Synthetic controls, rail, GL and liquidity providers. Refuses to bind anything other than loopback. |
| customer-web | 4300 | Vite dev server | Customer portal. Sign in as `alice` or `bob`. |
| staff-web | 4301 | Vite dev server | Workforce portal. Sign in as `reviewer`, `approver` or `security`. |

The fixture password for all five portal users is `Synthetic-Local-123!`. Every credential above is a published synthetic fixture; never reuse one anywhere else.

Two rules apply throughout this runbook:

- **Always use `127.0.0.1`, never `localhost`.** Issuer redirects, cookie scope and the trust-boundary proxies are all configured for `127.0.0.1`. With `localhost`, sign-in loops or fails.
- **One owner per port set.** `make dev`, the persistent preview and every HTTP or browser test suite all bind ports 4180 to 4301. Run only one of them at a time.

## 2. Prerequisites

### 2.1 Host

| Item | Requirement |
| --- | --- |
| OS | Linux x86_64. The reference workstation runs Ubuntu. Scripts assume GNU userland and `/proc`. macOS and Windows are untested. |
| CPU and memory | 4+ cores and 16 GB RAM is a workable minimum. The reference machine has 8 cores and 62 GB, and full Rust builds are CPU-heavy. |
| Disk | At least 40 GB free. `target/` grows with every test build; 77 GB was observed on the reference workstation. `scripts/cargo.sh clean` reclaims it. |
| Free ports | 4180, 4181, 4182, 4300, 4301, 14222, 15432, 18222, 18280, all on loopback. |
| Network | Needed for the first image pull, Rust crates, npm packages, Playwright Chromium and the PlantUML JAR. After that, everything runs offline. |

Check the ports before you start. Empty output means they are free:

```sh
ss -ltn | grep -E '127\.0\.0\.1:(4180|4181|4182|4300|4301|14222|15432|18222|18280)\b'
```

### 2.2 Tools

| Tool | Version | Why | Check |
| --- | --- | --- | --- |
| Podman | 5.x (5.7.0 verified) | Runs the three containers. The seed script, several tests and the backup runbook call `podman` directly. | `podman --version` |
| Compose provider for Podman | Docker Compose v2 plugin (2.40.3 verified) or `podman-compose` | `podman compose` hands off to it | `podman compose version` |
| Python | 3.11 or newer (3.14.4 verified) | Orchestration scripts; `scripts/cargo.sh` needs `tomllib` | `python3 --version` |
| Rust | 1.97.0 exactly, with `rustfmt` and `clippy` | Pinned in `rust-toolchain.toml` | `scripts/cargo.sh --version` |
| Node.js | 24.15.0 exactly | Pinned in `.node-version`; `scripts/npm.sh` refuses other versions | `scripts/npm.sh exec -- node --version` |
| npm | 11.12.1 | Pinned in `infra/versions.json` | `scripts/npm.sh --version` |
| make, git, curl, a C compiler | any recent | Builds; `ring` compiles C | `make --version; cc --version` |
| Java 17+ and Graphviz | optional | Only for `make docs`, `make diagrams` and `make check` | `java -version; dot -V` |
| Playwright Chromium | from `@playwright/test` 1.64.1 | Only for browser test suites | see [§3.1](#31-get-the-source-and-install-dependencies) |
| Android SDK, emulator, JDK 17 | optional | Only for the native app; see the [mobile runbook](mobile-development.md) | |

**Docker instead of Podman.** `make dev-up`, `make dev-down` and `scripts/database.py` honor `CONTAINER_RUNTIME=docker`. `scripts/seed.py`, the browser suites and the backup commands call `podman` directly, so a Docker-only host cannot complete [§3.5](#35-seed-the-synthetic-fixtures) without changes. Podman is the supported runtime.

### 2.3 Install the tools (Ubuntu 24.04)

Skip anything you already have at the right version.

```sh
# Base tools, container runtime and the Compose v2 plugin that podman compose uses
sudo apt-get update
sudo apt-get install -y build-essential git curl make python3 podman docker-compose-v2

# Optional: diagram and documentation checks
sudo apt-get install -y default-jre graphviz

# Rust through rustup, then the pinned toolchain
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain none
. "$HOME/.cargo/env"
rustup toolchain install 1.97.0 --profile minimal --component rustfmt --component clippy

# Node through nvm, then the pinned version
curl -o- https://raw.githubusercontent.com/nvm-sh/nvm/v0.40.3/install.sh | bash
. "$HOME/.nvm/nvm.sh"
nvm install 24.15.0
npm --version   # if this is not 11.12.1: npm install -g npm@11.12.1
```

`scripts/cargo.sh` finds the pinned Rust toolchain even when `~/.cargo/bin` is missing from `PATH`. `scripts/npm.sh` looks in `~/.nvm/versions/node/v24.15.0`. Always run Cargo and npm through these wrappers so the pinned versions are used.

Rootless Podman needs subordinate UID and GID ranges for your user, which Ubuntu sets up by default. Check them with `grep "$(id -un)" /etc/subuid /etc/subgid`.

## 3. First-time deployment

Run every command from the repository root. Each step lists what it does, the output to expect, and how to check it.

### 3.1 Get the source and install dependencies

```sh
git clone <repository-url> piggybank && cd piggybank
scripts/npm.sh ci                                   # web workspaces: web-core, api-client, customer-web, staff-web
scripts/cargo.sh build --workspace --locked         # all Rust binaries into target/debug/
scripts/npm.sh exec -- playwright install chromium  # only needed for browser tests
```

Expected: `npm ci` finishes without errors, and Cargo finishes with `Finished dev profile`. The first Rust build takes several minutes. Check the binaries:

```sh
ls target/debug/ | grep -E '^(bank-api|bank-worker|provider-simulator|bank-migrate|bank-seed|bank-fund)$'
```

All six names should be listed. The mobile app has its own lockfile and is not installed here; see the [mobile runbook](mobile-development.md).

### 3.2 Start the infrastructure containers

```sh
make dev-up
```

This runs `podman compose -p piggybank -f infra/compose/compose.yaml up -d`. It then waits up to 120 seconds each for Keycloak's customers-realm discovery document and the NATS health endpoint.

- **On first start**, PostgreSQL runs `infra/compose/bootstrap.sql`. That creates `bank_migrator`, makes it owner of `piggybank`, creates the `keycloak` role and database, and revokes `CREATE` on `public`.
- **On first start**, Keycloak imports `infra/identity/customers-realm.json` and `workforce-realm.json`, plus the `piggybank` login theme. The import only runs when a realm is missing, so later restarts keep existing users.
- **Image pinning.** All images are pinned by digest; see `infra/versions.json`.

Expected last line: `Persistent local PostgreSQL, Keycloak and JetStream are ready.`

Check:

```sh
python3 scripts/dev.py status        # three containers, all "Up"
podman exec piggybank-postgres-1 pg_isready -U dev_admin -d piggybank
curl -fsS http://127.0.0.1:18222/healthz && echo
curl -fsS http://127.0.0.1:18280/realms/customers/.well-known/openid-configuration | head -c 120; echo
```

On a fresh volume, Keycloak can take 30 to 90 seconds before discovery responds; `make dev-up` waits for it.

### 3.3 Create runtime roles and apply migrations

```sh
make migrate
```

This does two things:

1. **`scripts/database.py`** creates the ten module login roles if they are missing: `bank_platform`, `bank_identity`, `bank_core`, `bank_controls`, `bank_journeys`, `bank_payments`, `bank_finance`, `bank_documents`, `bank_data` and `bank_simulator`. They get `NOINHERIT` and no schema ownership. Output: `Synthetic module runtime roles provisioned; no schema ownership granted.`
2. **`bank-migrate`** connects as `bank_migrator` (override with `MIGRATION_DATABASE_URL`) and applies every pending file in `migrations/` in order. Output: `Owned schema migrations applied successfully.`

Migrations are forward-only and expand-only. Applied files are locked by checksum in `contracts/migrations.lock.json`. Re-running `make migrate` is safe; it applies only what is new.

Check: the version and count should both equal the number of files in `migrations/`, which is 80 at schema 80.

```sh
ls migrations/[0-9]*.sql | wc -l
podman exec piggybank-postgres-1 psql -U dev_admin -d piggybank -At \
  -c "SELECT max(version), count(*), bool_and(success) FROM _sqlx_migrations"
podman exec piggybank-postgres-1 psql -U dev_admin -d piggybank -At \
  -c "SELECT rolname FROM pg_roles WHERE rolname LIKE 'bank_%' ORDER BY 1"   # 11 roles including bank_migrator
python3 scripts/migrations.py check
```

### 3.4 Configure the identity provider

```sh
make identity-setup
```

`scripts/identity.py` signs in to the Keycloak admin API as `local-admin`. It is idempotent: it creates or updates objects and never deletes anything. It sets up the following.

**Realm settings**
- `customers` and `workforce`: 30-second access tokens, refresh-token rotation with no reuse, and user and admin events enabled.
- Customer self-registration is on (`registrationAllowed`). Set `BANK_CUSTOMER_REGISTRATION=disabled` before running to turn it off. The registration terms step is required, and new users get the `customer` role by default.

**Clients**
- `customer-web`: confidential client, redirect `http://127.0.0.1:4180/auth/customer/callback`.
- `bank-mobile`: public PKCE client, redirect `piggybank://auth/callback`.
- `staff-web`: confidential client.
- `bank-worker`: service account.
- `bank-identity-admin`: service client with only `manage-users` and `view-users` in `customers`.

Password grants are disabled for all application clients.

**Fixture users**
- `alice` and `bob` in `customers`.
- `reviewer`, `approver` and `security` in `workforce`.

**Partners realm**
- A separate `partners` realm with the `partner-budget` and `partner-other` client-credential clients.

Expected output: `Synthetic issuer clients, roles and users configured; password grants disabled for application clients.`

Check. Expected output: `['alice', 'bob']`.

```sh
python3 -c "import sys; sys.path.insert(0,'scripts'); from identity import admin_token, call; \
print(sorted(u['username'] for u in call(admin_token(),'customers/users?max=100')))"
```

> **Never enable "Email as username"** (`registrationEmailAsUsername`) on the customers realm. Keycloak then renames existing users to their email addresses, and the `alice` and `bob` fixtures stop working. A test that registers a user deletes it afterwards, so the realm should list only `alice` and `bob`.

### 3.5 Seed the synthetic fixtures

```sh
make dev-seed
```

This builds `bank-seed`, then `scripts/seed.py` sets up the following. It refuses to run unless `BANK_PROFILE` is `local`.

- **Workforce links and roles.** Links `reviewer`, `approver` and `security` to canonical persons and grants their roles for 365 days:
  - `reviewer` is the maker: product, case, finance, support, payment-operator, records, data and assisted-service roles;
  - `approver` is the independent checker;
  - `security` handles role administration and replay.
- **Queue membership.** Adds REVIEW (`reviewer`) and APPROVE (`approver`) membership to five queues: `controls-operations`, `support`, `payments-operations`, `finance-operations` and `data-operations`.
- **On-call roster.** `reviewer` is primary and `approver` secondary.
- **Settlement account.** Creates the zero-balance settlement ledger account `4f1a780b-3e31-48ef-a294-000000000003`. Synthetic funding draws from it in balanced postings.
- **Product configuration.** `bank-seed` publishes the TST product, its terms and the versioned GL charts through the real maker/checker paths: `reviewer` proposes and `approver` approves.

It creates no customer accounts and no money. Re-running it is safe: grants are refreshed and existing published versions are kept.

Expected last line: `Synthetic workforce roles and explicit queue relationships configured. Bootstrap is local-only.`

Check (17 active grants, 10 queue memberships, settlement account ACTIVE):

```sh
P='podman exec piggybank-postgres-1 psql -U dev_admin -d piggybank -At -c'
$P "SELECT count(*) FROM controls.role_grants WHERE revoked_at IS NULL"
$P "SELECT count(*) FROM controls.case_queue_members WHERE revoked_at IS NULL"
$P "SELECT kind, state FROM core.ledger_accounts WHERE id='4f1a780b-3e31-48ef-a294-000000000003'"
```

### 3.6 Start the applications

Choose one of two modes. Both build the workspace first, then start `bank-api`, `bank-worker`, `provider-simulator` and both Vite portals with the same defaults.

**Mode A: foreground (`make dev`).** Best while developing, because logs go to your terminal and Ctrl-C stops everything.

```sh
make dev
```

Expected: `Staff workbench: http://127.0.0.1:4301 · Customer portal: http://127.0.0.1:4300 · API: http://127.0.0.1:4180`. If the API is not ready within 15 seconds the run aborts, usually because a dependency is down; see [§10](#10-troubleshooting). Ctrl-C sends SIGTERM to each process group, waits 10 seconds, then sends SIGKILL.

**Mode B: persistent preview (`scripts/local_cx.py`).** Best for manual testing and demos. It keeps running after the terminal closes.

```sh
python3 scripts/local_cx.py prepare   # database roles + build + migrate + seed (one command for §3.3 and §3.5)
python3 scripts/local_cx.py start     # refuses to start if any of 4180–4301 is occupied
python3 scripts/local_cx.py status
python3 scripts/local_cx.py stop      # stops only the preview's own processes
```

- **Ownership.** `start` records the preview's process ID and start identity in `.local/cx-runtime.json` (mode 0600).
- **Readiness.** It waits up to 90 seconds for the API and both portals.
- **Logs.** Output goes to `.local/cx-runtime.log`.
- **Stopping.** `stop` only signals the process it started. It never touches containers, data or other processes.
- **Identity.** `prepare` does not run `make identity-setup`; run that yourself the first time.
- **Guard.** `prepare` refuses to run if any `*_DATABASE_URL` points away from `127.0.0.1:15432`.

## 4. Verify the deployment

Run these checks after every first deployment and every upgrade.

### 4.1 Health

```sh
curl -fsS http://127.0.0.1:4180/health/live;  echo   # {"status":"alive"...}
curl -fsS http://127.0.0.1:4180/health/ready; echo   # {"status":"ready","dependencies":{"postgres":true,"jetstream":true,"oidc":true}...}
curl -fsS http://127.0.0.1:4181/health/ready; echo   # worker, same shape
curl -fsS -o /dev/null -w '%{http_code}\n' http://127.0.0.1:4300   # 200
curl -fsS -o /dev/null -w '%{http_code}\n' http://127.0.0.1:4301   # 200
curl -fsS http://127.0.0.1:4180/openapi.json | head -c 80; echo
```

- **Readiness checks.** `/health/ready` checks PostgreSQL, JetStream and OIDC discovery in parallel, with a 1.5-second deadline. If any dependency is down it returns HTTP 503 with `"status":"degraded"` and the failed dependency set to `false`.
- **What readiness does not mean.** A ready response says the dependencies are reachable. It does not mean the system is financially or legally ready for anything real.

**Automated smoke test.** `make dev-smoke` builds the workspace, starts the API, worker and simulator, and asserts liveness and readiness on 4180 and 4181. Expected output: `API and worker liveness/readiness passed with real local dependencies.` It needs the ports free, so stop Mode A or B first.

### 4.2 Sign-in checks

1. **Customer.** Open `http://127.0.0.1:4300`, sign in as `alice` / `Synthetic-Local-123!`, and land on the home screen. `curl` cannot show this because the session cookie is HttpOnly; use the browser.
2. **Workforce.** Open `http://127.0.0.1:4301` and sign in as `reviewer`. The navigation should show the queues and the Finance & GL workspace.
3. **Sign out.** Sign out of both. The issuer session ends and you return to the signed-out page.

### 4.3 First money movement (optional)

1. **Open an account.** As `alice`, choose **Open an account**, fill in the synthetic profile and an `example.invalid` contact, attach the evidence text, accept the terms and submit. The worker completes the synthetic checks within a few seconds and the account becomes active.
2. **Fund it.** Go to **Cards & wallet → Link a simulated bank** and add `500.00` TST. The simulator allows 1,000 TST in total per person, across all links.
3. **Save into a vault.** Create a vault under **Savings & vaults** and move `50.00` into it.

The full guided journey is in the [local CX walkthrough](../implementation/ui/LOCAL_CX_TEST.md).

## 5. Using the deployment

### 5.1 Accounts and roles

| User | Portal | What they can do |
| --- | --- | --- |
| `alice`, `bob` | customer, 4300 | All customer journeys. The two are distinct persons, so payments between them are real transfers in the synthetic ledger. |
| `reviewer` | staff, 4301 | Maker and first reviewer: proposes product, terms, policy and correction changes, reviews cases and operates rails. |
| `approver` | staff, 4301 | Independent checker. A change cannot be approved by the same canonical person who proposed it. |
| `security` | staff, 4301 | Proposes and revokes role grants (approved by `approver`), runs security reviews and event replay. |

### 5.2 Fund a customer account directly

The simulated bank link (see [§4.3](#43-first-money-movement-optional)) is the normal route. For test setup you can instead post a balanced synthetic funding entry from the settlement account. This is a ledger fixture with a full journal record, not a balance override. It works only in the local profile.

```sh
# 1. Find the customer's Keycloak subject and their active accounts
SUB=$(python3 -c "import sys; sys.path.insert(0,'scripts'); from identity import admin_token, call; \
print(call(admin_token(),'customers/users?exact=true&username=alice')[0]['id'])")
podman exec piggybank-postgres-1 psql -U dev_admin -d piggybank -At -c "
  SELECT r.account_id, a.state, a.booked
  FROM controls.person_links l
  JOIN core.account_relationships r ON r.person_id=l.person_id AND r.revoked_at IS NULL
  JOIN core.ledger_accounts a ON a.id=r.account_id
  WHERE l.issuer='http://127.0.0.1:18280/realms/customers' AND l.subject='$SUB'"

# 2. Fund 1,000.00 TST (amounts are minor units: 100000 = 1,000.00)
target/debug/bank-fund "$(cat /proc/sys/kernel/random/uuid)" <account-id> 100000
```

- **Empty result.** If step 1 returns no rows, the customer has not opened an account yet.
- **Idempotency.** `bank-fund` prints the journal batch ID. Running it again with the same operation UUID posts nothing new.

### 5.3 Register a new customer and verify their email

- **Register.** On the customer sign-in page choose **Create account**. Registration requires a password of 8+ characters with an uppercase letter and a symbol, plus ticking the synthetic-demo terms. The new user returns signed in.
- **Verify the email.** **Profile & security** sends a 6-digit code. No email is sent; the code goes to a synthetic mailbox table, which you can read:

```sh
podman exec piggybank-postgres-1 psql -U dev_admin -d piggybank -At -c \
  "SELECT body FROM simulator.contact_outbox WHERE destination='<email>' ORDER BY created_at DESC LIMIT 1"
```

- **Code limits.** Codes expire after 10 minutes. Each code allows 5 attempts, and each person can request at most 5 codes per hour.
- **Cleaning up.** Delete self-registered users you no longer need in the Keycloak admin console (customers realm → Users). The realm should go back to listing only `alice` and `bob`.

### 5.4 Things that depend on the real clock

The worker runs periodic ticks against the real UTC clock. Nothing simulates elapsed time in the persistent deployment.

- **Savings interest** for a month is finalized and credited after the month closes. The current month shows a running estimate only.
- **Challenge and referral bonuses** pay when their conditions are met. A referral pays after 30 days at 100 TST or more.
- **Recurring SAVE/INVEST instructions** run on their next due UTC date.
- **Statements** become available after the month closes.
- **Dividends** are paid for seeded corporate actions on their record dates.

Disposable test databases override the clock; the persistent database does not.

## 6. Daily operation

### 6.1 Start and stop

| Task | Command | Effect |
| --- | --- | --- |
| Start containers | `make dev-up` | Starts or resumes the three containers; data is kept. |
| Start apps, foreground | `make dev` | Ctrl-C stops the apps only. |
| Start apps, background | `python3 scripts/local_cx.py start` | Persistent preview. |
| Stop apps, background | `python3 scripts/local_cx.py stop` | Containers and data are kept. |
| Stop containers | `make dev-down` | Runs `compose down` on the `piggybank` project only. Volumes are kept. Stop the apps first. |
| Container status | `python3 scripts/dev.py status` | |
| App status | `python3 scripts/local_cx.py status` | Reports whether this checkout owns a running preview. |

Starting after a reboot: `make dev-up`, then `make dev` or `python3 scripts/local_cx.py start`. You do not need to migrate, set up identity or seed again unless the source changed.

### 6.2 Logs

| Source | Where |
| --- | --- |
| Apps in Mode A | Your terminal |
| Apps in Mode B | `.local/cx-runtime.log`; follow it with `tail -f .local/cx-runtime.log` |
| PostgreSQL, Keycloak, NATS | `podman logs --tail 200 piggybank-postgres-1`, and likewise for `piggybank-keycloak-1` and `piggybank-nats-1` |
| Keycloak user and admin events | Admin console → realm → Events |
| JetStream state | `curl -s http://127.0.0.1:18222/jsz?streams=1` |
| Private evidence objects | `.local/evidence/`, set by `BANK_EVIDENCE_DIR` |
| CI evidence | `.local/ci/<UTC timestamp>/evidence.json` plus one log per step |

Logs and evidence never contain credentials or tokens. `.local/` is git-ignored.

### 6.3 Inspect data

Read-only queries as the admin role are safe:

```sh
podman exec -it piggybank-postgres-1 psql -U dev_admin -d piggybank
```

**Never change money tables by hand.** Runtime roles cannot change journal facts directly, and the admin role bypasses those protections. Use the product flows, `bank-fund`, or the [correction workflow](finance-corrections.md). A useful invariant check, which should return 0:

```sql
SELECT count(*) FROM (SELECT batch_id FROM core.journal_entries GROUP BY batch_id HAVING sum(delta)<>0) u;
```

### 6.4 Upgrade to new source

```sh
python3 scripts/local_cx.py stop          # or Ctrl-C on make dev
git pull
scripts/npm.sh ci                         # when package-lock.json changed
make identity-setup                       # when infra/identity or scripts/identity.py changed; safe to run anyway
python3 scripts/local_cx.py prepare       # build + forward migrations + reseed (keeps all existing data)
python3 scripts/local_cx.py start         # or make dev
```

Then run the [§4.1](#41-health) checks and confirm the schema version from [§3.3](#33-create-runtime-roles-and-apply-migrations).

- **Schema only moves forward.** Checking out an older commit does not downgrade the database. An older binary against a newer schema generally works because migrations are expand-only. An older schema against a newer binary does not.
- **Never edit or delete an applied migration.** The checksum lock fails, and the fix is a new forward migration. See the [upgrade runbook](deployment-upgrades.md).

### 6.5 Backups

Follow the [backup and restore runbook](backup-restore.md). Short version: run `pg_dump -Fc` inside the container, copy the dump to `.local/backups/`, and restore only into a new database. The dump does not include Keycloak users or secret files. To keep issuer state too, also dump the `keycloak` database.

### 6.6 Native mobile app

Follow the [mobile development runbook](mobile-development.md). It uses the same running stack: `adb reverse` for 4180 and 18280, with the `bank-mobile` client. Android works; iOS is still pending.

## 7. Running tests against a local deployment

Most suites create their own disposable PostgreSQL database and NATS stream on the shared containers, then remove them. They still bind the shared ports and share the server's connection budget, so:

1. Stop Mode A or Mode B first: `python3 scripts/local_cx.py stop`.
2. Run HTTP and browser suites one at a time.
3. Keep the containers running; the suites need them.

| Command | What it covers | Uses the persistent database? |
| --- | --- | --- |
| `make app-check` | rustfmt, clippy `-D warnings`, contract drift, TypeScript | No |
| `make test-unit` | Rust unit tests | No |
| `make test-integration` | Rust database suites (two threads) against disposable databases | No (creates `piggybank_test_*`) |
| `make test-customer`, `test-staff`, `test-payments`, `test-consumer` | Playwright Chromium journeys | No (isolated fixture database) |
| `make test-lifecycle`, `test-identity`, `test-events`, `test-ingress`, `test-security` | HTTP and process suites | No |
| `make test-foundation` | Restarts the PostgreSQL container and pauses NATS to check degraded readiness and fresh issuer logins | Data is kept, but the shared containers restart and pause |
| `make test-mobile-api`, `make test-mobile` | Native contract and Android emulator | No |
| `python3 scripts/ci.py` | The whole gated pipeline, stopping at the first failure | No |
| `python3 scripts/ci.py --apply-local --device` | Also migrates the persistent database, runs the port-bound suites and the Android journey (37 steps) | **Yes** (forward migration and seed only) |

For a single database test, run:

```sh
BANK_TEST_ADMIN_URL=postgres://dev_admin:synthetic-admin-only@127.0.0.1:15432/piggybank \
  scripts/cargo.sh test -p bank-core --test database -- <name-filter>
```

For a single browser test, run:

```sh
scripts/npm.sh exec -- playwright test --config playwright.consumer.config.ts -g "<title>"
```

`make test-load` and `make test-resilience` exist, but they are not part of routine local verification.

## 8. Configuration reference

All settings are optional. The defaults are the local fixture values. Set them in the environment of `make dev`, `scripts/local_cx.py` or the individual binary.

| Variable | Default | Used by | Purpose |
| --- | --- | --- | --- |
| `BANK_PROFILE` | `local` | all binaries, seed | Any other value makes the binaries exit. Only the local profile is implemented. |
| `BANK_BIND` | `127.0.0.1:<4180, 4181 or 4182>` | api, worker, simulator | Listen address. The simulator refuses non-loopback addresses. |
| `DATABASE_URL` | `postgres://bank_platform:synthetic-platform-only@127.0.0.1:15432/piggybank` | health, platform | Platform principal |
| `BANK_<MODULE>_DATABASE_URL` | `postgres://bank_<module>:synthetic-<module>-only@127.0.0.1:15432/piggybank` | each module | Per-module role, for example `BANK_CORE_DATABASE_URL` |
| `MIGRATION_DATABASE_URL` | `postgres://bank_migrator:synthetic-local-only@127.0.0.1:15432/piggybank` | bank-migrate | The only principal that can change the schema |
| `BANK_DB_POOL_MAX` | `5` (1 to 32) | all | Connection pool ceiling per URL; pools are lazy |
| `NATS_URL` | `nats://127.0.0.1:14222` | api, worker | JetStream |
| `BANK_EVENT_STREAM` / `BANK_EVENT_NAMESPACE` | `BANK_EVENTS` / `bank.events` | api, worker | Stream and subject prefix |
| `OIDC_ISSUER` | `http://127.0.0.1:18280/realms/customers` | health | Issuer probed by readiness |
| `BANK_CUSTOMER_WEB_ORIGIN` / `BANK_WORKFORCE_WEB_ORIGIN` | `http://127.0.0.1:4300` / `:4301` (set by `make dev`) | api | Allowed browser origins (CSRF and origin checks) |
| `BANK_CUSTOMER_WEB_PORT` / `BANK_WORKFORCE_WEB_PORT` | `4300` / `4301` | Vite | Portal ports |
| `BANK_API_PROXY_TARGET` | `http://127.0.0.1:4180` | Vite | Must be plain-HTTP loopback |
| `BANK_RATE_LIMIT` / `BANK_RATE_SCOPE` | `500` per minute / `bank-api` | api | Shared ingress quota. Browser test fixtures raise it to 5,000. |
| `BANK_EVIDENCE_DIR` | `.local/evidence` | api, worker | Private evidence object store |
| `BANK_SIMULATOR_URL` / `BANK_SIMULATOR_KEY` | `http://127.0.0.1:4182` / fixture | worker | Controls simulator |
| `BANK_RAIL_PROVIDER_BASE`, `BANK_RAIL_SIGNING_KEY`, `BANK_RAIL_B_SIGNING_KEY` | loopback 4182 / fixtures | payments | Simulated rails A and B |
| `BANK_FINANCE_PROVIDER_BASE`, `BANK_FINANCE_SIGNING_KEY` | loopback 4182 / fixture | finance | Simulated GL |
| `BANK_LIQUIDITY_SIGNING_KEY`, `BANK_ATM_NETWORK_KEY`, `BANK_BATCH_NETWORK_KEY`, `BANK_JOURNEY_PARTNER_KEY` | fixtures | various | Independent signing keys for each simulated counterparty |
| `BANK_SESSION_KEYS_FILE` | unset: fixed synthetic key | api | `kid:base64(32 bytes)` per line; the first line seals. Validated at startup. |
| `BANK_IDENTITY_ADMIN_SECRET` / `_FILE` | `synthetic-identity-admin-only` | api, identity.py | Secret for `bank-identity-admin`. A file must be mode 0600 with 16+ characters. |
| `BANK_METRICS_TOKEN_FILE` | unset: `/metrics` disabled | api | 32+ character bearer token file, mode 0600 |
| `CONTAINER_RUNTIME` | `podman` | dev.py, database.py | Container CLI; see the Docker note in [§2.2](#22-tools) |
| `KEYCLOAK_ADMIN_PASSWORD`, `BANK_IDENTITY_ADMIN_BASE` | fixture, `http://127.0.0.1:18280` | identity.py, seed.py | Keycloak admin access |
| `BANK_FIXTURE_PASSWORD` | `Synthetic-Local-123!` | identity.py | Password for new fixture users. Existing users keep theirs. |
| `BANK_CUSTOMER_REGISTRATION` | `enabled` | identity.py | `disabled` turns off self-registration |
| `BANK_REALM_DIR` | `infra/identity` | identity.py | Location of the realm JSON files |
| `BANK_TEST_ADMIN_URL` | none (tests fail without it) | database tests | Admin URL used to create disposable test databases |

## 9. Security boundaries of the local profile

- **Loopback only.** Every port is bound to `127.0.0.1`. Do not change the Compose bindings or `BANK_BIND` to expose the stack on a network. A phone or another machine cannot use these URLs; use the emulator with `adb reverse`.
- **Fixture credentials are public.** Never copy them into a real environment, and never put real personal data, real card numbers or real money identifiers into this stack.
- **Session cookies** are HttpOnly and SameSite=Lax, without the Secure flag because the stack uses plain HTTP. The BFF holds the tokens, and browser storage holds only non-secret references.
- **TST has no monetary value.** Interest, rewards and investment results are labeled as synthetic. They make no compliance claim, and the Brazilian legal review of the savings rules is still pending.

## 10. Troubleshooting

| Symptom | Likely cause | Fix |
| --- | --- | --- |
| `make dev-up` prints `Timed out waiting for http://127.0.0.1:18280/...` | Keycloak still starting, or crashed because PostgreSQL wasn't ready | `podman logs --tail 100 piggybank-keycloak-1`. Run `make dev-up` again; it is idempotent. |
| `Error: ... address already in use` / `Port 4180 is occupied` | Another run, preview or test suite owns the port | Run `ss -ltnp \| grep -E ':(4180\|4181\|4182\|4300\|4301)\b'` to find the owner. Stop it through the tool that started it (`local_cx.py stop`, or Ctrl-C). Do not kill processes you don't recognize. |
| `make dev` exits with `Timed out waiting for http://127.0.0.1:4180/health/ready` | A dependency is down (containers stopped, migrations missing) | `python3 scripts/dev.py status`, then `curl http://127.0.0.1:4180/health/ready` in a separate run to see which dependency is `false`. Run `make dev-up` and `make migrate`. |
| Readiness shows `"postgres":false` | Container stopped, roles missing, or pool exhausted | `make dev-up`, then `python3 scripts/database.py`. Check `BANK_DB_POOL_MAX` against `max_connections`. |
| Readiness shows `"oidc":false` | Keycloak down or restarting | Wait, then re-check. After a PostgreSQL restart Keycloak can serve discovery before logins work; retry the login form. |
| Binary exits with `Only the synthetic local profile is implemented` | `BANK_PROFILE` is set to something other than `local` | `unset BANK_PROFILE` |
| `Node 24.15.0 is required` | Wrong Node selected | `nvm install 24.15.0`. `scripts/npm.sh` picks it up from `~/.nvm` automatically. |
| `Install Rust 1.97.0 with rustup` | Toolchain missing | `rustup toolchain install 1.97.0 --profile minimal --component rustfmt --component clippy` |
| `podman compose` prints "Executing external compose provider" | Informational: Podman is delegating to the Docker Compose plugin | Nothing to fix. |
| Sign-in loops back to the login page, or "invalid redirect" | Portal opened on `localhost`, or a stale cookie | Use `http://127.0.0.1:4300`. Clear that origin's cookies. |
| Signed-in user gets `user_not_found` or sign-in fails for alice or bob | Customers realm was changed (for example, email-as-username enabled) | Turn `registrationEmailAsUsername` off in the realm, then run `make identity-setup`. Check usernames as in [§3.4](#34-configure-the-identity-provider). |
| Session ends quickly | Access tokens last 30 seconds by design; refresh failed | Check the API log for refresh errors. Signing out everywhere revokes every session. |
| `bank-migrate` fails with a checksum or "previously applied but modified" error | An applied migration file was edited locally | Restore it with `git checkout -- migrations/<file>`. Put changes in a new migration. |
| `make dev-seed` fails with `podman: not found` | Docker-only host | Install Podman (see [§2.2](#22-tools)). |
| `make dev-seed` fails with HTTP 401 from Keycloak | Admin password changed | Export `KEYCLOAK_ADMIN_PASSWORD` with the current value. |
| Browser tests fail with port or fixture errors | Preview or `make dev` still running, or another suite running at the same time | `python3 scripts/local_cx.py stop`, then run the suites one at a time. |
| A customer action stays pending | Worker not running or not ready | `curl http://127.0.0.1:4181/health/ready`. Check the worker log for `PENDING` warnings. |
| Disk full during build | `target/` growth | `scripts/cargo.sh clean`, then rebuild. |

When asking for help, collect the following. None of it contains secrets.

```sh
git rev-parse HEAD; python3 scripts/dev.py status
curl -s http://127.0.0.1:4180/health/ready; curl -s http://127.0.0.1:4181/health/ready
podman exec piggybank-postgres-1 psql -U dev_admin -d piggybank -At -c "SELECT max(version) FROM _sqlx_migrations"
tail -n 100 .local/cx-runtime.log
```

## 11. Teardown

### 11.1 Stop and keep everything

```sh
python3 scripts/local_cx.py stop   # or Ctrl-C on make dev
make dev-down                      # containers removed; postgres-data and nats-data volumes kept
```

`make dev-up` brings back the same data, users and balances.

### 11.2 Full reset (destructive)

This permanently deletes every account, journal, receipt, Keycloak user change and event in this deployment. Take a backup first ([§6.5](#65-backups)) if anything matters. It affects only the `piggybank` Compose project's volumes.

```sh
python3 scripts/local_cx.py stop
podman compose -p piggybank -f infra/compose/compose.yaml down -v   # removes postgres-data and nats-data
podman volume ls | grep piggybank                                   # expect no output
```

Then redeploy from [§3.2](#32-start-the-infrastructure-containers): `make dev-up`, `make migrate`, `make identity-setup`, `make dev-seed`, then start the apps. The schema is rebuilt from all migrations, and Keycloak re-imports the realm files.

Local evidence under `.local/` (CI runs, screenshots, preview logs, evidence objects) is kept. Delete specific subfolders yourself if you need to; the private evidence store is `.local/evidence/`.

## 12. Related runbooks

- [Local development notes](local-development.md): background on runtime behavior, payment, finance and policy slices.
- [Local CX walkthrough](../implementation/ui/LOCAL_CX_TEST.md): the step-by-step customer and staff journey.
- [Deployment, upgrades and rollback](deployment-upgrades.md): expand, deploy, contract; the secrets interface.
- [Backup and restore](backup-restore.md).
- [Native mobile development](mobile-development.md).
- [Event replay](event-replay.md), [security incidents](security-incident.md) and [on-call](oncall.md).
- [Cloud deployment packages](../deployment/README.md): separate, sealed releases for GCP and AWS.
