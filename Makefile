.PHONY: check diagrams catalog docs plan plan-check plan-next dev-up dev-down dev dev-smoke app-check test-unit
.PHONY: test-foundation migrate test-integration identity-setup test-identity test-browser contracts test-ingress

check:
	python3 scripts/implementation_plan.py check
	python3 scripts/build_catalog.py --check
	python3 scripts/validate_docs.py
	python3 scripts/render_diagrams.py --check

diagrams:
	python3 scripts/render_diagrams.py
	python3 scripts/build_catalog.py

catalog:
	python3 scripts/build_catalog.py

docs: diagrams check

plan:
	python3 scripts/implementation_plan.py render

plan-check:
	python3 scripts/implementation_plan.py check

plan-next:
	python3 scripts/implementation_plan.py next

dev-up:
	python3 scripts/dev.py up

dev-down:
	python3 scripts/dev.py down

dev:
	python3 scripts/dev.py run

dev-smoke:
	python3 scripts/dev.py smoke

app-check:
	scripts/cargo.sh fmt --all -- --check
	scripts/cargo.sh clippy --workspace --all-targets --locked -- -D warnings
	python3 scripts/contracts.py --check
	scripts/npm.sh run check --workspace @piggybank/api-client
	$(MAKE) web-check

test-unit:
	scripts/cargo.sh test --workspace --locked --lib

test-foundation:
	scripts/cargo.sh build --workspace --locked
	python3 tests/integration/foundation.py

migrate:
	python3 scripts/database.py
	scripts/cargo.sh run --locked --bin bank-migrate

identity-setup:
	python3 scripts/identity.py

test-identity:
	scripts/cargo.sh build --workspace --locked
	python3 tests/integration/identity.py

test-browser:
	scripts/npm.sh exec playwright test

contracts:
	python3 scripts/contracts.py

test-ingress:
	scripts/cargo.sh build --workspace --locked
	python3 tests/integration/ingress.py

test-integration:
	scripts/cargo.sh build --locked --bin provider-simulator
	python3 scripts/database.py
	BANK_TEST_ADMIN_URL=postgres://dev_admin:synthetic-admin-only@127.0.0.1:15432/piggybank scripts/cargo.sh test --workspace --locked --test database -- --test-threads=2

.PHONY: test-events
test-events:
	scripts/cargo.sh build --workspace --locked
	python3 tests/integration/events.py

.PHONY: dev-seed
dev-seed:
	scripts/cargo.sh build --locked --bin bank-seed
	python3 scripts/seed.py

.PHONY: test-lifecycle
test-lifecycle:
	scripts/cargo.sh build --workspace --locked
	python3 tests/integration/lifecycle.py

.PHONY: web-check web-build test-customer
web-check:
	scripts/npm.sh run check --workspace @piggybank/web-core
	scripts/npm.sh run check --workspace @piggybank/customer-web
	scripts/npm.sh run check --workspace @piggybank/staff-web

web-build:
	scripts/npm.sh run build --workspace @piggybank/customer-web
	scripts/npm.sh run build --workspace @piggybank/staff-web

test-customer:
	scripts/cargo.sh build --workspace --locked
	scripts/npm.sh exec -- playwright test --config playwright.customer.config.ts

.PHONY: test-staff
test-staff:
	scripts/cargo.sh build --workspace --locked
	scripts/npm.sh exec -- playwright test --config playwright.staff.config.ts

.PHONY: test-payments
test-payments:
	scripts/cargo.sh build --workspace --locked
	scripts/npm.sh exec -- playwright test --config playwright.payments.config.ts

.PHONY: cloud-check
cloud-check:
	python3 tests/deployment/check.py

.PHONY: ui-sources test-consumer cx-prepare cx-start cx-stop
ui-sources:
	python3 scripts/validate_ui_sources.py

test-consumer:
	scripts/cargo.sh build --workspace --locked
	scripts/npm.sh exec -- playwright test --config playwright.consumer.config.ts

cx-prepare:
	python3 scripts/local_cx.py prepare

cx-start:
	python3 scripts/local_cx.py start

cx-stop:
	python3 scripts/local_cx.py stop

