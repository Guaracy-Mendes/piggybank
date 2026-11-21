# Digital Bank — Source, Requirement and Acceptance Traceability

**Baseline:** 1.0, 2026-10-22. Source evidence is the supplied *Digital Bank Logical Architecture* image (kept outside this repository), transcribed in the [architecture inventory](ARCHITECTURE.md#source-capability-inventory). All requirements and test scenarios are proposals. This map establishes documentation coverage, not implementation completeness or executed product-test results.

## Coverage convention

Each `SRC` row represents one distinct label/capability from the image. Repeated journey-analytics labels share SRC-44, with the repetition explicitly retained in the architecture inventory. Grouped source boundaries and datastore icons are described in that inventory rather than invented as independent products. The source's L2 badges are preserved separately from C4 levels.

Functional requirement groups below expand to the individually numbered rows of [PRD section 7](PRD.md#7-functional-requirements). Every one of the 77 functional requirements has at least one direct reference in the [acceptance scenario table](PRD.md#16-acceptance-and-test-strategy). Priority and release remain attached to the individual requirement; this matrix does not accelerate R2/R3 features into R1.

## Source-to-requirement coverage

| Source ID | Observed capability | Requirement group(s) | Acceptance scenarios | Detailed models |
| --- | --- | --- | --- | --- |
| SRC-01 | Mobile Banking | FR-CH, FR-IAM | AT-01, AT-14, AT-19, AT-26 | [C4 L2](diagrams/c4/02-containers.puml), [authorization](diagrams/uml/12-authorization-sequence.puml) |
| SRC-02 | Desktop Banking | FR-CH, FR-IAM | AT-01, AT-14, AT-19, AT-26 | [C4 L2](diagrams/c4/02-containers.puml), [use cases](diagrams/uml/01-use-cases.puml) |
| SRC-03 | Branch | FR-CH, FR-IAM | AT-22 | [C4 L2](diagrams/c4/02-containers.puml), [authorization](diagrams/uml/12-authorization-sequence.puml) |
| SRC-04 | Private Banker Portal | FR-CH, FR-IAM | AT-22 | [C4 L2](diagrams/c4/02-containers.puml), [use cases](diagrams/uml/01-use-cases.puml) |
| SRC-05 | ATM | FR-CH, FR-EXT | AT-22, AT-28 | [C4 L2](diagrams/c4/02-containers.puml), [deployment](diagrams/c4/09-deployment.puml) |
| SRC-06 | Call Center | FR-CH, FR-IAM | AT-22 | [C4 L2](diagrams/c4/02-containers.puml), [authorization](diagrams/uml/12-authorization-sequence.puml) |
| SRC-07 | API Gateway | FR-CH, FR-IAM | AT-01, AT-27 | [C4 L2](diagrams/c4/02-containers.puml), [deployment](diagrams/c4/09-deployment.puml) |
| SRC-08 | Open APIs | FR-EXT, FR-IAM | AT-21, AT-27 | [context](diagrams/c4/01-context.puml), [authorization](diagrams/uml/12-authorization-sequence.puml) |
| SRC-09 | Experience APIs | FR-CH, FR-DATA | AT-01, AT-26, AT-27 | [C4 L2](diagrams/c4/02-containers.puml), [authorization](diagrams/uml/12-authorization-sequence.puml) |
| SRC-10 | IAM | FR-IAM | AT-14, AT-21, AT-29 | [authorization](diagrams/uml/12-authorization-sequence.puml), [controls](diagrams/c4/06-controls-components.puml) |
| SRC-11 | Orchestration | FR-ONB, FR-ACC, FR-OPS | AT-03, AT-05, AT-13, AT-26 | [Journey L3](diagrams/c4/03-onboarding-components.puml), [events](diagrams/uml/11-event-components.puml) |
| SRC-12 | Customer Acquisition | FR-ACQ | AT-02, AT-23, AT-26 | [Journey L3](diagrams/c4/03-onboarding-components.puml), [application state](diagrams/uml/08-application-state.puml) |
| SRC-13 | New Customer Onboarding | FR-ONB, FR-CTRL | AT-02, AT-03, AT-04, AT-05 | [onboarding sequence](diagrams/uml/03-onboarding-sequence.puml), [application state](diagrams/uml/08-application-state.puml) |
| SRC-14 | New Account Opening | FR-ACC, FR-DOC | AT-05 | [opening activity](diagrams/uml/04-account-opening-activity.puml), [account state](diagrams/uml/09-account-state.puml) |
| SRC-15 | Account Servicing | FR-ACC, FR-IAM | AT-01, AT-11, AT-21, AT-26 | [account state](diagrams/uml/09-account-state.puml), [exception activity](diagrams/uml/10-exception-activity.puml) |
| SRC-16 | Payment & Money Movement | FR-PAY | AT-06, AT-07, AT-08, AT-09, AT-10, AT-11 | [dynamic](diagrams/c4/08-payment-dynamic.puml), [payment sequence](diagrams/uml/05-payment-sequence.puml), [state](diagrams/uml/07-payment-state.puml) |
| SRC-17 | Accounts & Parties | FR-CORE, FR-ACC | AT-02, AT-05, AT-06, AT-11, AT-26 | [Core L3](diagrams/c4/04-core-components.puml), [domain](diagrams/uml/02-domain-model.puml) |
| SRC-18 | Product Mgmt and Pricing | FR-CORE, FR-ACQ | AT-05, AT-24, AT-26 | [Core L3](diagrams/c4/04-core-components.puml), [domain](diagrams/uml/02-domain-model.puml) |
| SRC-19 | Journal & Sub-ledger | FR-CORE, FR-FIN | AT-06, AT-07, AT-10, AT-12, AT-24 | [Core L3](diagrams/c4/04-core-components.puml), [reconciliation](diagrams/uml/06-reconciliation-sequence.puml) |
| SRC-20 | Events & Core Workflow | FR-CORE, FR-OPS | AT-07, AT-13, AT-17 | [Core L3](diagrams/c4/04-core-components.puml), [events](diagrams/uml/11-event-components.puml) |
| SRC-21 | Alerts & Notifications | FR-DATA, FR-DOC, FR-OPS | AT-13, AT-15, AT-20 | [events](diagrams/uml/11-event-components.puml), [documents](diagrams/uml/13-document-sequence.puml) |
| SRC-22 | Customer Service | FR-DATA, FR-ACC | AT-01, AT-23, AT-26, AT-27 | [C4 L2](diagrams/c4/02-containers.puml), [events](diagrams/uml/11-event-components.puml) |
| SRC-23 | Account Service | FR-DATA, FR-ACC | AT-01, AT-06, AT-18, AT-26 | [Core L3](diagrams/c4/04-core-components.puml), [domain](diagrams/uml/02-domain-model.puml) |
| SRC-24 | Audience Service | FR-DATA, FR-ACQ | AT-23 | [events](diagrams/uml/11-event-components.puml), [C4 L2](diagrams/c4/02-containers.puml) |
| SRC-25 | Tx Service | FR-DATA, FR-PAY | AT-07, AT-27, AT-28 | [Payments L3](diagrams/c4/05-payments-components.puml), [payment sequence](diagrams/uml/05-payment-sequence.puml) |
| SRC-26 | Transaction Service | FR-DATA, FR-ACC | AT-01, AT-18, AT-26, AT-27 | [events](diagrams/uml/11-event-components.puml), [C4 L2](diagrams/c4/02-containers.puml) |
| SRC-27 | Insight Service | FR-DATA | AT-23 | [events](diagrams/uml/11-event-components.puml), [C4 L2](diagrams/c4/02-containers.puml) |
| SRC-28 | Big Data Platform | FR-DATA | AT-13, AT-23 | [events](diagrams/uml/11-event-components.puml), [deployment](diagrams/c4/09-deployment.puml) |
| SRC-29 | Mastered Data | FR-DATA, FR-ONB | AT-02, AT-23 | [events](diagrams/uml/11-event-components.puml), [domain](diagrams/uml/02-domain-model.puml) |
| SRC-30 | Data Warehouse | FR-DATA, FR-FIN | AT-23, AT-25 | [events](diagrams/uml/11-event-components.puml), [C4 L2](diagrams/c4/02-containers.puml) |
| SRC-31 | Approvals & Routing | FR-PAY, FR-IAM | AT-14, AT-24, AT-28 | [Payments L3](diagrams/c4/05-payments-components.puml), [authorization](diagrams/uml/12-authorization-sequence.puml) |
| SRC-32 | Execution (Clearing, Settlement) | FR-PAY, FR-EXT, FR-FIN | AT-06, AT-07, AT-08, AT-09, AT-10, AT-11, AT-12 | [Payments L3](diagrams/c4/05-payments-components.puml), [payment L4](diagrams/c4/07-payment-code.puml), [reconciliation](diagrams/uml/06-reconciliation-sequence.puml) |
| SRC-33 | General Ledger | FR-FIN, FR-CORE | AT-12, AT-24 | [reconciliation](diagrams/uml/06-reconciliation-sequence.puml), [domain](diagrams/uml/02-domain-model.puml) |
| SRC-34 | Financial Reporting & Liquidity | FR-FIN | AT-12, AT-24, AT-25 | [reconciliation](diagrams/uml/06-reconciliation-sequence.puml), [C4 L2](diagrams/c4/02-containers.puml) |
| SRC-35 | Terms & Conditions | FR-DOC, FR-ACC | AT-05, AT-16 | [documents](diagrams/uml/13-document-sequence.puml), [opening activity](diagrams/uml/04-account-opening-activity.puml) |
| SRC-36 | Statements | FR-DOC | AT-15, AT-16 | [documents](diagrams/uml/13-document-sequence.puml), [domain](diagrams/uml/02-domain-model.puml) |
| SRC-37 | Ops Reporting | FR-CTRL, FR-OPS | AT-20, AT-25 | [Controls L3](diagrams/c4/06-controls-components.puml), [exception activity](diagrams/uml/10-exception-activity.puml) |
| SRC-38 | Enterprise Entitlements | FR-IAM, FR-CTRL | AT-01, AT-14, AT-21, AT-29 | [Controls L3](diagrams/c4/06-controls-components.puml), [authorization](diagrams/uml/12-authorization-sequence.puml) |
| SRC-39 | Sanction Screening | FR-CTRL, FR-ONB, FR-PAY | AT-04, AT-25, AT-27 | [Controls L3](diagrams/c4/06-controls-components.puml), [onboarding sequence](diagrams/uml/03-onboarding-sequence.puml) |
| SRC-40 | Risk & Compliance | FR-CTRL | AT-04, AT-25, AT-27 | [Controls L3](diagrams/c4/06-controls-components.puml), [exception activity](diagrams/uml/10-exception-activity.puml) |
| SRC-41 | Fraud & AML | FR-CTRL, FR-PAY | AT-25, AT-30 | [Controls L3](diagrams/c4/06-controls-components.puml), [events](diagrams/uml/11-event-components.puml) |
| SRC-42 | Identity Verification | FR-ONB, FR-CTRL | AT-03, AT-04 | [Controls L3](diagrams/c4/06-controls-components.puml), [onboarding sequence](diagrams/uml/03-onboarding-sequence.puml) |
| SRC-43 | Event Bus | FR-CORE, FR-DATA, FR-OPS | AT-13, AT-23, AT-28, AT-30 | [events](diagrams/uml/11-event-components.puml), [deployment](diagrams/c4/09-deployment.puml) |
| SRC-44 | Journey Analytics & Orchestration (repeated) | FR-EXT, FR-DATA, FR-ACQ | AT-23, AT-28 | [events](diagrams/uml/11-event-components.puml), [context](diagrams/c4/01-context.puml) |
| SRC-45 | ACH | FR-PAY, FR-EXT, FR-FIN | AT-08, AT-09, AT-10, AT-11, AT-12, AT-28 | [Payments L3](diagrams/c4/05-payments-components.puml), [rail matrix](ARCHITECTURE.md#rail-capability-matrix) |
| SRC-46 | Checks | FR-PAY, FR-EXT, FR-FIN | AT-08, AT-09, AT-10, AT-11, AT-12, AT-28 | [Payments L3](diagrams/c4/05-payments-components.puml), [rail matrix](ARCHITECTURE.md#rail-capability-matrix) |
| SRC-47 | Debit | FR-PAY, FR-EXT, FR-FIN | AT-08, AT-09, AT-10, AT-11, AT-12, AT-28 | [Payments L3](diagrams/c4/05-payments-components.puml), [rail matrix](ARCHITECTURE.md#rail-capability-matrix) |
| SRC-48 | Bill pay | FR-PAY, FR-EXT, FR-FIN | AT-08, AT-09, AT-10, AT-11, AT-12, AT-28 | [Payments L3](diagrams/c4/05-payments-components.puml), [rail matrix](ARCHITECTURE.md#rail-capability-matrix) |
| SRC-49 | Wire | FR-PAY, FR-EXT, FR-FIN | AT-08, AT-09, AT-10, AT-11, AT-12, AT-28 | [Payments L3](diagrams/c4/05-payments-components.puml), [rail matrix](ARCHITECTURE.md#rail-capability-matrix) |
| SRC-50 | Lockbox | FR-PAY, FR-EXT, FR-FIN | AT-08, AT-09, AT-10, AT-11, AT-12, AT-28 | [Payments L3](diagrams/c4/05-payments-components.puml), [rail matrix](ARCHITECTURE.md#rail-capability-matrix) |
| SRC-51 | Swift | FR-PAY, FR-EXT, FR-FIN | AT-08, AT-09, AT-10, AT-11, AT-12, AT-28 | [Payments L3](diagrams/c4/05-payments-components.puml), [rail matrix](ARCHITECTURE.md#rail-capability-matrix) |
| SRC-52 | P2P | FR-PAY, FR-EXT, FR-FIN | AT-08, AT-09, AT-10, AT-11, AT-12, AT-28 | [Payments L3](diagrams/c4/05-payments-components.puml), [rail matrix](ARCHITECTURE.md#rail-capability-matrix) |

The rail rows identify a shared acceptance template. They do not assert that every rail is an R1 feature or that the same settlement/cancellation rules apply to every rail. Each enabled rail needs its own supplement and evidence for applicable branches.

## Requirement-group navigation

| Group | Individual IDs | Primary scope | Model focus |
| --- | --- | --- | --- |
| FR-CH | FR-CH-01 through FR-CH-06 | Channels / gateway / experience | C4 L2, deployment, authorization |
| FR-IAM | FR-IAM-01 through FR-IAM-06 | Identity / factors / grants / entitlements | Authorization sequence, Controls L3 |
| FR-ACQ | FR-ACQ-01 through FR-ACQ-04 | Product discovery / application / attribution | Journey L3, application state |
| FR-ONB | FR-ONB-01 through FR-ONB-06 | Identity / screening / review / party creation | Journey/Controls L3, onboarding sequence |
| FR-ACC | FR-ACC-01 through FR-ACC-08 | Opening / inquiry / service / closure / delegation | Opening activity, account state, domain |
| FR-PAY | FR-PAY-01 through FR-PAY-10 | Money movement lifecycle / all rail capabilities | Payments L3/L4, dynamic, sequence, state, reconciliation |
| FR-CORE | FR-CORE-01 through FR-CORE-06 | Financial authority / products / journal | Core L3, domain, payment sequence |
| FR-CTRL | FR-CTRL-01 through FR-CTRL-07 | Checks / cases / evidence / reporting | Controls L3, onboarding/authorization, exception activity |
| FR-DATA | FR-DATA-01 through FR-DATA-05 | Service façades / projections / enterprise assets | C4 L2, event components, domain |
| FR-DOC | FR-DOC-01 through FR-DOC-04 | Terms / statements / retention / retrieval | Document sequence, domain, opening activity |
| FR-FIN | FR-FIN-01 through FR-FIN-05 | GL / reconciliation / reporting / liquidity | Reconciliation sequence, domain |
| FR-EXT | FR-EXT-01 through FR-EXT-05 | Providers / Open APIs / journey partners | Context, Payments L3, event components |
| FR-OPS | FR-OPS-01 through FR-OPS-05 | Workbench / correlation / replay / recovery / pauses | Deployment, exception activity, event components |

Business invariants BR-01 through BR-15 are realized across Core L3, payment L4, payment state/sequence and reconciliation. SEC-01 through SEC-10 are covered by authorization/Controls L3/deployment and AT-01/09/14/16/19/21/27/29. NFR-01 through NFR-14 have measurement definitions in the PRD and relevant load/recovery/operations scenarios AT-13/14/17/18/19/20/25/28.

## Validation boundary

The [implementation plan](IMPLEMENTATION_PLAN.md) and [work-package board](implementation/WORK_PACKAGES.md) extend this product traceability into 41 executable packages. Their machine-readable backlog assigns a primary implementation owner to all 77 functional requirements, 15 business rules, 10 security requirements, 14 quality objectives and eight outcome metrics. All assignments are planned; package status and actual evidence remain separate from intended coverage.

`make docs` checks that local links and anchors resolve, all 52 source IDs appear in this matrix, all 77 functional requirements have a direct acceptance reference, referenced IDs exist, every canonical diagram is cataloged with its exact PlantUML source, all diagram sources pass PlantUML syntax checking, and all expected SVGs render as XML. The [validation record](VALIDATION.md) distinguishes these documentation checks from future product acceptance. Semantic review is still required when requirements or architecture change.
