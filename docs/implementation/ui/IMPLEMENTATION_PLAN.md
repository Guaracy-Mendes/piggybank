# Official UI templates and consumer wealth implementation

Authorized 2026-11-16. This work extends the completed essential WP-24 baseline. The user has explicitly requested the HTML bundles in `mock/` as the official look and feel and the union of their interactions and the complete UI PRD as functionality. Original files remain intact; [source hashes and screen status](backlog.json) retain provenance.

## Source reconciliation

The HTML consistently uses emerald `#1D6E4E`, slate-teal `#1E2D2F`, mint canvas `#F6F9F8`, white 16px cards, fine borders and Inter/Plus Jakarta Sans. These visual values take precedence over the supplied Markdown's conflicting coral/indigo palette. Desktop uses the HTML sidebar/bento structure; mobile follows the HTML cards, safe areas and the PRD's Home/Save/Spend/Invest/Profile bottom tabs. Both light and dark appearance are retained as preferences.

HTML and PRD numbers describe different screens. For example HTML 11 is developer access, while PRD 11 is vault detail; HTML 27–29 are desktop portfolio/trading/autopilot and 30–32 mobile investing/detail/watchlist. Both capabilities are required. UI identifiers below refer to PRD numbering; source HTML numbering is preserved separately in the implementation record.

## Steps to execute

1. Establish shared official design tokens, responsive desktop/mobile shell and component styling. Preserve access to the existing account, payments, onboarding, periodic finance and staff controls.
2. Add owner-scoped persisted preferences, financial goals, vaults/shared contributions/locks, budgets, category metadata, payment requests, linked-bank simulation, card controls, notifications and statement exports. Core remains the sole money authority; no browser-local balances or success-only buttons.
3. Add a synthetic brokerage with separately funded cash, native positions, an explicitly simulated asset/price catalogue, native bounded fractional arithmetic, exact quote review, action-bound issuer reauthentication, market/limit orders, cancellation and immutable fill/cash receipts. Submitted orders and confirmed fills remain distinct.
4. Add watchlists, allocation/performance from retained facts, declared synthetic dividends/DRIP and versioned recurring consent. Serialize concurrent orders and automation; retry one original economic effect and retain failures for review.
5. Build all consumer workspaces using the HTML styling and functional PRD, including desktop investment terminal and mobile flows. Use real stored records, filters, charts and saved routes. Provider/device capabilities use actual available adapters; simulated prices, links and cards are labeled. Do not copy unsupported FDIC/SIPC membership or invented returns from demonstration markup.
6. Validate exact financial effects/holds/ownership/replay/cancellation/insufficient funds in PostgreSQL, then actual issuer/browser flows and mobile/accessibility. Run shared-database suites sequentially. Reconcile documentation/contracts/diagrams with final implementation and accurately mark any provider/device qualification limits.

## Financial and identity boundaries

TST and all investment assets/prices in this evaluation are synthetic. No live brokerage execution, banking/card network, Plaid credentials or regulatory membership is inferred from a template. Browser storage may retain UI preference or unanswered command keys, never authoritative funds, holdings, orders or approval. Existing OIDC/CSRF/current ownership controls remain active. Sensitive trades and card data require action-bound reauthentication; a cosmetic PIN screen cannot replace it. FaceID/TouchID and external banking providers require their actual platform/provider integrations before claiming availability.

Original roadmap status is kept separately from this additional UI scope. Completing template adoption does not silently certify production launch, real markets, legal protections, device hardware or latency targets.

## Accepted local CX checkpoint — 2026-11-16

The shared design, native personal-finance/brokerage foundation and the core synthetic journeys are implemented and accepted for local CX review. [Acceptance evidence](EVIDENCE.md) records the actual native/browser checks, schema-59/60/61 forward upgrades and persistent preview smoke. The schema-60 increment adds approved recurring-instruction revisions, execution summaries, trailing round-up totals and shared contribution breakdowns; schema 61 adds derived saving streaks, badges, milestone cards, monthly net worth and lifetime interest. Continue from the explicit remaining requirements in the [32-screen coverage](SCREEN_COVERAGE.md); the full production PRD is not marked complete.