.PHONY: mobile-check mobile-build test-mobile-api test-mobile
mobile-check:
	scripts/mobile.sh install
	scripts/mobile.sh check

mobile-build:
	scripts/mobile.sh build

test-mobile-api:
	scripts/cargo.sh build --workspace --locked
	python3 tests/mobile/native_api.py

# Android emulator journey; iOS requires macOS/Xcode and is reported as pending.
test-mobile: test-mobile-api mobile-build
	python3 tests/mobile/device.py

.PHONY: test-security
test-security:
	scripts/cargo.sh build --workspace --locked
	python3 tests/acceptance/security/security.py

.PHONY: migrations-check migrations-lock test-deployment ci
migrations-check:
	python3 scripts/migrations.py check

# Run after a migration is applied to a shared environment (it becomes immutable).
migrations-lock:
	python3 scripts/migrations.py lock

test-deployment:
	scripts/cargo.sh build --workspace --locked
	python3 tests/integration/deployment/config_failure.py
	python3 tests/integration/deployment/operations.py

# Local CI-equivalent pipeline with sanitized evidence in .local/ci/<timestamp>/ (no remote host is configured).
ci:
	python3 scripts/ci.py

.PHONY: acceptance test-acceptance test-load test-load-measured test-resilience
# Branch-level acceptance manifest (WP-30): every R1 scenario branch has evidence or an explicit reason.
acceptance:
	python3 scripts/acceptance.py

test-acceptance: acceptance

# Short load smoke on optimised binaries; the measured qualification run is test-load-measured.
test-load:
	scripts/cargo.sh build --release --locked -p bank-api -p bank-worker -p provider-simulator
	python3 tests/load/run.py --sustained 60 --burst 30 --core-seconds 15

test-load-measured:
	scripts/cargo.sh build --release --locked -p bank-api -p bank-worker -p provider-simulator
	python3 tests/load/run.py --sustained 600 --burst 300 --core-seconds 60

test-resilience:
	scripts/cargo.sh build --workspace --locked
	python3 tests/resilience/drills.py

.PHONY: test-partners
# WP-33 partner consent, versioning, quota, signed callbacks, AT-21 in-flight revocation and delegation.
test-partners:
	scripts/cargo.sh build --workspace --locked
	python3 scripts/identity.py
	python3 tests/acceptance/partners/partners.py

.PHONY: test-assisted
# WP-34 assisted channels: customer authorization, channel policy, common restrictions and audit (AT-22).
test-assisted:
	scripts/cargo.sh build --workspace --locked
	python3 tests/acceptance/assisted/assisted.py

.PHONY: test-atm
# WP-35 ATM channel against the network simulator role (includes a real 120 s authorization timeout).
test-atm:
	scripts/cargo.sh build --workspace --locked
	python3 tests/acceptance/atm/atm.py

.PHONY: test-batch
# WP-36 synthetic batch credit rail (the test plays the network).
test-batch:
	scripts/cargo.sh build --workspace --locked
	python3 tests/acceptance/batch/batch.py

.PHONY: test-analytics
# WP-37 governed warehouse with one local adapter (staff API, real issuer and worker).
test-analytics:
	scripts/cargo.sh build --workspace --locked
	python3 tests/acceptance/analytics/analytics.py

.PHONY: test-journeys
# WP-38 audiences, insights and the local journey-partner simulator role (the test plays the partner).
test-journeys:
	scripts/cargo.sh build --workspace --locked
	python3 tests/acceptance/journeys/journeys.py

.PHONY: test-routing
# WP-39 routing across two simulated providers (real issuer, worker and simulator).
test-routing:
	scripts/cargo.sh build --workspace --locked
	python3 tests/acceptance/routing/routing.py

.PHONY: test-liquidity
# WP-40 liquidity forecasting and controlled finance automation (real issuer, worker and simulators).
test-liquidity:
	scripts/cargo.sh build --workspace --locked
	python3 tests/acceptance/liquidity/liquidity.py

.PHONY: test-outcomes
# WP-41 outcome KPIs (local capacity template; real issuer, worker and simulator).
test-outcomes:
	scripts/cargo.sh build --workspace --locked
	python3 tests/acceptance/outcomes/outcomes.py
