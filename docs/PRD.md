# Digital Bank — Product Requirements Document

| Field | Value |
| --- | --- |
| Document ID | DB-PRD-001 |
| Version / date | 1.0 / 2026-10-22 |
| Status | Proposed requirements baseline; stakeholder decisions pending |
| Product / working repository | Digital Bank / `piggybank` |
| Primary evidence | *Digital Bank Logical Architecture* image (kept outside this repository) |
| Product owner / engineering owner | TBD / TBD |
| Control owners | Compliance, Risk, Security, Finance, Operations; named accountable people TBD |
| Intended readers | Product, design, engineering, architecture, QA, operations, finance, risk, compliance, partner integration teams |

## Contents

1. [Evidence and interpretation](#1-evidence-and-interpretation)
2. [Product vision and problem](#2-product-vision-and-problem)
3. [Outcomes and success measures](#3-outcomes-and-success-measures)
4. [Users and stakeholders](#4-users-and-stakeholders)
5. [Scope and delivery boundaries](#5-scope-and-delivery-boundaries)
6. [User journeys](#6-user-journeys)
7. [Functional requirements](#7-functional-requirements)
8. [Business rules and financial integrity](#8-business-rules-and-financial-integrity)
9. [Data requirements](#9-data-requirements)
10. [Integration and API requirements](#10-integration-and-api-requirements)
11. [Security, privacy, and control requirements](#11-security-privacy-and-control-requirements)
12. [Quality attributes and service objectives](#12-quality-attributes-and-service-objectives)
13. [Experience and accessibility](#13-experience-and-accessibility)
14. [Operations and support](#14-operations-and-support)
15. [Analytics and reporting](#15-analytics-and-reporting)
16. [Acceptance and test strategy](#16-acceptance-and-test-strategy)
17. [Delivery plan and dependencies](#17-delivery-plan-and-dependencies)
18. [Risks and mitigations](#18-risks-and-mitigations)
19. [Assumptions and open decisions](#19-assumptions-and-open-decisions)
20. [Readiness and change control](#20-readiness-and-change-control)
21. [Architecture and UML index](#21-architecture-and-uml-index)
22. [Glossary and references](#22-glossary-and-references)

## 1. Evidence and interpretation

**Observed:** The image describes customer channels, a DMZ API gateway, Open APIs, Experience APIs, IAM, orchestration, five business process services, core banking, data services, enterprise data assets, payments execution, finance, documentation, controls, an event bus, journey analytics, and payment rails. Its arrows indicate main data flow rather than executable contracts. Its datastore icons indicate persistence but do not identify databases or system-of-record ownership.

**Proposed:** This PRD turns those capabilities into behavioral requirements and a staged delivery plan. Every “shall,” acceptance scenario, state machine, endpoint, event, and quantitative target below is proposed product detail. None is asserted to be present in an existing implementation. “Complete” means that the supplied capability landscape is covered, including operational and exception behavior; it does not mean that unresolved commercial or jurisdictional decisions have been made.

The source repeats **Journey Analytics & Orchestration** in the ecosystem layer. This specification models one logical integration capability with multiple provider adapters, rather than assuming three distinct products. **Tx Service** and **Transaction Service** both appear in Data Services: the proposed interpretation is a transaction command façade and a transaction inquiry projection, respectively. This distinction is provisional (DEC-08). The abbreviation **MM** is interpreted as money movement, not money market, pending confirmation (DEC-02).

No jurisdiction, bank license, deposit insurance scheme, legal entity, currency, vendor, cloud, implementation language, card processor, database, or production scale can be inferred from the image. Control names in the diagram establish capability scope; applicable legal obligations and retention periods require jurisdiction-specific decisions. [Architecture mapping](ARCHITECTURE.md#source-capability-inventory) preserves the source labels and identifies additions.

## 2. Product vision and problem

Provide a consistent digital banking experience in which customers can become verified customers, open and service accounts, move money, and obtain reliable records, while bank staff can manage exceptions, reconcile financial activity, and enforce access and risk controls.

The logical architecture suggests a need to coordinate fragmented channels, core records, external payment networks, and control systems. Without a common operating model, customers can receive conflicting account or payment status, staff can repeat identity checks, and finance can struggle to reconcile external settlement with internal postings. This specification addresses those problems through explicit ownership, durable workflows, observable status, and financial invariants. These are inferred product needs, rather than pain points validated through customer research.

The value proposition is continuity across channels and traceability across the financial lifecycle: one application identifier, one account identity, one payment operation identity, and an auditable chain from customer intent to final financial outcome.

## 3. Outcomes and success measures

All targets are **proposed launch planning targets**, with no measured baseline. Product and control owners must ratify denominators and measurement exclusions before R1. Report both the full population and any separately segmented technical metric so that exclusions do not hide customer failures.

| ID | Outcome and definition | Proposed target / observation window | Accountable role |
| --- | --- | --- | --- |
| KPI-01 | Onboarding completion: submitted applications reaching approved or declined decision without abandonment, divided by submitted applications; manual review shown separately | At least 90% within 24 hours; rolling 30 days | Product / onboarding operations |
| KPI-02 | Straight-through onboarding: approved applications needing no staff intervention, divided by all approved applications | At least 80%; rolling 30 days; segment by risk tier | Product / Compliance |
| KPI-03 | Account-opening reliability: eligible, approved opening requests activated without remediation, divided by eligible approved requests | At least 99%; rolling 30 days | Account product / Core team |
| KPI-04 | Payment technical success: valid, funded, approved instructions reaching the rail-defined successful outcome within its SLA, divided by such instructions | At least 99.5%; report provider and bank failures separately | Payments |
| KPI-05 | Financial integrity: duplicate economic executions or unbalanced committed journal batches caused by the bank | Zero; continuous monitoring and daily review | Finance / Core |
| KPI-06 | Reconciliation completion: external records matched or assigned an owned exception by the agreed business-day deadline | 100%; daily; no unexplained silent write-offs | Finance / Operations |
| KPI-07 | Customer service clarity: sampled journeys showing current state, next action, and correct balance semantics | 100% of critical journey acceptance cases | Product / QA |
| KPI-08 | Control accountability: privileged changes and risk decisions with attributable audit evidence | 100% of sampled and automated controls; daily | Security / Compliance |

Acquisition conversion and satisfaction targets require a research baseline. Track cost per acquired verified customer, campaign conversion, support contacts per 1,000 active customers, complaint resolution time, and payment completion by rail without inventing launch benchmarks.

## 4. Users and stakeholders

| Persona | Need / principal jobs | Permissions and constraints |
| --- | --- | --- |
| Prospect | Explore products, understand costs, begin and resume an application | Public content and their own application only |
| Retail customer | Access accounts, view balances and history, transfer money, manage profile, retrieve statements | Only owned or explicitly delegated accounts; step-up for sensitive actions |
| Assisted-channel banker | Help customers apply and service accounts in branch, private banker portal, or call center | Scoped customer relationship and action permissions; customer authorization recorded |
| Payments operator | Investigate failed or uncertain payments, approve permitted exceptions, manage rail incidents | Cannot invent settlement evidence or alter posted journals |
| Compliance analyst | Review identity, screening, fraud, and AML cases | Need-to-know case access; evidence and reason required; no self-approval |
| Finance analyst | Reconcile subledger, general ledger, settlement, fees, and liquidity | Read financial records; submit controlled adjustments through approval workflow |
| Product administrator | Configure product eligibility, pricing, terms, and limits | Versioned effective-dated changes; independent approval where policy requires |
| Security / entitlement administrator | Manage access policy and service principals | Segregated administration; no unrestricted financial execution privilege |
| Partner application | Access consented data or initiate permitted requests through Open APIs | Separate client identity, scopes, consent, expiry, quotas, and revocation |
| Auditor / reporting consumer | Verify decisions, financial records, and control evidence | Read-only, purpose-limited, time-bounded access; exports audited |

The initial customer segment is assumed to be adult individual retail customers (ASM-01). Business, joint, minor, and fiduciary account rules require separate requirements before enablement. Persona definitions do not establish legal eligibility.

## 5. Scope and delivery boundaries

The **full target** covers every capability in the source image. Releases are proposed sequencing decisions, not omissions from that target.

| Release | Product boundary | Exit evidence |
| --- | --- | --- |
| R0 — Foundation and controlled environments | Capability ownership, IAM, gateway, audit, durable workflows, core/ledger contracts, controls, observability, provider simulators, disaster-recovery design | Named owners, runnable contract suites, financial invariant checks, threat model, data classification, selected launch market and providers |
| R1 — Limited retail launch | Mobile and desktop banking; acquisition, onboarding, a basic deposit account, servicing, internal transfers, inbound/outbound external pilot rail, documents, controls, reconciliation, finance exports, basic reporting and support | All P0 R1 acceptance gates; certified pilot rail; staffed exceptions; signed product terms and controls; recovery exercise |
| R2 — Channel and ecosystem expansion | Branch/private banker/ATM/call center adapters, delegated partner Open APIs, additional rails, richer audience/insight services and enterprise analytics | Channel/provider certification, delegated authorization tests, rail-specific settlement and dispute rules |
| R3 — Optimization | Advanced journey orchestration, personalization and routing, broader finance/liquidity automation, scale tuning | Measured improvement with unchanged financial and control invariants |

R1 includes a basic staff operations workbench to run required manual reviews and exceptions. This workbench is a proposed enabling tool; it is distinct from the source's customer-facing branch and private banker channels scheduled for R2.

**Rail scope:** ACH, checks, debit, bill pay, wire, lockbox, SWIFT, and P2P are all source capabilities. R1 selects one external rail only after DEC-04. Each remaining rail is conditional R2 scope with a provider-specific functional supplement. SWIFT messaging is not assumed to establish final settlement. “Debit” does not by itself establish a card issuing program, and P2P does not imply a particular scheme. The full rail matrix is in [Architecture](ARCHITECTURE.md#rail-capability-matrix).

**Outside the defined baseline:** lending/credit underwriting, trading/investments, crypto, insurance, multi-currency conversion, full card issuing/acquiring, cash logistics, international tax advice, and enterprise treasury products. The private banker channel is preserved without assuming investment products. New product families require their own business rules and acceptance gates.

Priorities apply within the named release: **P0** is a release blocker, **P1** is important but can move with explicit scope control, and **P2** is an optimization. A P0 requirement in R2 is not an R1 launch dependency. A moved requirement cannot weaken a financial or access invariant required by an enabled feature.

## 6. User journeys

### J-01 — Discover and apply

**Actor / trigger:** Prospect selects a product in mobile or desktop banking. **Preconditions:** Published product/terms version and an enabled jurisdiction. **Flow:** View eligibility, fees, disclosures and support options; record permitted acquisition attribution; create an application; collect contact information and applicable consent; resume using a secure link or authenticated session. **Outcome:** A stable application ID and saved progress. **Exceptions:** Ineligible product or geography gives a clear explanation without collecting unnecessary sensitive data; duplicate contact information does not automatically merge identities; marketing consent withdrawal does not erase required application evidence. **Measures:** Funnel abandonment, consent capture, resume success.

### J-02 — Verify and onboard

**Actor / trigger:** Applicant submits required data and identity evidence. **Preconditions:** Application consent and required fields complete. **Flow:** Validate data; request identity verification; screen applicable parties; evaluate risk; approve, decline, request information, or create manual review; retain attributable decisions and policy versions. **Outcome:** An approved customer record or a clear permitted next step. **Exceptions:** Provider timeout remains pending; duplicate callbacks do not create duplicate customers; suspected duplicates require review; a screening hit blocks progression until authorized resolution; expired evidence requires reassessment. **Models:** UML onboarding sequence and application state machine.

### J-03 — Open an account

**Actor / trigger:** Approved customer accepts a product offer. **Preconditions:** Current eligibility, identity/control clearance, and accepted terms version. **Flow:** Confirm product/rate/fee snapshot; authorize request; create an account once; configure ownership, limits and currency; register documents; activate after all gates; notify customer. **Outcome:** Active account with a stable account ID and zero opening balance unless funded by a separately recorded movement. **Exceptions:** Repeated request returns the same result; terms changes require renewed acceptance; a partially created core record remains restricted pending recovery; no spendable account appears before activation. **Model:** UML account-opening activity.

### J-04 — View and service accounts

**Actor / trigger:** Customer signs in or asks for support. **Flow:** Show available, ledger and held balances with as-of time; page transaction history; view details and documents; update permitted contact/preferences with verification; request freezes or closure; create a support case. **Outcome:** Accurate authorized account information and auditable changes. **Exceptions:** A stale projection is labeled; debit decisions consult the authoritative core; frozen accounts reject restricted actions; closure waits for zero balance, no holds, no outstanding obligations, and applicable retention handling. **Models:** Account state and authorization sequence.

### J-05 — Move money

**Actor / trigger:** Customer or delegated actor prepares a payment. **Preconditions:** Active permitted account, authorized actor, beneficiary validation, sufficient availability, enabled rail and limits. **Flow:** Display amount/currency, beneficiary, fee, dates and cancellation limits; authenticate and collect approval as needed; run controls; reserve funds atomically in core; execute internal transfer or persist external submission intent; expose progress; process authenticated acknowledgements; settle using evidence; release or consume holds through core; notify and reconcile. **Outcome:** One economic instruction and traceable financial entries. **Exceptions:** Changed payload with the same idempotency key is rejected; unknown provider outcome remains under reconciliation; rail acknowledgement alone is not final settlement; returns and reversals are linked new movements. **Models:** C4 payment dynamic, UML payment sequence/state, and reconciliation sequence.

### J-06 — Investigate and resolve an exception

**Actor / trigger:** Operator receives a payment, identity, fraud, settlement or customer dispute case. **Flow:** See evidence and aging; obtain permitted independent approval; request information or execute a bounded remediation; record reason, policy and before/after state; notify permitted parties; verify financial effect and close case. **Outcome:** Owned, auditable resolution with traceable postings where required. **Exceptions:** The case creator cannot approve their own restricted action; adjustments create new entries; no staff action treats an unknown external payment as definitively failed without evidence. **Model:** UML exception activity.

### J-07 — Close the business day and issue records

**Actor / trigger:** Finance cut-off and statement schedule. **Flow:** Ingest settlement evidence; match rail/core records; create exceptions for differences; reconcile journal and GL control totals; approve exports and reports; generate statements from an immutable period snapshot; publish authorized downloads; retain evidence. **Outcome:** Reproducible customer and financial records with exception ownership. **Exceptions:** Late files are versioned; reruns do not duplicate GL entries or statements; corrections publish a superseding version; missing provider evidence is visible in completeness reporting. **Models:** UML reconciliation and document sequence.

### J-08 — Use a consented partner

**Actor / trigger:** Customer authorizes a registered third party. **Flow:** Show requested scopes/accounts/purpose/expiry; collect consent; issue bounded authorization; enforce authorization and consent for every action; expose revocation and usage history. **Outcome:** Partner sees or initiates only permitted activity. **Exceptions:** Revoked/expired consent blocks subsequent calls; initiated irrevocable rail activity follows payment lifecycle rather than being silently undone; partner tokens cannot impersonate bank staff. R2 delivery.

## 7. Functional requirements

Each row is a verifiable proposed requirement. “Acceptance” gives the minimum observable proof; [section 16](#16-acceptance-and-test-strategy) defines integrated cases. Owners are accountable roles, with names TBD. Diagram and source coverage is recorded in [Traceability](TRACEABILITY.md).

### 7.1 Channels and experience — owner: Channel product

| ID | Requirement | Priority / release | Minimum acceptance |
| --- | --- | --- | --- |
| FR-CH-01 | Mobile and desktop channels shall support acquisition, application status, account opening, inquiry, payments and documents against shared product rules. | P0 / R1 | The same customer sees matching account and payment identities on both channels; server-side permission checks apply to both. |
| FR-CH-02 | Experience APIs shall tailor payloads and aggregate authorized read data for each channel without becoming financial systems of record. | P0 / R1 | A failed aggregation causes no posting or duplicate command; balance freshness is returned. |
| FR-CH-03 | Branch and private banker portal adapters shall support assisted journeys with explicit actor and customer identities. | P0 / R2 | Audit distinguishes staff actor, represented customer and recorded authorization. |
| FR-CH-04 | Call center and ATM integrations shall apply the same account restrictions and product rules with channel-specific authentication and action limits. | P0 / R2 | Restricted transactions fail consistently; ATM/provider certification proves enabled operations. |
| FR-CH-05 | API Gateway shall enforce authentication routing, request size limits, quotas, correlation IDs, and version policy at the edge. | P0 / R1 | Unauthorized, over-quota, oversized and unsupported-version calls are rejected without domain side effects. |
| FR-CH-06 | Customer journeys shall support safe resume, localized display and explicit pending/error states across session interruptions. | P1 / R1 | After interruption, the customer sees the persisted application or operation rather than creating a replacement. |

### 7.2 IAM and enterprise entitlements — owner: Security

| ID | Requirement | Priority / release | Minimum acceptance |
| --- | --- | --- | --- |
| FR-IAM-01 | IAM shall manage customer, staff and service identities, authenticated sessions, factor enrollment, recovery and revocation with distinct trust contexts. | P0 / R1 | A disabled identity loses access within the approved revocation objective; customer tokens cannot access staff functions. |
| FR-IAM-02 | High-risk actions shall require step-up bound to the intended action and key transaction details. | P0 / R1 | A proof for one beneficiary or amount cannot authorize a changed payment. |
| FR-IAM-03 | Domain services shall enforce account relationship, action entitlement, consent and policy conditions independently of UI visibility. | P0 / R1 | Substituting another customer's account ID or staff-only action returns denial. |
| FR-IAM-04 | Privileged role grants, product changes and restricted financial adjustments shall require independently attributable approval and expiry where applicable. | P0 / R1 | A maker cannot approve their own controlled action, including through another channel. |
| FR-IAM-05 | Partner access shall use registered clients, scoped credentials, customer grants, quotas, expiry, revocation and authenticated callbacks. | P0 / R2 | Expired, revoked and over-scoped requests fail without leaking resource existence. |
| FR-IAM-06 | Account recovery and device/factor changes shall be risk evaluated, audited, and communicated using the approved recovery policy. | P0 / R1 | A lost-device drill restores access without bypassing ownership verification; suspicious recovery is reviewable. |

### 7.3 Customer acquisition — owner: Growth / Product

| ID | Requirement | Priority / release | Minimum acceptance |
| --- | --- | --- | --- |
| FR-ACQ-01 | Publish versioned product information, eligibility, prices and required disclosures before application submission. | P0 / R1 | Displayed offer version matches the application snapshot. |
| FR-ACQ-02 | Capture acquisition attribution and contact preferences only for permitted purposes and with required consent. | P1 / R1 | Consent withdrawal suppresses optional marketing use without deleting mandatory financial evidence. |
| FR-ACQ-03 | Create and resume a stable application without unsafe automatic identity merging. | P0 / R1 | Duplicate submission/retry returns the existing application; matching email alone never merges parties. |
| FR-ACQ-04 | Audience and insight services shall support permissioned segmentation and measurable campaign outcomes. | P1 / R2 | Segment membership respects purpose restrictions and can be traced to the source/version. |

### 7.4 New customer onboarding — owner: Onboarding / Compliance

| ID | Requirement | Priority / release | Minimum acceptance |
| --- | --- | --- | --- |
| FR-ONB-01 | Capture required party data, jurisdiction-dependent declarations, evidence and versioned consent. | P0 / R1 | Incomplete required evidence blocks submission; sensitive evidence is excluded from ordinary logs. |
| FR-ONB-02 | Integrate identity verification with durable request IDs, signed/authenticated results, evidence expiry and safe retries. | P0 / R1 | Duplicate or delayed callbacks do not create duplicate decisions or customers. |
| FR-ONB-03 | Perform required screening and risk checks, routing unresolved matches to a restricted review queue. | P0 / R1 | No unresolved blocking case reaches approved status. |
| FR-ONB-04 | Support request-information, manual review, approve and decline outcomes with reasons, timestamps and policy versions. | P0 / R1 | Every decision has attributable evidence; customer explanations follow disclosure policy. |
| FR-ONB-05 | Create one authoritative party identity after approval, detecting suspected duplicates for controlled resolution. | P0 / R1 | Concurrent approval callbacks yield one linked customer identity per application. |
| FR-ONB-06 | Rescreen and reverify when evidence expires or relevant customer/policy data changes. | P0 / R1 | An expired decision is not reused for a new account-opening approval. |

### 7.5 Account opening and servicing — owner: Accounts

| ID | Requirement | Priority / release | Minimum acceptance |
| --- | --- | --- | --- |
| FR-ACC-01 | Opening shall validate current customer eligibility, controls, product availability and accepted terms before activation. | P0 / R1 | Any unmet gate leaves the account absent or restricted, with a recoverable operation. |
| FR-ACC-02 | Opening shall be idempotent and preserve account, customer, product and consent references. | P0 / R1 | A retry with the same request returns the same account; a changed request with the same key conflicts. |
| FR-ACC-03 | Inquiry shall show available balance, ledger balance, holds, currency and authoritative as-of timestamp. | P0 / R1 | A test hold reduces availability without misrepresenting posted ledger balance. |
| FR-ACC-04 | Transaction inquiry shall support pagination, dates, amount/currency, fees, status, linked reversals and stable movement IDs. | P0 / R1 | Paging a stable snapshot neither omits nor duplicates entries; reversals remain visible. |
| FR-ACC-05 | Verified profile changes shall propagate to designated masters and trigger required control reassessment. | P0 / R1 | A contact change is audited, verified as required, and does not overwrite legal identity without review. |
| FR-ACC-06 | Account restriction and closure shall apply explicit debit/credit rules, outstanding-obligation checks and customer communication. | P0 / R1 | Frozen or closing accounts reject prohibited actions; closure cannot discard residual funds or holds. |
| FR-ACC-07 | Service requests, complaints and disputes shall create trackable cases with ownership, deadlines and permitted remediation. | P0 / R1 | Customer obtains a case reference; aging and escalation are visible to authorized staff. |
| FR-ACC-08 | Authorized delegates and multi-party account relationships shall support bounded authority and revocation. | P1 / R2 | Removing delegation blocks new actions while preserving historical actor evidence. |

### 7.6 Payments and money movement — owner: Payments

| ID | Requirement | Priority / release | Minimum acceptance |
| --- | --- | --- | --- |
| FR-PAY-01 | Capture payer/payee, integer minor-unit amount, currency, requested date, purpose, rail choice and disclosed fees before confirmation. | P0 / R1 | Invalid amounts/currencies/beneficiaries fail before any financial action. |
| FR-PAY-02 | Use a stable operation ID, scoped idempotency key and request fingerprint for every financial command. | P0 / R1 | Repeated equivalent requests produce one economic instruction; altered payloads conflict. |
| FR-PAY-03 | Validate ownership, entitlement, step-up, account restrictions, limits, screening and fraud decisions before execution. | P0 / R1 | Each denied or review-required instruction has no unauthorized provider submission. |
| FR-PAY-04 | Internal transfers shall atomically debit and credit accounts in the same supported ledger/currency after all controls pass. | P0 / R1 | A fault cannot commit only one side; duplicate retries return the original posting result. |
| FR-PAY-05 | External outgoing payments shall reserve availability atomically, persist submission intent and map provider lifecycle evidence. | P0 / R1 | Two competing debits cannot spend the same funds; accepted transport responses do not imply settlement. |
| FR-PAY-06 | Uncertain submission or settlement shall remain traceable under reconciliation without unsafe resubmission or automatic hold release. | P0 / R1 | Timeout-after-acceptance recovers the same provider instruction or creates an owned exception. |
| FR-PAY-07 | Definitive failure/cancellation shall release holds once; returns/reversals shall create linked corrective movements under rail policy. | P0 / R1 | Correction preserves original postings and never duplicates a refund or releases a consumed hold. |
| FR-PAY-08 | Inbound payments shall authenticate/deduplicate evidence, validate destination and controls, and credit or restrict funds using the enabled rail's finality policy. | P0 / R1 | Replayed evidence cannot duplicate credit; unavailable destination/blocked funds create a controlled exception. |
| FR-PAY-09 | Customers and staff shall see canonical status, fees, dates, permitted cancellation actions and rail-specific finality expectations. | P0 / R1 | Pending, failed, settled and returned states are distinguishable; staff detail includes provider evidence. |
| FR-PAY-10 | Additional rails and advanced approval/routing policies shall be enabled only through certified adapters and effective-dated configuration. | P0 / R2 | Every enabled rail passes its supplement for cut-offs, acknowledgements, finality, returns and reconciliation. |

### 7.7 Core platform — owner: Core banking

| ID | Requirement | Priority / release | Minimum acceptance |
| --- | --- | --- | --- |
| FR-CORE-01 | Core shall own authoritative accounts/parties, account restrictions, holds, journal/subledger and core workflow outcomes. | P0 / R1 | No channel, data mart or analytics job directly changes authoritative balances. |
| FR-CORE-02 | Product management shall version eligibility, pricing, fees, limits and terms with effective periods and controlled activation. | P0 / R1 | Account/payment records resolve the applied configuration; unauthorized retrospective edits fail. |
| FR-CORE-03 | Every committed posting batch shall balance by currency and use exact arithmetic and unique business references. | P0 / R1 | Unbalanced, overflow, unsupported-scale and duplicate batches fail atomically. |
| FR-CORE-04 | Holds, posting and release shall be serialized against authoritative availability and account restrictions. | P0 / R1 | Concurrent requests cannot exceed available funds under the configured no-overdraft policy. |
| FR-CORE-05 | Committed changes and outgoing events shall be durably linked through an outbox or equivalent atomic mechanism. | P0 / R1 | Crash after commit still emits the event; consumers tolerate redelivery. |
| FR-CORE-06 | Corrections, accruals and fees shall use versioned rules and new traceable entries; source journals shall remain immutable. | P0 / R1 | A correction can be traced to original batch, reason and independent approval. |

### 7.8 Controls — owner: Risk / Compliance

| ID | Requirement | Priority / release | Minimum acceptance |
| --- | --- | --- | --- |
| FR-CTRL-01 | Identity verification, sanctions screening, risk/compliance and fraud/AML decisions shall be distinct, attributable checks with policy and evidence versions. | P0 / R1 | Passing identity verification cannot substitute for unresolved screening clearance. |
| FR-CTRL-02 | Timeout/unavailability of mandatory checks shall hold affected actions in a safe pending state with an owned exception. | P0 / R1 | Control outage never silently approves an onboarding or payment action. |
| FR-CTRL-03 | Cases shall support evidence, assignment, severity, aging, escalation, independent approval and final disposition. | P0 / R1 | Every blocking case has an owner and next action; prohibited overrides are denied. |
| FR-CTRL-04 | Transaction monitoring shall detect configured patterns from durable events and permit retrospective investigation. | P0 / R1 | Replay detects the seeded scenario once at the economic-operation level. |
| FR-CTRL-05 | Operations reporting shall expose payment/control backlog, reconciliation breaks, failed integrations and overdue cases. | P0 / R1 | Reports reconcile to underlying operation/case counts for a fixed snapshot. |
| FR-CTRL-06 | Enterprise entitlements shall govern staff/customer/delegate actions with least privilege and separation of duties. | P0 / R1 | A role grant alone does not bypass account relationship or maker/checker rules. |
| FR-CTRL-07 | Control decisions and permitted disclosures shall be reproducible for audit with restricted evidence access. | P0 / R1 | An auditor can reconstruct why an action was allowed or denied without changing the record. |

### 7.9 Data and enterprise assets — owner: Data platform

| ID | Requirement | Priority / release | Minimum acceptance |
| --- | --- | --- | --- |
| FR-DATA-01 | Customer, account, transaction command and transaction inquiry services shall expose scoped, versioned contracts and explicit freshness. | P0 / R1 | Read models identify their source/as-of version; commands reach the authoritative owner. |
| FR-DATA-02 | Mastered data shall maintain governed canonical identifiers, provenance and controlled matching/merge history. | P0 / R1 | A merge is attributable and reversible at the reference level without rewriting financial evidence. |
| FR-DATA-03 | Warehouse and big data ingestion shall use versioned schemas, lineage, purpose restrictions and reproducible transforms. | P1 / R2 | An aggregate can be traced to input schema/version and authorized dataset. |
| FR-DATA-04 | Audience/insight and journey analytics shall minimize personal data, respect consent and distinguish operational from marketing use. | P1 / R2 | A user without marketing permission is excluded from optional targeting and exports. |
| FR-DATA-05 | Data quality checks shall detect missing/duplicate financial events, broken party/account references and stale projections. | P0 / R1 | Seeded defects produce owned alerts; repair does not write balances from analytics. |

### 7.10 Documentation — owner: Customer records

| ID | Requirement | Priority / release | Minimum acceptance |
| --- | --- | --- | --- |
| FR-DOC-01 | Preserve published terms versions and acceptance evidence linked to customer, product, account, channel and time. | P0 / R1 | The exact accepted document and its integrity digest are retrievable by an authorized reviewer. |
| FR-DOC-02 | Generate statements from consistent, versioned period snapshots with opening/closing balances, postings, fees and corrections. | P0 / R1 | Closing balance equals opening balance plus signed period movements; rerun preserves version history. |
| FR-DOC-03 | Deliver authorized documents with short-lived retrieval access, audit and channel notifications that avoid sensitive content. | P0 / R1 | Cross-customer access is denied; an expired download reference cannot be reused. |
| FR-DOC-04 | Apply document retention, legal holds, export and permitted disposal under an approved policy. | P0 / R1 | Disposal is blocked by an active legal hold and records evidence of permitted completion. |

### 7.11 Finance — owner: Finance

| ID | Requirement | Priority / release | Minimum acceptance |
| --- | --- | --- | --- |
| FR-FIN-01 | Map subledger batches to versioned general-ledger accounts and export/import with unique references and balanced controls. | P0 / R1 | Reprocessing the same batch cannot double-book the GL. |
| FR-FIN-02 | Reconcile core journal, payment instructions, provider settlement and GL by currency, account and business date. | P0 / R1 | Missing, duplicate, fee and timing differences produce explicit classifications and owned cases. |
| FR-FIN-03 | Provide liquidity positions that distinguish available, reserved, expected and externally settled funds with as-of times. | P0 / R1 | Stale or missing provider evidence is visible and cannot be presented as confirmed liquidity. |
| FR-FIN-04 | Financial reporting shall be reproducible for a reporting snapshot with source lineage and controlled corrections. | P0 / R1 | A corrected report preserves the original and explains the adjustment. |
| FR-FIN-05 | Advanced liquidity forecasting and automated finance workflows shall preserve approved limits and independent controls. | P1 / R3 | A simulated breach stops restricted actions and creates the configured escalation. |

### 7.12 Ecosystem integration — owner: Integration

| ID | Requirement | Priority / release | Minimum acceptance |
| --- | --- | --- | --- |
| FR-EXT-01 | External adapters shall isolate provider credentials, formats, retries, status mapping, callback verification and certification. | P0 / R1 | Provider faults do not leak credentials or create additional business operations. |
| FR-EXT-02 | Open APIs shall expose documented, scoped customer-consented contracts and support safe version transitions. | P0 / R2 | Contract tests verify client identity, consent, quotas and sunset behavior. |
| FR-EXT-03 | Payment adapters shall support provider inquiry and settlement evidence, or declare an operational/manual fallback before launch. | P0 / R1 | Unknown outcomes have a practiced recovery path without blind financial retries. |
| FR-EXT-04 | Journey orchestration integrations shall accept minimized approved signals and return advisory next actions under channel policy. | P1 / R2 | A partner recommendation cannot directly approve identity, post money or override controls. |
| FR-EXT-05 | Provider outages, credential rotation and callback replay shall be handled with bounded retries, queues and escalation. | P0 / R1 | A rotation/outage drill preserves operation identity and recovers without duplicate effects. |

### 7.13 Operations — owner: Operations / SRE

| ID | Requirement | Priority / release | Minimum acceptance |
| --- | --- | --- | --- |
| FR-OPS-01 | Provide a restricted staff workbench for R1 approvals, exception cases, payment inquiry and support. | P0 / R1 | Authorized staff complete a seeded manual review; unsupported financial overrides are unavailable. |
| FR-OPS-02 | Correlate customer requests, workflows, postings, events and provider references using durable identifiers. | P0 / R1 | Support reconstructs an end-to-end operation without requiring unmasked identity evidence. |
| FR-OPS-03 | Support controlled event replay and dead-letter recovery with deduplication and audit. | P0 / R1 | Replay restores a failed projection without resubmitting the external payment. |
| FR-OPS-04 | Support backup restoration, failover and reconciliation before financial writes resume. | P0 / R1 | Recovery drill demonstrates no unfenced concurrent writers and explains all uncertain operations. |
| FR-OPS-05 | Provide effective-dated rail enablement, emergency pause and rollback of compatible configuration. | P0 / R1 | Pausing new submissions preserves processing of existing acknowledgements, returns and reconciliation. |

## 8. Business rules and financial integrity

| ID | Rule | Observable invariant |
| --- | --- | --- |
| BR-01 | Maintain exact monetary values using integer minor units plus currency and supported scale; no floating-point money. | Arithmetic, overflow, rounding and currency compatibility are validated before commit. |
| BR-02 | Every posted journal batch balances within one currency; cross-currency exchange is outside the baseline. | Sum of debits equals sum of credits for each committed batch/currency. |
| BR-03 | Core is authoritative for posted balances, holds and spend decisions. Under the R1 no-overdraft assumption, available balance = ledger balance minus active debit holds minus other configured restrictions. | A reserve/post transaction atomically checks account state and availability; projections cannot authorize spending. |
| BR-04 | One command identity produces at most one economic instruction and one posting per posting kind. | Uniqueness on operation/posting-kind and provider instruction prevents duplicate effects despite at-least-once delivery. |
| BR-05 | Idempotency is scoped to authenticated actor/client, operation type and key, with a canonical request fingerprint. | Same key and same request returns the persisted result; same key and different request conflicts. Business-operation deduplication survives API cache expiry. |
| BR-06 | Mandatory controls and independent approvals precede execution; changed material details invalidate prior approval. | A review-required or denied instruction is never externally submitted. |
| BR-07 | An acknowledgement, HTTP success or delivered event is not proof of rail settlement. | Finality requires authenticated provider/network evidence interpreted by the certified rail policy. |
| BR-08 | An ambiguous external outcome retains its instruction identity and safe financial restriction until inquiry, evidence or controlled resolution. | Timeout cannot trigger a replacement payment or release money that may already have moved. |
| BR-09 | Cancellation is allowed only in states and provider windows where non-execution is proven; cancellation after submission is a request until confirmed. | “Cancellation requested” never implies funds are released or payment failed. |
| BR-10 | Returns and reversals create linked corrective movements and new balanced entries; historical entries remain immutable. | Original and correction are both visible, with reason, amount, currency, actor and provider evidence. |
| BR-11 | Product, consent, fee, limit, screening and entitlement decisions reference effective policy versions and business timestamps. | An auditor can reconstruct the rules applied at action time. |
| BR-12 | Application approval, account activation, payment settlement and GL export are separate milestones. | No upstream state is treated as proof that a downstream milestone occurred. |
| BR-13 | Customer identity is not inferred from matching email, phone or name alone. | Suspected duplicates are reviewed; merges preserve provenance and cannot rewrite journal ownership history. |
| BR-14 | Maker/checker separation uses durable person identity across sessions and channels. | One human cannot satisfy both required roles by changing channel or credential. |
| BR-15 | All business deadlines specify rail calendar, cut-off, timezone and business date; internal timestamps use UTC. | The customer sees an explicit expected date; daylight-saving and holiday cases use the approved calendar. |

**Financial example (proposed accounting policy):** A customer has ledger balance 10,000 minor units and no holds. Reserving an external outgoing payment of 2,500 leaves ledger balance 10,000 and available balance 7,500. Definitive pre-settlement failure releases the hold and restores availability. Confirmed settlement atomically consumes the hold and posts the customer liability reduction against the configured settlement/clearing account, leaving ledger and available balance 7,500 before separately disclosed fees. A later return creates a linked incoming correction. The chart of accounts, fee booking, treatment of inbound provisional funds, interest/accrual policy, tax and business-date rules require Finance decisions; this example is not a jurisdiction-specific accounting mandate.

For an internal transfer in one core/currency, debit source liability and credit destination liability in one journal transaction. External settlement cannot be included in a local database transaction; the workflow uses durable intent and reconciliation. No distributed “exactly once” guarantee is assumed.

## 9. Data requirements

### 9.1 Core entities and ownership

| Entity / aggregate | Minimum fields and relationships | Authoritative owner / classification |
| --- | --- | --- |
| Party / Customer | Stable party ID, party type, verified attributes, evidence references, jurisdiction, risk status; separate IAM subject link | Core party master / restricted identity |
| Application | Application ID, version, product, applicant reference, state, required fields, decision/evidence refs, timestamps | Onboarding / restricted |
| IdentityVerification / ScreeningDecision | Provider request/result IDs, policy/evidence version, outcome, validity, case reference | Controls / highly restricted |
| ProductVersion | Product ID/version, currency, effective period, eligibility, pricing, limits, terms refs | Core product management / confidential |
| Account / AccountRelationship | Account ID, product version, currency, status, party role, restrictions, lifecycle dates | Core / restricted financial |
| Consent / TermsAcceptance | Subject, purpose/scopes/accounts, text/document version, digest, channel, actor, time, expiry/revocation | Documentation for acceptance evidence; IAM for enforceable access grants / restricted |
| Beneficiary | Beneficiary ID, owner, routing details/token, verification and screening state | Payments / restricted financial |
| PaymentInstruction | Operation ID, actor, payer/payee refs, amount/currency, fee quote, rail, state/version, idempotency fingerprint, policy refs | Payments / restricted financial |
| Hold | Hold ID, operation ID, account, amount/currency, status, reason, timestamps | Core / restricted financial |
| JournalBatch / JournalEntry | Unique business reference/posting kind, currency, balanced entries, account refs, business date, correction link | Core journal / restricted immutable financial |
| SettlementRecord / ReconciliationCase | Provider/record IDs, instruction refs, amounts, currency, dates, evidence hash, match status, owner, resolution | Payments/Finance / restricted financial |
| GeneralLedgerExport | Export/batch IDs, mapping version, debit/credit totals, status, acknowledgements | Finance / restricted financial |
| Document / Statement | Document ID/type, period, snapshot/version, digest, storage ref, supersedes ref, retention class | Documentation / restricted customer records |
| AuditEvent / ControlCase | Actor, represented subject, action/resource, decision, policy, reason, correlation, timestamps | Controls / security restricted |
| EventEnvelope / ConsumerReceipt | Event ID/type/schema version, producer, aggregate ID/version, correlation/causation, occurrence time, receipt/dedup status | Producer outbox / consumer inbox / classification inherited from payload |
| AnalyticalProjection | Source refs, schema and transform versions, as-of watermark, purpose/consent tags | Data platform / derived classification |

The class diagram defines proposed associations and invariants. It is a design-level model, not an existing database schema or generated code view. Datastore icons in the source do not require one physical database per icon.

### 9.2 Lifecycle, quality and privacy

Capture only required identity attributes and evidence, validate at ingestion, and record source/provenance. Keep provider raw evidence in a restricted evidence store with narrower access than customer or support profiles. Store secrets in a separate secrets manager rather than customer tables or documents.

Every data class shall have a retention owner, lawful processing purpose, retention trigger/period, legal-hold rule, backup treatment and permitted disposal method before launch (DEC-06). Periods remain TBD because the source gives no market or legal regime. Distinguish deletion of optional marketing data from retention of required financial/audit records. Data-subject requests use a controlled search/export/correction/restriction workflow with evidence and identity verification.

Financial and audit records are append-only within their required retention period, access controlled and integrity monitored. Immutability does not imply keeping every data class forever. Correct identity attributes through versioned changes; correct postings through new entries. Analytical datasets inherit purpose and classification restrictions. Data quality monitoring shall track projection lag, event gaps, orphan references, duplicates, missing settlement files and conflicting master records.

### 9.3 Time and consistency

Use UTC timestamps for transport and storage, separate business date and rail calendar, and display customer-local times with timezone labels where relevant. Distinguish occurrence time, receipt time, posting date and settlement date. Use monotonically increasing aggregate versions for state changes and reject stale transitions. Cross-domain analytics is eventually consistent; financial authorizations use authoritative synchronous core decisions. A balance response must identify whether it is authoritative or projected and expose its as-of watermark.

## 10. Integration and API requirements

Detailed proposed endpoints, event envelopes and sequence semantics appear in [Architecture](ARCHITECTURE.md#proposed-api-surface). No endpoint in this package is observed in source code.

- Customer and staff calls shall enter through the gateway and appropriate Experience API. Partner calls shall use Open APIs with client identity and consent checks. Sensitive domain authorization is repeated at the authoritative service boundary.
- Mutating commands shall return a durable operation/application/account reference. Long-running requests return an accepted/pending status and status resource; acceptance does not imply financial completion. A synchronous internal transfer may return its committed result.
- Requests shall use versioned schemas, correlation IDs, bounded pagination, structured field errors and safe error messages. Ownership failures shall not leak other customers' resource existence.
- Financial commands shall carry idempotency keys, actor/delegation context and verified approval context. Authorization shall be reevaluated even when replaying a cached response.
- External callbacks shall authenticate source and payload, enforce replay checks and semantic deduplication, persist original evidence, acknowledge only after durable receipt, and process state changes asynchronously where appropriate.
- Internal events shall have producer-owned schemas and an envelope with event, aggregate, correlation and causation identities. Use durable publication, consumer deduplication, bounded retries, dead-letter handling and replay controls.
- Service and provider failures shall be classified as validation, denied, review-required, transient, definitive failure or unknown outcome. Only safe/idempotent operations may be automatically retried.
- Each provider contract shall define business hours, cut-offs, status transitions, finality, inquiry, callback verification, duplicate behavior, fee reporting, reconciliation files, failure escalation and exit/data portability.

## 11. Security, privacy, and control requirements

These are proposed engineering and operating requirements. The words KYC, AML and sanctions identify control capabilities; they do not establish that any particular law or standard applies or that the product is compliant. The applicable control matrix must be settled under DEC-01 and DEC-06.

| ID | Requirement | Verification |
| --- | --- | --- |
| SEC-01 | Authenticate humans and workloads with separate trust contexts; enforce least privilege and deny by default at each domain boundary. | Customer/staff/service/partner negative authorization and delegation cases. |
| SEC-02 | Encrypt sensitive data in transit and at rest using organization-approved cryptography; centralize key ownership, rotation and access evidence. | Configuration review and key/certificate rotation drill; no secret values in logs or artifacts. |
| SEC-03 | Use phishing-resistant factors for privileged staff and risk-appropriate customer authentication/recovery; bind sensitive approvals to action details. | Factor enrollment, recovery abuse and changed-payment tests. |
| SEC-04 | Separate ingress, application, financial/control data, analytics and provider egress trust zones; narrowly allow communication. | Network policy review and denied unauthorized lateral access. |
| SEC-05 | Redact logs/telemetry and restrict identity evidence, screening matches and customer documents by purpose. | Automated seeded-sensitive-data inspection and access review. |
| SEC-06 | Record tamper-evident, attributable audit for access decisions, financial changes, control decisions, configuration, exports and replay. | Evidence reconstruction for critical scenarios; denied destructive modification. |
| SEC-07 | Validate input, encode output, protect browser sessions and state-changing requests, manage dependencies and test abuse paths. | Threat model, security checks and resolved release-blocking findings. |
| SEC-08 | Enforce webhook authenticity, replay defenses, rate/resource limits and partner credential revocation. | Forged, duplicated, stale and over-quota calls fail safely. |
| SEC-09 | Protect nonproduction environments using synthetic or irreversibly de-identified data; prohibit production credentials in test fixtures. | Environment/data access review. |
| SEC-10 | Maintain incident response, access recertification, vendor security review and evidence retention aligned with approved policy. | Tabletop incident exercise and documented closure of release-blocking gaps. |

Fail safely when mandatory authorization/control dependencies are unavailable. This can leave an operation pending rather than declining it. Availability measures must expose such degradation rather than hiding it as a successful business action.

## 12. Quality attributes and service objectives

The following numbers are **proposed design and acceptance targets**, not measurements or facts from the image. Ratify capacity, service boundaries, workload mix and budgets under DEC-07 before R1. No customer funds path may relax integrity to meet a latency target.

| ID | Attribute / measurement boundary | Proposed target | Acceptance evidence |
| --- | --- | --- | --- |
| NFR-01 | Availability of bank-controlled customer command/inquiry API: successful eligible responses divided by eligible requests; separately publish end-to-end business availability including providers | 99.95% monthly; planned maintenance counts | Synthetic probes and request metrics; expected validation denials classified separately, dependency timeouts counted as degradation |
| NFR-02 | Account/transaction inquiry latency at gateway, excluding Internet transit | p95 ≤ 500 ms, p99 ≤ 1,500 ms | Representative load with authorization and realistic history sizes |
| NFR-03 | Durable acceptance of account/payment commands at gateway; provider completion separately measured | p95 ≤ 1,000 ms, p99 ≤ 2,500 ms | Load plus storage latency and failure injection; accepted operation is queryable after restart |
| NFR-04 | Synchronous internal transfer completion after controls, including authoritative commit | p95 ≤ 2 s at approved launch load | Balanced posting and duplicate/concurrent cases under load |
| NFR-05 | Planning workload, to be replaced by validated forecast | 100,000 registered customers; 10,000 daily active; 200 read requests/s and 50 command requests/s sustained; 3× burst for 5 min | One-hour sustained test plus burst; no lost accepted work or financial inconsistency |
| NFR-06 | Critical accepted financial records during failure of one availability zone | RPO 0 for acknowledged financial commits; financial write recovery ≤ 30 min after safe writer fencing | Multi-zone fault drill with measured loss/recovery and post-recovery reconciliation |
| NFR-07 | Whole-region loss / rebuild | Proposed RPO ≤ 5 min, RTO ≤ 4 h; any unreplicated financial acceptance must be recovered/reconciled before write reopening | Restore drill with explicit missing/uncertain-operation inventory; no claim of zero regional loss |
| NFR-08 | Operational projection and alert event lag under healthy dependencies | p95 ≤ 5 s, p99 ≤ 30 s; daily analytics freshness ≤ 24 h | Watermark monitoring; stale data displayed; spending still uses core |
| NFR-09 | Security revocation propagation for active access decisions | ≤ 60 s for disabled identities/privileged grants; payment authorization revalidated before financial execution | Revocation during active session and queued payment test |
| NFR-10 | Incident visibility and response | Critical financial/control alerts delivered ≤ 1 min; on-call acknowledgement ≤ 5 min | Alert drill with timestamps and fallback escalation |
| NFR-11 | Accessibility | Proposed WCAG 2.2 AA target for mobile/web critical journeys, subject to channel-specific assessment | Keyboard/screen-reader, focus, error, contrast and accessible-document review |
| NFR-12 | Maintainability and portability | Versioned contracts; backward-compatible rolling changes; documented provider/data export and restoration paths | Upgrade/rollback and consumer compatibility tests |
| NFR-13 | Audit and financial correctness | 100% critical state changes attributable; zero unbalanced commits and duplicate economic executions | Continuous invariant checks plus replay/concurrency fault scenarios |
| NFR-14 | Reconciliation timeliness | Daily completeness by next business day 10:00 in approved settlement timezone, unless rail supplement is stricter | Calendar-aware completeness report with missing files and exceptions explicitly flagged |

RTO measures restoration of the defined usable service, not just process startup. RPO describes the potential data loss window. A five-minute regional replica gap is a real financial recovery risk; regional write reopening requires provider/core reconciliation and recovery of missing acknowledged intent. If the business requires zero regional loss, architecture and cost must change before launch. Provider settlement times are rail SLAs rather than bank API latency promises.

## 13. Experience and accessibility

Critical screens are product discovery/disclosure, sign-in/recovery, application capture/evidence upload, application status/next action, account offer/terms confirmation, account overview, transaction detail, beneficiary/payment confirmation, payment tracking/cancellation request, document list/download, case status, and profile/security settings. The staff workbench requires case queues, evidence views, approval action confirmation, reconciliation drill-down and audit history. These are proposed flows, not wireframes from the source.

Before payment confirmation, show amount, currency, beneficiary, total debit including fees, requested/expected dates, rail and cancellation limits. After acceptance, show a stable reference and actual lifecycle state. Never label a merely submitted payment as completed. For balances, explain available versus ledger/held funds and show stale-state messaging. For screening/AML-related outcomes, use only the customer disclosure policy approved by Compliance.

Support keyboard and assistive navigation, clear focus order, explicit field labels, error summaries linked to fields, sufficient contrast and text alternatives; do not encode status only in color. Provide accessible statements or an equivalent accessible record. Make timeouts recoverable without losing saved work. Keep authentication alternatives and document capture assistance accessible. Supported languages, legal translations and channel/device matrix remain DEC-10.

## 14. Operations and support

Define staffed ownership for onboarding review, screening/fraud cases, uncertain payments, settlement/reconciliation breaks, ledger/GL mismatch, document generation, partner outage and security incidents. Each queue needs age/severity thresholds, escalation, permitted actions and evidence access. R1 cannot rely on unstaffed manual review as a fallback.

Runbooks shall cover provider timeout-after-acceptance, duplicate callback, missing settlement file, event backlog/dead letter, stale balance view, posting failure, GL export replay, key rotation, identity provider outage, financial writer fencing, regional restoration, customer account freeze and emergency rail pause. Each runbook identifies diagnostics, safe action, escalation, evidence to retain and recovery verification. Operators use the workbench or controlled tooling; direct journal editing is prohibited.

Use request success/error/latency, queue depth/age, consumer lag, unknown payment age, hold age, failed control decisions, screening backlog, ledger balance checks, unmatched settlement totals, GL mismatch and document failure metrics. Set correlation IDs at ingress and preserve operation, application, account, batch and provider IDs through traces. Mask account details and never expose raw evidence/secrets in telemetry.

Deploy through separated environments with synthetic data, compatible migrations, controlled configuration and feature flags. Rollback may stop new feature traffic but must continue handling existing instructions and callbacks. Schema/event compatibility windows and forward recovery are required when financial records cannot be safely downgraded. Restoration must fence stale writers, restore authoritative state, identify gaps, replay deduplicated events and reconcile before enabling new financial writes.

## 15. Analytics and reporting

Operational dashboards support live intervention; warehouse/big data assets support governed historical analysis. Neither can become a balance authority. Audience and journey services use approved purpose/consent restrictions and minimized identifiers. The repeated ecosystem analytics boxes are adapter instances of the same logical capability.

Track each funnel stage using application/operation identities: visit, started, submitted, verification pending, review, decided, account activated, first funded, payment initiated, settled/returned, support case opened/closed. Keep operational telemetry separate from optional marketing collection. KPI reports specify date window, timezone, source watermarks, cohort, denominator and exclusions. Financial and operational reports reconcile to source records; missing or late data creates a completeness flag. Regulatory report formats and submission destinations require the market-specific reporting inventory under DEC-06.

## 16. Acceptance and test strategy

The following are future **product acceptance scenarios**, not tests executed by generating this documentation. Each has a stable ID used in [Traceability](TRACEABILITY.md). Use provider simulators for deterministic failures, then certify actual providers separately. Evidence must identify environment, versions, input identities, results and retained audit artifacts.

| ID | Scenario | Required result | Key requirements |
| --- | --- | --- | --- |
| AT-01 | Customer performs a read/command in mobile and desktop; another customer substitutes the account ID | Consistent own-account result; cross-customer request denied without leakage | FR-CH-01, FR-CH-02, FR-IAM-03 |
| AT-02 | Submit and resume one application concurrently, including incomplete evidence and matching email on another application | Required fields/evidence enforced; stable application identity; no unsafe party merge | FR-ACQ-03, FR-ONB-01, FR-ONB-05, BR-13 |
| AT-03 | Identity provider times out, then sends duplicate and out-of-order results | Pending then one valid decision; no duplicate customer; expired results rejected | FR-ONB-02, FR-ONB-06, FR-CTRL-02 |
| AT-04 | Screening hit, information request and independent review | Blocking state retained; authorized evidence-backed decision; no self-approval | FR-ONB-03, FR-ONB-04, FR-CTRL-03 |
| AT-05 | Open account with current, expired and changed accepted terms; repeat successful request | Invalid gates block activation; one account and exact accepted terms evidence | FR-ACC-01, FR-ACC-02, FR-DOC-01 |
| AT-06 | Two simultaneous debits would exceed available funds; a projection attempts a direct balance write | Only authoritative permitted reservations commit; projection write denied; no negative availability under no-overdraft policy | FR-CORE-01, FR-CORE-03, FR-CORE-04, FR-PAY-05 |
| AT-07 | Internal transfer crashes before/after commit; resend same and altered idempotency payload | Atomic balanced transfer once; same result for retry; altered payload conflicts | FR-PAY-02, FR-PAY-04, BR-04 |
| AT-08 | External provider accepts but response is lost | One instruction; state reconciles via inquiry/evidence; no premature release or second economic execution | FR-PAY-05, FR-PAY-06, FR-EXT-03 |
| AT-09 | Forged/replayed callback and conflicting late settlement event | Forgery rejected; replay deduplicated; conflict quarantined and investigated | FR-EXT-01, FR-PAY-08, SEC-08 |
| AT-10 | Provider rejection, confirmed cancellation, failed cancellation and later return | Correct hold handling; no false cancellation; linked new entries for return | FR-PAY-07, FR-PAY-09, BR-09, BR-10 |
| AT-11 | Inbound payment arrives twice or targets closed/restricted account | One credit when permitted; otherwise controlled suspense/review, never silent loss | FR-PAY-08, FR-ACC-06 |
| AT-12 | Settlement file has missing, duplicate, amount, currency, fee and timing differences; GL export is replayed | Classified owned breaks and balanced unique GL effects | FR-FIN-01, FR-FIN-02, FR-FIN-04 |
| AT-13 | Application or core commits, then crashes before event publication; replay event backlog | Event eventually published; consumers recover without duplicate effects | FR-CORE-05, FR-OPS-03, FR-DATA-05 |
| AT-14 | Disable staff/customer identity, revoke delegation, change payment after step-up, attempt self-approval | Revocation meets objective; changed approval invalid; maker/checker and role boundaries hold | FR-IAM-01, FR-IAM-02, FR-IAM-04, NFR-09 |
| AT-15 | Generate statement twice, then correct a period; cross-customer/expired download attempts | Reproducible versioned totals; superseding record; unauthorized retrieval denied | FR-DOC-02, FR-DOC-03 |
| AT-16 | Data access/correction/deletion request conflicts with mandatory retention and legal hold | Verified request handled under approved policy with traceable allowed/restricted outcome | FR-DOC-04, SEC-05, SEC-06 |
| AT-17 | Failure of one zone during financial load, then regional restore exercise | Measured RPO/RTO; fenced writers; explicit gap recovery; reconcile before financial reopening | FR-OPS-04, NFR-06, NFR-07 |
| AT-18 | Representative load, burst, slow core/provider and stale read projection | Targets measured; durable backpressure; clear freshness; no relaxed spend controls | NFR-01, NFR-02, NFR-03, NFR-04, NFR-05, NFR-08 |
| AT-19 | Operate critical mobile/web journeys using keyboard and screen reader; inspect seeded sensitive logs | Accessibility evidence; no forbidden sensitive telemetry | NFR-11, SEC-05 |
| AT-20 | Provider outage, callback key rotation, emergency rail pause, recovery and overdue case | Staff alert/escalation; no new paused submissions; existing operations continue safely | FR-EXT-05, FR-OPS-01, FR-OPS-05, NFR-10 |
| AT-21 | Revoke partner consent while calls and a previously submitted payment are in flight | New unauthorized calls fail; financial lifecycle of submitted payment remains traceable | FR-IAM-05, FR-EXT-02, FR-ACC-08 |
| AT-22 | Assisted staff/ATM/call center acts for customer with restricted account | Actor and customer both traced; common restrictions; channel certification evidence | FR-CH-03, FR-CH-04 |
| AT-23 | Marketing withdrawal, audience export, analytics replay and master-data conflict | Purpose restriction upheld; lineage retained; no automatic identity merge or balance writes | FR-ACQ-02, FR-ACQ-04, FR-DATA-02, FR-DATA-03, FR-DATA-04 |
| AT-24 | Liquidity feed is late, product/fee versions change, operator submits correction | Staleness shown; historical policy resolves; approval and immutable correction trail retained | FR-FIN-03, FR-FIN-05, FR-CORE-02, FR-CORE-06 |
| AT-25 | Full customer journey plus control/report evidence reconstruction | Every milestone has consistent IDs, policy/evidence versions, financial and operational totals | FR-CTRL-01, FR-CTRL-05, FR-CTRL-07, FR-OPS-02, NFR-13, NFR-14 |
| AT-26 | Published offer, interrupted channel flow, verified profile change, paged history and support case | Offer version retained; work resumes; verified changes recheck controls; stable history and case reference | FR-ACQ-01, FR-CH-06, FR-ACC-03, FR-ACC-04, FR-ACC-05, FR-ACC-07 |
| AT-27 | Malformed/oversized/over-quota request, mandatory control outage and unsafe public error | Bounded denial/pending with no side effects or leaked evidence; safe integration errors | FR-CH-05, FR-CTRL-02, SEC-01, SEC-07, FR-DATA-01 |
| AT-28 | Additional rail adapter, advisory journey recommendation and rolling schema upgrade | Certified finality/cut-offs; advice cannot bypass controls; compatible upgrade/rollback | FR-PAY-01, FR-PAY-03, FR-PAY-10, FR-EXT-04, NFR-12 |
| AT-29 | Factor recovery, credential rotation, data export and security incident exercise | Ownership preserved; synthetic test data; keys protected; attributable response and closure | FR-IAM-06, FR-CTRL-06, SEC-02, SEC-03, SEC-04, SEC-09, SEC-10 |
| AT-30 | Seed transaction monitoring pattern and duplicate durable events | One economically scoped alert/case with evidence and attributable disposition | FR-CTRL-04, FR-CTRL-03 |

Verification layers include domain invariants, state transition/authorization checks, provider contract tests, event/schema compatibility, end-to-end journeys, concurrency/replay/fault tests, performance, accessibility, security review and recovery exercises. Local mocks prove bank behavior under modeled responses; they do not prove live provider certification, rail settlement or regulatory compliance.

## 17. Delivery plan and dependencies

This is an ordered milestone plan rather than an invented calendar or budget. Estimates require team capacity, vendor lead times and DEC-01 through DEC-07. Dependencies run left to right; control and recovery work starts in R0 and remains release-blocking where applicable.

| Milestone | Depends on | Concrete deliverables / exit |
| --- | --- | --- |
| M0 — Product and market baseline | Sponsor and control owners | Customer/product/jurisdiction scope; bank/partner operating model; named owners; confirmed R1 rail and chart-of-accounts principles |
| M1 — Secure platform contracts | M0 | Gateway/IAM/entitlements, IDs/idempotency, authoritative ownership, audit/event contracts, environments, monitoring, simulators |
| M2 — Acquisition to active account | M1 + identity/screening providers | Verified onboarding with manual review, product/terms versioning, idempotent opening, inquiry and documents |
| M3 — Financial movement and operations | M1/M2 + core + selected rail | Internal/external/inbound lifecycle, core holds/postings, provider inquiries, exceptions, reconciliation and finance mappings |
| M4 — Controlled R1 pilot | M2/M3 + staffed operations | P0 R1 acceptance evidence, provider certification, reporting, recovery/security/accessibility evidence, approved limits and support coverage |
| M5 — R2 expansion | Stable R1 + adapter/provider contracts | Assisted channels, partner APIs/delegation, additional rail supplements, enterprise data and consented journey integrations |
| M6 — R3 optimization | Measured production baselines | Advanced routing, personalization, liquidity automation, capacity/cost tuning and outcome evaluation |

Key external dependencies are bank/license or sponsor model, core integration, identity and screening providers, payment schemes/processors, settlement accounts, legal documents, staff operating capacity, approved data residency/retention, and channel certification. An unavailable dependency may defer a feature; it cannot justify bypassing a required control.

## 18. Risks and mitigations

| ID | Risk / impact | Mitigation / evidence / owner |
| --- | --- | --- |
| RISK-01 | Diagram mistaken for a deployable or approved banking design | Evidence labels, DEC register, signed launch scope; Product / Architecture |
| RISK-02 | Duplicate external payment after timeout | Durable intent, stable provider key, inquiry/reconciliation, AT-08; Payments |
| RISK-03 | Overspend or unbalanced financial state under concurrency/failover | Core atomic invariants, writer fencing, AT-06/07/17; Core / SRE |
| RISK-04 | Control/provider outage creates unsafe approval or unbounded backlog | Safe pending, capacity and staffed queues, aging escalation, AT-03/20/27; Compliance / Operations |
| RISK-05 | Launch market and rail assumptions conflict with legal/finality requirements | DEC-01/04/06; certified rail supplements before enablement; Compliance / Payments |
| RISK-06 | Analytics/profile data exposes sensitive evidence or violates purpose restrictions | Data classification, minimization, restricted evidence store, AT-16/23; Security / Data |
| RISK-07 | Operational queues, reconciliation or finance exports are omitted from MVP | R1 P0 scope and AT-12/20/25; Operations / Finance |
| RISK-08 | Regional data loss loses acknowledged intent | Explicit RPO, recovered/provider evidence inventory, fenced reopen gate, AT-17; SRE / Finance |
| RISK-09 | Service decomposition creates excessive latency or operational burden | Proposed deployables can be consolidated without changing domain ownership; measure NFRs; Architecture |
| RISK-10 | Additional rails/channels overwhelm delivery or unsupported guarantees are implied | Release gates, adapter supplements, per-channel certification; Product / Integration |
| RISK-11 | Terms, fees or identity merges retroactively alter evidence | Versioned snapshots, independent control, immutable financial links, AT-05/24; Product / Core |
| RISK-12 | Budget and capacity are unknown | Estimate after provider selection and workload ratification; include evidence storage, replicas, control calls, monitoring and staff operations; Sponsor |

## 19. Assumptions and open decisions

### 19.1 Explicit assumptions

| ID | Proposed working assumption | Consequence if false |
| --- | --- | --- |
| ASM-01 | R1 serves adult individual retail customers and one basic deposit product | Add legal entity, joint/minor/delegate eligibility and authority rules before launch |
| ASM-02 | R1 uses one currency with configured scale, no FX and no overdraft | Add FX accounting, credit authorization and altered availability rules |
| ASM-03 | Core can provide atomic holds/postings, stable business refs and durable change publication | Add an owned financial subsystem or replace integration contract; do not emulate money integrity in projections |
| ASM-04 | Selected rail provides stable deduplication or inquiry evidence; manual fallback is contractually possible | Redesign submission/recovery strategy or do not enable the rail |
| ASM-05 | Staff are available to operate manual controls and reconciliation throughout the pilot's committed service hours | Narrow operating windows or delay pilot |
| ASM-06 | Technology/provider choices remain open; diagram services need not each become a microservice | Ratify implementation constraints under DEC-05 |

### 19.2 Decision register

| ID | Decision required / current status | Accountable role | Deadline / affected gate |
| --- | --- | --- | --- |
| DEC-01 | Launch country, legal entity, bank/sponsor model, target segment, applicable oversight and product eligibility — TBD | Sponsor / Compliance / Legal | M0; blocks real-customer launch |
| DEC-02 | Confirm MM means money movement and clarify source ambiguities/repeated analytics boxes — provisional interpretation | Product / Architecture | M0/M1 |
| DEC-03 | Product terms, fees/rates, limits, currency, overdraft, interest/accrual, tax and chart-of-accounts mapping — TBD | Product / Finance / Compliance | M2/M3; blocks account/payment activation |
| DEC-04 | R1 external rail/provider, settlement accounts, finality, cut-offs, returns, cancellation, inbound restrictions and dispute windows — TBD | Payments / Finance / Compliance | M3; blocks external rail enablement |
| DEC-05 | Build/buy boundaries, vendors, runtime stack, physical data stores, cloud/residency, actual deployment decomposition — TBD | Engineering / Architecture / Security | M1 |
| DEC-06 | Applicable control/reporting inventory, retention/residency, legal holds, required disclosures and dispute policy — TBD | Compliance / Legal / Security / Finance | M2; blocks launch |
| DEC-07 | Workload, availability, recovery targets, regional financial RPO risk and budget/staffing — proposed targets only | Product / SRE / Finance / Sponsor | M1 and M4 |
| DEC-08 | Distinguish or consolidate source Tx Service versus Transaction Service; master-data authority — provisional split | Architecture / Core / Data | M1 |
| DEC-09 | Exact assisted-channel/ATM/debit/private-banker operations and partner consent/delegation model — TBD | Channel / Integration / Security | Before relevant R2 build/certification |
| DEC-10 | Supported languages, devices, accessibility assessment, service hours and customer communication channels — TBD | Product / Design / Operations | M2/M4 |
| DEC-11 | Key retention/replay windows, provider evidence retention, event retention, max hold age and unresolved-payment escalation — TBD | Payments / Core / Security / Operations | M3; no automatic unsafe expiry |
| DEC-12 | Named owners, pilot limits, milestone estimates, cost envelope and launch acceptance authority — TBD | Sponsor / Product / Engineering | M0 and M4 |

Decisions are gates on implementation or launch, not requests for permission to produce this document. They do not prevent completing the architecture proposal.

## 20. Readiness and change control

R1 is ready only when P0 R1 requirements have evidence, enabled rail/provider contracts and certification are complete, legal/product/control policies are approved by accountable owners, operational queues are staffed, financial reconciliation and document production work, and recovery/security/accessibility findings are resolved to the accepted release policy. Complete AT-01 through AT-20 and AT-24 through AT-30 for R1-relevant functionality; AT-21/22/23 and AT-28 expansion portions apply when their R2/R3 features are enabled. Record non-applicable branches explicitly rather than marking unexecuted tests as passed.

The release record shall state requirement/version, scenario/environment, evidence link, owner, result, exception and expiry. An owner may accept a scoped operational limitation only under the organization's approved release authority; a limitation cannot silently remove a financial invariant or mandatory control. There is no sign-off or compliance claim in this documentation package.

Changes to scope, financial state transitions, event contracts, ownership, rail finality or control policy require synchronized updates to this PRD, architecture, diagrams and traceability. Version the decision and explain affected acceptance cases. Estimate changes after owners ratify assumptions; do not silently convert a proposed target into a contractual promise.

## 21. Architecture and UML index

The [diagram catalog](diagrams/README.md) provides editable PlantUML and rendered SVG for all views:

- C4 L1 system context and L2 container landscape.
- C4 L3 components for onboarding, core banking, payments, and controls, plus L4 proposed payment-domain classes in UML notation.
- C4 dynamic payment flow and deployment topology.
- UML use cases, domain classes, onboarding/payment/reconciliation/authorization/document sequences, account-opening and exception activities, application/account/payment state machines, and event integration components.

C4 levels describe zoom depth, not development stages. The source's L2 badges are preserved as provenance in the inventory and are not reinterpreted as C4 L2 containers. [Official C4 documentation](https://c4model.com/diagrams.html) defines its static and supporting view types; [C4 components](https://c4model.com/abstractions/component) are internal to containers rather than independently deployable units.

## 22. Glossary and references

| Term | Meaning in this specification |
| --- | --- |
| API Gateway / DMZ | Controlled ingress/routing capability in an exposed network zone; the source does not define implementation |
| Experience API | Channel-facing aggregation and adaptation layer |
| Open APIs | Externally exposed partner interfaces requiring explicit client authority/consent |
| IAM / entitlement | Identity authentication/session management / permission to act on a resource in a context |
| MM | Money movement, provisionally interpreted from the source |
| KYC / AML | Know-your-customer / anti-money-laundering capability labels; applicable obligations TBD |
| Journal / subledger / GL | Immutable balanced operational postings / detailed financial records / enterprise general ledger |
| Hold / available balance | Reservation restricting spendability / funds permitted to be spent under account policy |
| Settlement / finality | Completion under the rail's evidence and accounting rules / point at which specified obligations become final |
| Reconciliation | Comparing independent operational, settlement and financial evidence and assigning differences |
| Outbox / inbox | Producer records change+publication intent atomically / consumer records deduplication and local effect atomically |
| Saga / orchestration | Durable multi-step workflow with explicit recovery; no implied distributed atomic transaction |
| RPO / RTO | Maximum targeted recovery data loss window / maximum targeted time to restore the defined service |
| C4 | Context, containers, components and code; containers mean deployable applications/data stores, not only OS containers |
| P0 / R1 | Release-blocking priority / proposed limited retail launch |

Primary product source: the supplied architecture image (kept outside this repository), with the original credits retained in the root README. Modeling/tool references: [C4 diagram hierarchy](https://c4model.com/diagrams.html), [C4 deployment scope](https://c4model.com/diagrams/deployment), [C4-PlantUML](https://github.com/plantuml-stdlib/C4-PlantUML), and [PlantUML command-line documentation](https://plantuml.com/command-line). The proposed accessibility target refers to [W3C WCAG 2.2](https://www.w3.org/TR/WCAG22/). These references guide notation, rendering and accessibility criteria; they do not supply banking business requirements or evidence that the proposed bank exists.
