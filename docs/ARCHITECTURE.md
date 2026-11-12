# Digital Bank — Proposed Architecture Specification

**Version:** 1.0, 2026-10-22. **Status:** Proposed logical/implementation design derived from the supplied image. Read with the [PRD](PRD.md), [traceability](TRACEABILITY.md), and [diagram gallery](diagrams/README.md). No service, endpoint, event, datastore technology or infrastructure topology below is an observed implementation.

## Interpretation and boundaries

The image supplies a capability architecture. This specification proposes a deployable decomposition while preserving every named capability. It does not require one microservice per source box. A **C4 container** is a deployable application or data store; a **component** is internal to one container. The proposed container groupings can be consolidated for delivery if ownership, authorization and financial invariants are preserved. The [official C4 abstractions](https://c4model.com/abstractions/component) support this distinction.

The bank system boundary contains owned channels, gateway, experience/open APIs, IAM, journey processes, core, payments, controls, operational data services, finance, documentation and enterprise data infrastructure. External identity/screening vendors, payment networks/processors and journey partners are outside it. A purchased core can still be a logical bank-owned authority; its physical/vendor boundary requires DEC-05. Customers, staff, partners and auditors are distinct actors.

The source places controls across front/middle/back office and the event bus along multiple layers. Both are cross-cutting concerns, not sequential final stages. Its arrows are not a complete call graph. Relationships added here are explicit proposals, including authentication, provider callbacks, financial reconciliation and event publication. Every diagram title identifies its scope and proposed status.

## Source capability inventory

“Observed” below means the label is present in the image. The target placement and behavior are proposed. The **Detail badge** column records the source's L2 annotation and does not assign C4 depth. Group IDs support source-to-requirement traceability.

| Source ID | Layer / observed label | Detail badge | Proposed owner / representation |
| --- | --- | --- | --- |
| SRC-01 | Customer channels: Mobile Banking | — | Mobile client container |
| SRC-02 | Customer channels: Desktop Banking | — | Web client container |
| SRC-03 | Other Channels: Branch | — | Assisted-channel adapters; R2 |
| SRC-04 | Other Channels: Private Banker Portal | — | Assisted-channel portal adapter; R2 |
| SRC-05 | Other Channels: ATM | — | Certified ATM adapter; R2 |
| SRC-06 | Other Channels: Call Center | — | Assisted-channel adapter; R2 |
| SRC-07 | DMZ: API Gateway | — | Gateway container / edge trust zone |
| SRC-08 | Experience & IAM Services: Open APIs | — | Partner API container |
| SRC-09 | Experience & IAM Services: Experience APIs | — | Channel API container |
| SRC-10 | Experience & IAM Services: IAM | — | IAM container; separate human/workload/partner trust contexts |
| SRC-11 | Front & Middle Office: Orchestration | — | Journey process container / durable workflows |
| SRC-12 | Business Process Services: Customer Acquisition | L2 | Acquisition component in Journey processes |
| SRC-13 | Business Process Services: New Customer Onboarding | L2 | Onboarding component in Journey processes |
| SRC-14 | Business Process Services: New Account Opening | L2 | Account-opening component in Journey processes |
| SRC-15 | Business Process Services: Account Servicing | L2 | Servicing component in Journey processes |
| SRC-16 | Business Process Services: Payment & Money Movement | L2 | Payment-intent component in Journey processes; execution in Payments |
| SRC-17 | Core Banking Platform: Accounts & Parties | — | Core account/party registry and relationship ownership |
| SRC-18 | Core Banking Platform: Product Mgmt and Pricing | — | Core effective-dated product/pricing component |
| SRC-19 | Core Banking Platform: Journal & Sub-ledger | — | Core journal/posting authority |
| SRC-20 | Core Banking Platform: Events & Core Workflow | — | Core workflows and transactional outbox |
| SRC-21 | Data Services: Alerts & Notifications | — | Data/engagement delivery component |
| SRC-22 | Data Services: Customer Service | — | Customer query façade; does not mean a replacement customer-service desk |
| SRC-23 | Data Services: Account Service | — | Account query façade |
| SRC-24 | Data Services: Audience Service | — | Consented segmentation component |
| SRC-25 | Data Services: Tx Service | — | Proposed transaction command façade; DEC-08 |
| SRC-26 | Data Services: Transaction Service | — | Proposed transaction inquiry projection; DEC-08 |
| SRC-27 | Data Services: Insight Service | — | Governed derived insights component |
| SRC-28 | Enterprise Data Assets: Big Data Platform | L2 group | Enterprise event/data processing application and analytical object store |
| SRC-29 | Enterprise Data Assets: Mastered Data | L2 group | Governed mastered-data store; identity links preserve core authority |
| SRC-30 | Enterprise Data Assets: Data Warehouse | L2 group | Warehouse and reporting models |
| SRC-31 | Back Office: Payments & MM Engine / Approvals & Routing | L2 group | Payments approval coordinator, routing and limit policy |
| SRC-32 | Back Office: Payments & MM Engine / Execution (Clearing, Settlement) | L2 group | Payment lifecycle, rail adapters, inquiry and settlement matcher |
| SRC-33 | Finance: General Ledger | — | Finance GL integration/component; separate enterprise ledger |
| SRC-34 | Finance: Financial Reporting & Liquidity | — | Finance reporting and position service |
| SRC-35 | Documentation: Terms & Conditions | L2 group | Document evidence/version component |
| SRC-36 | Documentation: Statements | L2 group | Statement snapshot/generation component |
| SRC-37 | Controls: Ops Reporting | L2 group | Restricted operations reporting component |
| SRC-38 | Controls: Enterprise Entitlements | L2 group | Domain authorization policy component; IAM authenticates |
| SRC-39 | Controls: Sanction Screening | L2 group | Screening and review component |
| SRC-40 | Controls: Risk & Compliance | L2 group | Risk decision and compliance case components |
| SRC-41 | Controls: Fraud & AML | L2 group | Real-time decision and retrospective monitoring components |
| SRC-42 | Controls: Identity Verification | L2 group | Verification integration / restricted evidence component |
| SRC-43 | Cross-cutting: Event Bus | — | Durable broker container with producer outboxes and consumer receipts |
| SRC-44 | Ecosystem Partners: Journey Analytics & Orchestration (three occurrences) | Left occurrence L2 | One logical partner capability with multiple provider adapters; counts not interpreted as distinct products |
| SRC-45 | Payments & MM Rails: ACH | — | Conditional certified external adapter |
| SRC-46 | Payments & MM Rails: Checks | — | Conditional check/deposit-processing adapter |
| SRC-47 | Payments & MM Rails: Debit | — | Conditional debit adapter; issuing scope unconfirmed |
| SRC-48 | Payments & MM Rails: Bill pay | — | Conditional bill-payment adapter |
| SRC-49 | Payments & MM Rails: Wire | — | Conditional wire adapter |
| SRC-50 | Payments & MM Rails: Lockbox | — | Conditional receivables/lockbox adapter |
| SRC-51 | Payments & MM Rails: Swift | — | Conditional SWIFT message/network adapter; settlement evidence separate |
| SRC-52 | Payments & MM Rails: P2P | — | Conditional peer-to-peer scheme adapter |

Source datastore symbols indicate persistent records within core, enterprise assets, finance, documentation, controls and partner capabilities. Their exact placement does not establish a shared database or transactional boundary. Added design elements include operation/idempotency stores, holds, outboxes/inboxes, consent records, a staff workbench, restricted evidence storage, network zones, key management, observability and recovery infrastructure. They are supporting proposals rather than extra facts attributed to the image.

## Proposed container responsibilities

| Container / stable alias | Owns | Does not own |
| --- | --- | --- |
| Mobile / `mobile`, Web / `web`, Staff workbench / `staff`, Assisted adapters / `assisted` | Presentation, accessible flows, channel state; assisted channel adaptation | Account balances, final payment status or privileged authorization |
| API Gateway / `gateway` | Edge identity validation, ingress policy, request limits, routing, correlation | Domain permission or financial decision authority |
| Experience APIs / `experience` | Channel payloads and bounded aggregation | Journal writes or duplicated product rules |
| Open APIs / `openapi` | Partner schemas, client/grant validation, quotas and consent boundary | Staff impersonation or independent payment execution |
| IAM / `iam` | Subject/client identity, factors, sessions, credentials, revocation, enforceable consent grants | Financial account ownership or screening decisions |
| Journey processes / `journey` | Acquisition/application/workflow state, onboarding, account opening, servicing/payment-intent coordination | Core financial truth; direct provider execution |
| Core banking / `core` | Party/account/product records, restrictions, holds, journal/subledger, authoritative balance and atomic posting | Provider/network settlement truth or channel aggregation |
| Payments engine / `payments` | Payment instruction identity/state, approvals, routing, adapters, provider evidence, settlement matching | Mutable copies of authoritative core balances |
| Controls / `controls` | Entitlements, identity/screening/risk/fraud decisions, restricted evidence, cases, audit/ops reporting | Silent authorization fallback or direct journal edits |
| Data & engagement services / `data` | Customer/account/transaction query projections, transaction command façade, alerts, audience, insight and journey adapters | Spending authorization or authoritative master identity merge |
| Enterprise data processing / `analytics` | Governed ingestion, lineage, transformations and historical analytics | Operational writes to financial masters |
| Finance / `finance` | GL mapping/export, reconciliation, reporting, liquidity positions and finance evidence | Overwriting the operational journal |
| Documentation / `documents` | Terms versions/acceptance, immutable statement versions, retrieval and retention evidence | Calculating balances from inconsistent read projections |
| Event Bus / `bus` | Durable transport, partitions and delivery metadata | Exactly-once economic execution or source-of-record ownership |

Stores: `journeyDb`, `coreDb`, `paymentDb`, `controlDb`, `iamDb`, `projectionDb`, `financeDb`, `documentDb`, `evidenceStore`, `documentStore`, `masterData`, `warehouse`, and `dataLake`. These are logical stores with one owning container per transactional domain, not prescribed database products. Physical consolidation is permitted with enforced access/schema ownership. Evidence and documents have distinct access/retention policies. Analytics processing controls the enterprise datasets; core remains authoritative for operational party/account identity.

## C4 hierarchy and modeling scope

| View | Parent / purpose |
| --- | --- |
| L1 context | Digital Bank with people, partner applications and external systems |
| L2 containers | Decomposition of Digital Bank into proposed deployable containers and owned stores |
| L3 onboarding | Components inside `journey`, with external dependencies shown as containers |
| L3 core | Components inside `core`, with `coreDb` and other containers outside the component boundary |
| L3 payments | Components inside `payments`, including provider adapter interfaces and local evidence/state |
| L3 controls | Components inside `controls`, including entitlements, distinct control checks, cases and audit |
| L4 code | Proposed classes/interfaces inside the Payment lifecycle component from L3 payments; design, not generated implementation |
| Dynamic | Numbered container interactions for an external outgoing payment |
| Deployment | Proposed production topology mapping container instances to trust zones and recovery infrastructure |

The generic UML domain model is a separate business view. It is not labeled C4 L4 because it spans multiple owning containers. The focused payment code view supplies the requested fourth C4 zoom level honestly, without claiming that implementation classes exist.

## Authoritative financial transaction boundary

All account availability changes and posting effects commit inside core's transactional boundary, with row/version locking or equivalent serializable invariants. Each operation/posting kind has a durable unique business key. A hold transition from active to consumed/released is committed with the relevant journal/balance change. A crash cannot consume a hold without its posting or release funds twice. A core result is discoverable by business key even if the caller loses the response.

For outgoing external settlement, the proposed booking is a debit to customer deposit liability and credit to the configured clearing/settlement account, consuming the hold in the same core transaction. Inbound final credit debits the configured settlement/receivable account and credits customer liability; restricted/provisional treatment follows the rail-specific accounting policy. Internal transfers debit one customer liability and credit another atomically in one currency. These are proposed baseline patterns; Finance defines the actual chart, fees, accruals, suspense and finality policy under DEC-03/04.

Payments and core have separate stores. Payments can receive external settlement evidence before the core posting succeeds; retain `RECONCILING` with a settlement-confirmed subreason and retry the **same** idempotent core posting. Report `SETTLED` only after both valid settlement evidence and required core booking are present. Conversely, if core commits but the payment-state update crashes, query the same core business key and recover the state. This is a recoverable workflow, not a distributed transaction.

Hold expiration cannot release a potentially submitted payment automatically. Pre-submission stale holds can be released only when the lifecycle proves non-submission; uncertain cases escalate under DEC-11. Spend decisions consider authoritative holds/restrictions even when an account inquiry projection is stale.

## Proposed API surface

Paths are illustrative design contracts under `/api/v1`, not extracted router declarations. Staff paths require distinct audience and permission. External callbacks use a separately authenticated provider ingress policy. Exact schemas and versioning must be specified before implementation.

| Method / path | Owner and caller | Behavior / response |
| --- | --- | --- |
| `GET /products` | Experience → Journey/Core | Published current offers, versions and eligibility disclosures |
| `POST /applications` | Experience → Journey | Idempotent create; `201` with application ID and state |
| `GET /applications/{id}` | Experience → Journey | Owner-authorized state, required next action and permitted explanation |
| `PATCH /applications/{id}` | Experience → Journey | Version-checked editable fields only; submitted decision evidence not overwritten |
| `POST /applications/{id}/submissions` | Experience → Journey | Durable submit; `202` with operation/status reference |
| `POST /accounts` | Experience → Journey → Core | Idempotent approved opening; `202` while gates/activation pending |
| `GET /accounts` and `GET /accounts/{id}` | Experience/Open APIs → Data/Core | Relationship-authorized inquiry; posted/available/held, currency, as-of/source |
| `GET /accounts/{id}/transactions` | Experience/Open APIs → Data | Cursor pagination over a stable ordering/snapshot |
| `POST /accounts/{id}/closure-requests` | Experience → Journey/Core | Durable closure assessment; no forced loss of funds |
| `POST /beneficiaries` | Experience → Payments | Validate/store permitted beneficiary; sensitive details tokenized/masked |
| `POST /payment-quotes` | Experience/Open APIs → Journey/Payments | Fee/date/rail quote, expiry and material-details digest; no posting |
| `POST /payments` | Experience/Open APIs → Journey/Payments | Idempotent durable intent, approval context and quote; normally `202` |
| `GET /payments/{id}` | Experience/Open APIs → Payments | Canonical state, next action, provider status summary and linked movements |
| `POST /payments/{id}/cancellation-requests` | Experience → Payments | Idempotent request; completion only after safe cancellation proof |
| `GET /operations/{id}` | Experience/Open APIs → owning workflow | Current durable workflow outcome and authorized resource link |
| `GET /documents` and `GET /documents/{id}` | Experience → Documentation | Authorized metadata and short-lived retrieval reference |
| `POST /consents` and `POST /consents/{id}/revocations` | Customer authorization → IAM | Explicit bounded grants and prospective revocation; R2 partner use |
| `POST /service-cases` | Experience → Journey/Controls | Trackable complaint/dispute/service request |
| `GET /staff/cases` and `POST /staff/cases/{id}/decisions` | Staff → Controls | Restricted queue and independently approved policy-bound decision |
| `POST /staff/adjustment-requests` | Staff → Finance/Core | Maker/checker correction proposal; never direct entry editing |
| `POST /integrations/{provider}/callbacks` | Authenticated provider → owning adapter | Persist/deduplicate evidence before acknowledgement; invalid authenticity rejected |

Common headers/metadata: authentication context, request/correlation ID, `Idempotency-Key` for commands, expected resource version for updates, and approved actor/delegation context. Use `400/422` for invalid input, `401` for unauthenticated, `403` or non-disclosing `404` for unauthorized resources, `409` for fingerprint/version conflicts, `429` for throttling, `503` for safe unavailable acceptance, and `202` for durably accepted pending work. A response must not claim acceptance until intent is durable. An unknown financial outcome returns/query-exposes the known operation rather than prompting a new payment.

Illustrative external payment command:

```json
{
  "payerAccountId": "acct_123",
  "beneficiaryId": "ben_456",
  "amount": { "minorUnits": 2500, "currency": "<launch currency>" },
  "rail": "<enabled rail>",
  "quoteId": "quote_789",
  "approvalProofRef": "proof_321",
  "requestedExecutionDate": "2026-11-02",
  "purpose": "Customer-provided permitted purpose"
}
```

Actor identity, ownership, quote fee and approval validity are verified server-side; the body cannot assert them as trusted facts. The currency/rail placeholders deliberately avoid selecting a jurisdiction. A durable accepted response contains `operationId`, `paymentId`, `state`, `version`, and a status URL, without claiming settlement.

## Events and asynchronous workflows

Illustrative envelope (identifiers and timestamps are examples):

```json
{
  "eventId": "evt_001",
  "eventType": "payment.settlement-confirmed",
  "schemaVersion": 1,
  "producer": "payments",
  "aggregateType": "PaymentInstruction",
  "aggregateId": "pay_001",
  "aggregateVersion": 7,
  "occurredAt": "2026-10-22T16:00:00Z",
  "correlationId": "corr_001",
  "causationId": "provider_receipt_001",
  "data": {
    "operationId": "op_001",
    "settlementEvidenceRef": "evidence_001"
  }
}
```

This event records provider evidence, not necessarily completed core booking. `payment.settled` follows successful idempotent booking. Payloads use opaque references; raw identity documents, tokens, full account details and unnecessary personal information do not belong on broad topics.

| Producer | Proposed events | Consumers / permitted effect |
| --- | --- | --- |
| Journey | `application.submitted`, `application.decided`, `account.opening-requested` | Controls/workflow continuation; Data funnel projections |
| Controls | `identity.decision-recorded`, `screening.decision-recorded`, `control.case-opened`, `entitlement.changed` | Journey/Payments continuation with revalidation; operations reporting; IAM cache invalidation as applicable |
| Core | `customer.created`, `account.activated`, `account.restricted`, `hold.changed`, `journal.batch-posted` | Journey/Payments recovery, Data projections, Finance mapping, statement input |
| Payments | `payment.accepted`, `payment.submission-unknown`, `payment.settlement-confirmed`, `payment.settled`, `payment.failed`, `payment.return-recorded` | Reconciliation, Finance, Data/notifications and case handling |
| Finance | `reconciliation.break-opened`, `reconciliation.completed`, `gl.export-acknowledged` | Controls/Operations, reporting and document readiness |
| Documentation | `terms.accepted`, `statement.published`, `document.superseded` | Journey gating, alerts and customer document index |
| IAM | `consent.revoked`, `identity.disabled` | Open APIs/domain authorization invalidation and permitted audit reporting |

Publish from a producer outbox committed with domain changes. Partition ordered streams by aggregate identity; do not assume ordering across aggregates. Consumers atomically record receipt and local effect where possible; external effects use a durable work item plus an idempotent adapter/core key. Retrying the event bus is not a retry policy for a non-idempotent provider financial instruction. Retain deduplication references through the business replay/retention horizon (DEC-11), independently of short-lived API response caches. Quarantine incompatible schemas and conflicting evidence; a dead-letter queue has an owner and deadline. Replay preserves original event and operation identities and suppresses unintended customer notifications as policy requires.

## Rail capability matrix

Every enabled rail requires a supplement covering supported directions, participant/beneficiary validation, limits, calendar/cut-offs, fee/FX policy if ever enabled, instruction deduplication, acknowledgements, finality, cancellation, return/reversal, inbound controls, evidence retention, reconciliation and certification. The following rows specify questions to resolve, rather than unsupported scheme guarantees.

| Observed option | Intended capability / scope to confirm | Required rail-specific decisions and evidence |
| --- | --- | --- |
| ACH | Batch electronic debit/credit movements where supported in selected market | Scheme/geography, permitted debit authority, batch windows, acknowledgements, return windows and provisional/final funding policy |
| Checks | Check deposit and/or issuance; enabled direction TBD | Image/evidence standards, duplicate presentment, holds/availability, return evidence and loss/stop-payment policy |
| Debit | Debit transfer/processor integration; card program not assumed | Processor/program scope, authorization/clearing distinction, reversals, disputes and any issuing/credential requirements |
| Bill pay | Customer payment to billers | Biller directory/validation, delivery mode, remittance matching, proof and undeliverable/return handling |
| Wire | High-value transfer capability | Network, beneficiary/routing validation, cut-offs, confirmation/finality, recall requests and approval limits |
| Lockbox | Receivables collection/matching | File format, payer references, unmatched receipts, deposit evidence and customer posting policy |
| Swift | Financial messaging adapter; settlement is separate | Message types, correspondent relationships, tracking references, fees and independent settlement confirmation |
| P2P | Person-to-person scheme transfer | Scheme identity/directory, alias resolution, beneficiary confirmation, fraud limits, finality and dispute policy |

R1 chooses one certified external rail under DEC-04; all rows remain part of the target architecture. A rail is disabled unless its supplement and acceptance evidence exist. Inbound and outbound semantics can differ even on the same rail.

## Security and trust zones

The deployment view proposes public channel devices, edge/DMZ, application, restricted financial/control data, analytical data and external-provider zones. Workloads authenticate each other; gateway validation does not replace domain authorization. IAM authenticates, Controls evaluates entitlements, Core verifies account relationship/restriction, and Payments verifies execution authority and current approval/control evidence before financial action.

Staff and partner identities use distinct audiences/clients and scoped policies. Delegated calls preserve both actor and customer identities. Proofs are purpose/action-bound, expire and are invalidated by material changes. Account recovery is a risk-sensitive workflow. Credentials/keys belong to a secrets/key management capability; evidence, documents and analytics use purpose-specific access. Audit records and financial journals have integrity protection and controlled retention. Detailed controls are in PRD SEC-01–SEC-10.

## Deployment and recovery proposal

Use a provider-neutral topology: an edge tier, horizontally scalable application instances across at least two failure zones, synchronously protected authoritative financial storage for the single-zone RPO target, durable replicated messaging and object/evidence storage, plus asynchronous recovery-region replication and backups. Runtime, database and cloud remain DEC-05. Zones are a planning abstraction, not proof that a particular topology satisfies the targets.

The deployment diagram identifies replicated instances for the logical application containers and storage groups whose logical store membership is enumerated above. “Multi-zone” does not automatically imply synchronous protection for every store. Financial intent/core/payment/critical control stores need an agreed acknowledgement durability boundary. Analytics may tolerate longer recovery, while financial writes require fencing and reconciliation.

A recovery sequence is: pause new financial commands, fence old writers, restore/fail over authoritative stores, identify unreplicated acknowledged intent and uncertain external submissions, reconcile provider/core evidence, replay deduplicated events, verify journal/GL/hold invariants, and reopen a bounded pilot before general traffic. A regional RPO of five minutes can lose acknowledged local records; operations must recover those gaps before new spending is allowed. The source does not specify disaster recovery; all targets and topology are proposed.

## Decision rationale

| Decision proposal | Rationale / trade-off | Ratification |
| --- | --- | --- |
| Keep Core authoritative for financial state | Makes spend/post integrity locally atomic; requires strong core contracts | DEC-03/05, ASM-03 |
| Use durable workflows and at-least-once events | External settlement cannot be a local transaction; requires explicit recovery and deduplication | DEC-05/11 |
| Separate IAM, entitlement, control and account ownership checks | Preserves authority boundaries and prevents one passed check replacing others; adds integration latency | DEC-05/06 |
| Group source services into deployable containers | Limits operational fragmentation while retaining domain boundaries; exact deployment size remains open | DEC-05/08 |
| Keep read/analytics projections outside spend decisions | Supports scalable inquiry and insights with visible freshness; requires projection lag management | DEC-07/08 |
| Ship basic operations/finance/docs with R1 | Manual review, financial records and recovery are essential to enabled journeys; increases initial scope | DEC-12 |
| Disable uncertified rails and channels | Prevents generic lifecycle assumptions being mistaken for scheme guarantees; expansion needs supplements | DEC-04/09 |

Notation references: [C4 view types](https://c4model.com/diagrams.html), [C4 components](https://c4model.com/abstractions/component), [C4 deployment](https://c4model.com/diagrams/deployment), [C4-PlantUML](https://github.com/plantuml-stdlib/C4-PlantUML), [PlantUML sequence](https://plantuml.com/sequence-diagram), [PlantUML state](https://plantuml.com/state-diagram) and [PlantUML class](https://plantuml.com/class-diagram). Requirements and design choices originate in this proposal, not those notation references.
