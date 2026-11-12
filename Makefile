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
