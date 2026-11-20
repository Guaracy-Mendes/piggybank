# Piggybank — Digital Bank


The [official-template consumer and investment preview](docs/implementation/ui/ARCHITECTURE.md) extends the essential banking baseline. Use the [local end-to-end CX walkthrough](docs/implementation/ui/LOCAL_CX_TEST.md) and [32-screen coverage record](docs/implementation/ui/SCREEN_COVERAGE.md).

This repository contains a proposed product requirements baseline, editable C4/UML models, and an evolving synthetic local application based on the supplied [Digital Bank Logical Architecture](Digital%20Bank%20Logical%20Architecture.jpeg). The [current implementation](docs/implementation/README.md) distinguishes working capabilities from the full target design.

**Product baseline:** Draft v1.0, 2026-10-22. The image is the only product source supplied. Requirements, release boundaries, interfaces, deployment topology, and numerical targets are proposals unless explicitly marked as observations from that image. Implementation progress and evidence are tracked separately in the backlog.

| Document | Purpose |
| --- | --- |
| [Implementation completion status](docs/implementation/COMPLETION_STATUS.md) | Current package counts, completed scope, remaining work, validation and deployment status |
| [Current implementation and runtime diagrams](docs/implementation/README.md) | Implemented scope, local C4/UML models and explicit remaining work |
| [Implementation plan](docs/IMPLEMENTATION_PLAN.md) | Execution order, technical baseline, local/provider boundaries, validation and release gates |
| [Implementation work packages](docs/implementation/WORK_PACKAGES.md) | 41 actionable packages with dependencies, requirement allocation and completion conditions |
| [Product requirements](docs/PRD.md) | Vision, scope, personas, journeys, functional requirements, business rules, quality targets, delivery, risks, and acceptance |
| [Architecture specification](docs/ARCHITECTURE.md) | Source-to-target mapping, ownership, integration contracts, ledger behavior, events, security, and deployment |
| [Traceability and validation](docs/TRACEABILITY.md) | Every source capability, requirement group, model, and acceptance scenario linked together |
| [Diagram catalog and gallery](docs/diagrams/README.md) | C4 levels 1–4, dynamic and deployment views, plus detailed UML models, with SVG previews |
| [GCP and AWS deployment packages](docs/deployment/README.md) | Step-by-step cloud runbooks, Terraform, containers, operator tunnels and matched backup/restore |
| [Validation record](docs/VALIDATION.md) | Checks actually performed on this documentation package and their limits |

The source image credits **AlphaMille / Victor Yaromin** as its source and **Simon Koci** for the visual. Those credits describe the supplied image; they are not an endorsement of this proposed specification.

WP-01–WP-24 are complete within documented local synthetic scope. The [essential WP-24 completion evidence](docs/implementation/evidence/WP-24-complete.md) and [scope decision](docs/implementation/wp24-essential-scope.md) record the financial lifecycle and deliberate deferrals. Work stops before WP-25; full R1 and production qualification remain outstanding.

## Read and render

Start with the PRD, then the C4 context and container diagrams. Follow the catalog to the detailed models. All diagrams have editable `.puml` sources and rendered `.svg` copies, so reading does not require a PlantUML extension.

```sh
make check       # Check Markdown references, traceability, and PlantUML syntax
make diagrams    # Render all PlantUML diagrams to SVG
make docs        # Check and render the complete package
make plan        # Refresh the work-package board from the implementation backlog
make plan-check  # Validate dependencies, ownership and board synchronization
make plan-next   # Print the next implementation package and its completion conditions
```

Prerequisites: Python 3.10+, Java 17+, Graphviz `dot`, and `make`. The first diagram command downloads the pinned PlantUML JAR from its official GitHub release and verifies its SHA-256. Later runs use the local cache. The bundled C4 library supplies all C4 includes; diagram sources do not fetch remote includes. See the [rendering guide](docs/diagrams/README.md#rendering).

The implementation plan was added on 2026-10-23. Application implementation is tracked in the [backlog](docs/implementation/backlog.json). `make check` includes plan consistency checks. The plan distinguishes synthetic local qualification, actual provider integration and controlled production launch.

## Run locally

The [local development runbook](docs/runbooks/local-development.md) describes the current runtime and its limitations. Use the pinned Rust and Node toolchains, then run:

```sh
make dev-up
make migrate
make identity-setup
make dev-seed
make dev
```

The customer portal is at `http://127.0.0.1:4300`, the workforce portal is at `http://127.0.0.1:4301`, and the API is at `http://127.0.0.1:4180`, with `/health/live` and `/health/ready`; the worker health port is `4181`, and the synthetic provider process is at `4182`. `make dev-down` retains the project's persistent data volumes. Product capabilities become available as their work packages are implemented; real providers and production activation remain separate gates.

## Evidence convention

- **Observed:** explicitly visible in the supplied image.
- **Proposed:** added here to make the product actionable and testable.
- **TBD:** needs a named business or technical decision before the affected release gate.

The diagram's **L2** badges mean “details in L2 architecture” according to its legend. They do not establish C4 levels or implementation boundaries. C4 levels in this package are explicitly modeled proposals, using the [official C4 hierarchy](https://c4model.com/diagrams.html) and [C4-PlantUML notation](https://github.com/plantuml-stdlib/C4-PlantUML).

Periodic accounting configuration now has a [complete specification](docs/implementation/periodic-accounting-policies.md), [operator runbook](docs/runbooks/periodic-accounting-policies.md), exact example inputs and C4/UML views. It supports independent approval and immutable preview policies; periodic customer posting and linked corrections remain open in WP-24.

The [controlled funding correction workflow](docs/implementation/finance-corrections.md) adds exact-source independent approval, one native linked reversal and original receipt recovery. Its [runbook](docs/runbooks/finance-corrections.md) covers local/cloud fixture funding, financial review/execution and GL lineage. Periodic charges/accruals remain disabled pending their enrollment, economic finalization and posting work.

[Native periodic basis observations](docs/implementation/periodic-native-basis.md) now preserve the exact committed Core business-date journal prefix, source provenance and capture clock. The [operator runbook](docs/runbooks/periodic-native-basis.md) covers immutable replay and fresh evidence. These observations support previews without posting money or establishing customer enrollment.

[Durable Finance audit delivery](docs/implementation/finance-audit-delivery.md) now preserves original records while progressing past failed events. The [runbook](docs/runbooks/finance-audit-delivery.md) covers native receipt confirmation, retained pending records and forward-upgrade checks.

[Customer periodic disclosures](docs/implementation/customer-periodic-disclosures.md) expose exact approved preview rules for owned accounts. The [runbook](docs/runbooks/customer-periodic-disclosures.md) explains material digests and saved versions. This information does not enroll accounts or enable charges.

[Retained preview reviews](docs/implementation/periodic-preview-reviews.md) add explicit customer acknowledgment, exact archived material and original receipt recovery. The [runbook](docs/runbooks/periodic-preview-reviews.md) covers response loss, historical evidence and integrity failures. These records do not establish financial consent or enrollment.
