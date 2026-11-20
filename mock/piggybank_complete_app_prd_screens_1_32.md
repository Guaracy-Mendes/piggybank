# Product Requirements Document (PRD)

## PiggyBank — Complete Personal Finance & Wealth Ecosystem (Screens 1–32)

| Metadata | Details |
| :--- | :--- |
| **Document Status** | Final / Master Specification |
| **Product Name** | PiggyBank Mobile App (iOS & Android) |
| **Scope** | Complete Application UI (Phases 1–4 / Screens 1–32) |
| **Target Release** | v2.4 (Q4 2026) |
| **Author** | Product Management & Design Systems Team |
| **Design Reference** | `PiggyBank Master UI Architecture (Screens 1-32)` |

---

## 1. Executive Summary & Product Vision

### 1.1 Overview
**PiggyBank** is an end-to-end consumer fintech mobile application designed to bridge the gap between everyday banking, automated micro-savings, intelligent budgeting, and retail wealth management. This master PRD defines the complete UI architecture, component hierarchy, and interaction specifications across all **32 core screens**, structured into four progressive product phases:
* **Phase 1 (Screens 1–8):** Onboarding, KYC Identity Verification & Core Home Dashboard
* **Phase 2 (Screens 9–16):** Automated Savings Vaults, Round-Ups & Goal Tracking
* **Phase 3 (Screens 17–26):** Smart Budgeting, Expense Analytics, P2P Transfers & Virtual Cards
* **Phase 4 (Screens 27–32):** Investments & Retail Broker Suite

### 1.2 Core Business & User Objectives
* **Unified Financial Hub:** Consolidate checking, high-yield savings vaults, spending analytics, and self-directed investing into a single intuitive mobile interface.
* **Behavioral Nudges:** Leverage gamified streaks, spare-change round-ups, and automated Dollar-Cost Averaging (DCA) to build long-term financial resilience.
* **Institutional Trust with Consumer Warmth:** Pair approachable typography and clear visual hierarchy with bank-grade security, biometric authentication, and regulatory transparency.

---

## 2. Global Design System & UI Architecture

### 2.1 Visual Language & Tokens
* **Theme Parity:** Full support for Light Mode (default) and OLED Dark Mode.
* **Typography:** Geometric sans-serif (*Inter* / *SF Pro Display*) with `tabular-nums` enforced on all currency values, percentages, card numbers, and market prices.
* **Semantic Color Palette:**
  * **Brand Primary:** PiggyBank Coral (`#FF5A79`) & Deep Indigo (`#1E1B4B`)
  * **Positive / Bullish / Savings Growth:** Emerald (`#10B981` text / `#ECFDF5` surface)
  * **Negative / Bearish / Over-Budget Alert:** Crimson (`#EF4444` text / `#FEF2F2` surface)
  * **Warning / Pending:** Amber (`#F59E0B` text / `#FFFBEB` surface)
  * **Canvas & Cards:** Off-White background (`#F8FAFC`), Elevated White cards (`#FFFFFF`, `1px solid #E2E8F0`, `16px` radius).

### 2.2 Global Navigation Structure
* **Primary Bottom Tab Bar (5 Tabs — Persistent on Root Screens):**
  1. **Home** (Screen 08)
  2. **Save** (Screen 09)
  3. **Spend** (Screen 17)
  4. **Invest** (Screen 27)
  5. **Profile** (Screen 25)
* **Safe Areas:** Baseline viewport `375x812pt` scaling fluidly to `430x932pt` with strict adherence to OS status bar and home-indicator insets.

---

## 3. Phase 1: Onboarding, KYC & Core Home (Screens 1–8)

### Screen 01: Splash & Value Proposition Carousel
* **Purpose:** Introduce brand identity and core value pillars upon first launch.
* **Components:** Animated PiggyBank mascot/logo, 3-slide swipeable feature carousel (*Save Automatically*, *Spend Smarter*, *Invest Effortlessly*), pagination dots, primary `Get Started` CTA, and secondary `Log In` link.

### Screen 02: Account Registration & Credentials
* **Purpose:** Capture user email/phone and create secure login credentials.
* **Components:** Progress stepper (`Step 1 of 4`), Email input, Password input with real-time strength meter (8+ chars, symbol, uppercase), Referral code expandable field, Terms of Service checkbox, `Continue` CTA.

### Screen 03: Two-Factor Authentication (2FA / OTP)
* **Purpose:** Verify ownership of phone number or email address.
* **Components:** Masked destination display (`+1 (•••) •••-8921`), 6-digit auto-advancing OTP input boxes with SMS autofill support, 45-second countdown timer for `Resend Code`.

### Screen 04: KYC Identity Verification
* **Purpose:** Satisfy CIP/AML regulatory requirements for banking and brokerage accounts.
* **Components:** Legal First/Last Name, Date of Birth picker, Residential Address autocomplete, SSN/Tax ID masked input (`•••-••-1234`), Citizenship & Employment status dropdowns, encryption badge (`256-bit AES`).

### Screen 05: Biometric & Passcode Setup
* **Purpose:** Enable rapid, secure app unlocking and transaction authorization.
* **Components:** FaceID / TouchID illustration, `Enable Biometrics` primary CTA, fallback 6-digit app PIN setup flow, `Skip for Now` secondary option.

### Screen 06: Bank Linking (Plaid / Open Banking)
* **Purpose:** Connect external funding accounts for deposits, round-ups, and transfers.
* **Components:** Institution search bar, popular bank quick-select grid (Chase, Bank of America, Wells Fargo, Citi), OAuth WebView modal trigger, manual routing/account micro-deposit fallback link.

### Screen 07: Financial Goals & Personalization Quiz
* **Purpose:** Tailor the home dashboard and savings recommendations to user intent.
* **Components:** Multi-select pill cards (*Build an Emergency Fund*, *Pay Off Debt*, *Save for a Home*, *Grow Wealth via Stocks*, *Track Daily Spending*), `Personalize My App` CTA.

### Screen 08: Master Home Dashboard (Root Tab 1)
* **Purpose:** Provide an executive summary of total net worth across Cash, Savings, and Investments.
* **Components:**
  * **Header:** User greeting, Streak flame badge (`14-Day Saver`), Notification bell with unread dot.
  * **Net Worth Hero Card:** Combined balance (`$24,510.80`) with eye-icon privacy toggle (`••••••`), monthly delta pill (`▲ +$1,120.40 (+4.8%)`).
  * **Account Summary Horizontal Cards:** *Checking Cash* (`$3,440.30`), *Savings Vaults* (`$6,250.00`), *Investments* (`$14,820.50`).
  * **Quick Action Row:** `Add Money`, `Transfer`, `Round-Ups`, `Buy Stock`.
  * **Recent Activity Feed:** Unified chronological ledger of recent card swipes, vault deposits, and dividend payouts.

---

## 4. Phase 2: Automated Savings & Vaults (Screens 9–16)

### Screen 09: Savings Hub & Vaults Overview (Root Tab 2)
* **Purpose:** Central command for high-yield savings goals and automated saving rules.
* **Components:** Total Savings readout (`$6,250.00`), APY badge (`4.50% APY`), `Create New Vault` CTA button, grid/list of active **Goal Vaults** showing custom emoji/cover image, progress bar, target amount, and percentage funded.

### Screen 10: Create / Edit Savings Vault
* **Purpose:** Configure a dedicated goal-based savings sub-account.
* **Components:** Goal Name input (*Emergency Fund*, *Tokyo Trip*), Category icon/photo selector, Target Dollar Amount (`$10,000`), Target Date calendar picker, auto-calculated weekly/monthly savings suggestion banner.

### Screen 11: Vault Detail & Ledger
* **Purpose:** Track progress and manage funds for a specific savings goal.
* **Components:** Circular radial progress ring, Current Balance vs. Target Goal, `Deposit` & `Withdraw` action buttons, Vault Lock toggle (prevents impulsive withdrawals), vault-specific transaction history.

### Screen 12: Spare-Change Round-Ups Configuration
* **Purpose:** Automate micro-savings from everyday debit/credit card purchases.
* **Components:** Master Round-Ups toggle switch, Multiplier selector (`1x`, `2x`, `3x`, `5x` spare change), Linked Card selector, Destination Vault dropdown, Weekly Round-Up Cap slider, trailing 30-day round-up total card.

### Screen 13: Recurring Auto-Save Rules
* **Purpose:** Schedule automated payday or calendar-based transfers into vaults.
* **Components:** List of active auto-save rules, `+ Add Rule` modal trigger supporting *Payday Split (% of direct deposit)*, *Scheduled Recurring ($ amount)*, and *Guilty Pleasure Tax* (e.g., save `$5` every time user spends at coffee shops).

### Screen 14: High-Yield APY & Interest Tracker
* **Purpose:** Visualize compound interest earnings and APY boost status.
* **Components:** Lifetime Interest Earned hero figure, Current APY tier card, monthly interest payout bar chart, `Boost APY` referral/direct-deposit challenge banner.

### Screen 15: Shared / Group Savings Vault
* **Purpose:** Enable collaborative saving with partners, family, or friends.
* **Components:** Participant avatar stack with individual contribution progress bars, shared activity feed, `Invite Member` share sheet, group withdrawal permission settings (require mutual approval toggle).

### Screen 16: Savings Milestones & Gamification Badges
* **Purpose:** Reinforce positive saving habits through achievements and streaks.
* **Components:** Active saving streak calendar heatmap, unlocked vs. locked achievement badges (*First \$1K*, *100 Round-Ups*, *6-Month Streak*), shareable milestone card generator.

---

## 5. Phase 3: Smart Budgeting, Spend & Cards (Screens 17–26)

### Screen 17: Spend & Budgeting Overview (Root Tab 3)
* **Purpose:** Monitor monthly cash flow, safe-to-spend balance, and category budgets.
* **Components:** Monthly *Safe-to-Spend* gauge (`$1,420.00 left for 12 days`), Income vs. Spending comparison bar, Category Budget progress list (*Dining*, *Groceries*, *Transport*, *Shopping*, *Utilities*) with color-coded threshold alerts (`80%` amber, `100%+` red).

### Screen 18: Spending Analytics & Cash Flow Breakdown
* **Purpose:** Provide granular visual insights into spending habits over time.
* **Components:** Interactive Donut Chart by merchant category, Month-over-Month spending trend bar chart, *Top Merchants* leaderboard (by total spend and visit count), export CSV/PDF statement button.

### Screen 19: Category Budget Editor
* **Purpose:** Set or adjust monthly spending limits per category.
* **Components:** Category header with 3-month average spend benchmark, interactive slider + numeric input for monthly limit, Rollover unused budget toggle, push notification alert threshold selector.

### Screen 20: Transaction Detail & Receipt Split
* **Purpose:** Inspect individual merchant charges, attach receipts, and split bills.
* **Components:** Merchant logo, clean merchant name, GPS map snippet, timestamp, Category re-assignment pill, `Split Bill` CTA (routes to P2P split), Receipt camera upload button, `Report an Issue / Dispute` link.

### Screen 21: Recurring Bills & Subscription Manager
* **Purpose:** Detect, track, and cancel unwanted recurring subscriptions.
* **Components:** Total Monthly Subscriptions summary (`$184.90/mo`), upcoming bills calendar strip, list of detected subscriptions (*Netflix*, *Spotify*, *Gym*) with price-hike alert badges and `Cancel Subscription` concierge trigger.

### Screen 22: Peer-to-Peer (P2P) Send & Request Money
* **Purpose:** Enable instant fee-free transfers and bill splitting with contacts.
* **Components:** Searchable contacts & `@cashtag` bar, recent recipients avatar row, `Send` / `Request` toggle, custom numeric keypad, memo/emoji note input, `Split Equally` multi-contact selector.

### Screen 23: PiggyBank Debit & Virtual Cards Hub
* **Purpose:** Manage physical debit cards and instant disposable virtual cards.
* **Components:** Interactive 3D-tilt card preview with tap-to-reveal Card Number, CVV, and Expiry; quick toggles for `Freeze Card`, `Apple Pay / Google Pay`, `Online Transactions`, and `ATM Withdrawals`; `+ Create Virtual Card` CTA for secure online shopping.

### Screen 24: Add Money / Deposit & Withdrawal Hub
* **Purpose:** Move funds into or out of the PiggyBank ecosystem.
* **Components:** Amount input, `From` and `To` account selectors, Transfer Speed selector (*Instant Debit Transfer — 1.5% fee* vs. *Standard ACH 1–3 Business Days — Free*), Direct Deposit routing/account copy card.

### Screen 25: Profile, Security & Account Settings (Root Tab 5)
* **Purpose:** Manage personal details, security preferences, tax documents, and support.
* **Components:** User profile header with KYC verification badge, `Linked Banks`, `Security & Biometrics`, `Notifications`, `Tax Documents & Statements (1099 / Monthly)`, `Help & Live Chat`, and `Log Out`.

### Screen 26: Notification Center & Smart Alerts
* **Purpose:** Surface actionable financial alerts, trade fills, and budget warnings.
* **Components:** Filter tabs (`All`, `Transactions`, `Savings`, `Investments`), grouped chronological alert cards with deep-link navigation to relevant screens, `Mark All Read` action.

---

## 6. Phase 4: Investments & Broker Suite (Screens 27–32)

### Screen 27: Investments Hub & Portfolio Dashboard (Root Tab 4)
* **Purpose:** Serve as the primary landing screen for the wealth suite, providing an immediate snapshot of portfolio health, total balance, and market discovery entry points.
* **Components:**
  * **Header Bar:** Title *"Investments"*, Brokerage Account Status pill (`Individual •••• 4829`), Search icon, Notification bell.
  * **Portfolio Hero Card:** Total Portfolio Value (`$14,820.50`), Net Return Pill (`▲ +$342.18 (+2.36%) Today`), touch-scrubbable interactive sparkline chart with haptic feedback, Timeframe Selector Pills (`1D`, `1W`, `1M`, `3M`, `1Y`, `ALL`).
  * **Buying Power Banner:** Available uninvested cash (`Buying Power: $1,250.00`) + `+ Deposit` CTA.
  * **Asset Allocation & Holdings Preview:** Horizontal stacked allocation bar (Stocks, ETFs, Crypto/Bonds), scrollable list of **Your Holdings** (Logo, Ticker, Name, Shares, Mini Sparkline, Equity Value, `% Gain/Loss`), and footer link *"View Full Portfolio Analytics →"* (routes to **Screen 31**).
  * **Market Movers Carousel:** Horizontal cards for *Top Gainers*, *Popular ETFs*, and *Sector Collections* (routes to **Screen 28**).
* **States:** New-investor empty state (*"Start investing with as little as \$1"*) and Pre-Market / After-Hours status badge.

### Screen 28: Asset Detail & Market Research
* **Purpose:** Equip users with real-time pricing, historical charts, fundamental metrics, and analyst ratings for a specific security.
* **Components:**
  * **Sticky Asset Header:** Back button, Ticker (`AAPL`), Company Name (`Apple Inc.`), Watchlist Toggle star.
  * **Price & Interactive Chart Module:** Live tabular price (`$228.45`), live pulse indicator, Dollar/Percent delta (`+$4.12 (+1.84%)`), Line vs. OHLC Candlestick toggle, Timeframe Bar (`LIVE`, `1D`, `1W`, `1M`, `1Y`, `5Y`).
  * **User Position Summary Card (Conditional):** 2x2 Grid (*Shares Owned*, *Market Value*, *Average Cost*, *Total Return \$ / %*).
  * **Key Statistics Grid:** `Market Cap`, `P/E Ratio`, `Dividend Yield`, `52-Week High`, `52-Week Low`, `Volume`.
  * **About & Analyst Ratings:** Expandable company bio and visual Buy/Hold/Sell consensus bar.
  * **Sticky Bottom Dock:** Dual `Sell` / `Buy` buttons (if owned) or full-width `Buy [TICKER]` button (routes to **Screen 29**).

### Screen 29: Order Execution Ticket (Buy / Sell)
* **Purpose:** Provide a streamlined, error-proof interface for configuring trade parameters.
* **Components:**
  * **Top Bar & Order Type Selector:** Close (`X`), Title (`Buy AAPL`), Order Type dropdown (`Market Order`, `Limit Order`, `Recurring Investment` → routes to **Screen 32**).
  * **Investment Mode Switcher:** Toggle between **Dollars (\$)** (fractional shares) and **Shares (#)**.
  * **Hero Amount Display:** Large numeric readout, real-time share/cost conversion subtitle (`Est. 0.4377 shares @ $228.45`), and available balance context.
  * **Quick-Select Preset Chips:** `$25`, `$100`, `$500`, `Max`.
  * **Limit Order Conditional Fields:** *Target Limit Price* input + *Time in Force* (`Good for Day` vs. `GTC`).
  * **Custom Numeric Keypad & Review CTA:** 3x4 tactile keypad and `Review Order` button (routes to **Screen 30**).
* **Guardrails:** Inline insufficient-funds validation and Pattern Day Trader (PDT) / Good Faith Violation warnings.

### Screen 30: Order Review & Confirmation Receipt
* **Purpose:** Satisfy regulatory transparency prior to execution and deliver an unambiguous digital trade receipt upon submission.
* **Components (Two-State Flow):**
  * **State A (Pre-Execution Review):** Action badge (`BUY`/`SELL`), Asset Ticker, Line-Item Breakdown (*Order Type*, *Estimated Price*, *Estimated Shares*, *Commission/Regulatory Fees*, *Estimated Total*), FINRA/SEC execution disclosure, and *Swipe-to-Submit* / `Submit Order` CTA.
  * **State B (Post-Execution Confirmation):** Animated emerald checkmark, Status Headline (*"Order Placed"* / *"Order Filled"*), timestamp, Receipt Summary Card (filled quantity, execution price, reference ID `#PB-9948210`, updated buying power), `Done` CTA (returns to **Screen 27**), and `Set Up Recurring Buy` CTA (routes to **Screen 32**).

### Screen 31: Portfolio Analytics & Asset Allocation
* **Purpose:** Deliver deep visibility into portfolio diversification, sector exposure, benchmark performance, and dividend income.
* **Components:**
  * **Allocation Donut Chart Card:** Interactive segmented ring chart with toggle tabs for **Asset Class**, **Sector**, and **Individual Holdings**, paired with an interactive percentage/dollar legend.
  * **Performance Comparison Benchmark:** Multi-line chart comparing `PiggyBank Portfolio` vs. `S&P 500` and `NASDAQ` across `1M`, `3M`, `YTD`, `1Y`, `ALL`.
  * **Income & Dividends Card:** *Annualized Projected Dividend Income*, *Next Scheduled Payout*, and trailing 6-month dividend payout bar chart.
  * **Top Performers & Laggards:** Side-by-side comparison cards for highest-gaining and lowest-performing holdings.

### Screen 32: Recurring Investments (Auto-Invest) Management
* **Purpose:** Enable automated Dollar-Cost Averaging (DCA) by allowing users to create, pause, edit, or cancel scheduled recurring purchases.
* **Components:**
  * **Auto-Invest Summary Banner:** Total monthly automated commitment (`$450.00 / month across 3 schedules`) and next execution date.
  * **Active Schedules List:** Cards displaying Asset Logo/Ticker, Amount & Frequency (`$50.00 • Every Monday`), Funding Source (`PiggyBank Cash` or `Linked Bank`), Next Execution Date, Total Invested-to-Date, and `Active` / `Paused` toggle.
  * **Create / Edit Schedule Bottom Sheet:** Searchable Asset Selector, Dollar Amount ($\ge \$5.00$), Frequency Pills (`Daily`, `Weekly`, `Bi-Weekly`, `Monthly`), Contextual Day/Date Picker, Funding Source Selector, and `Confirm Schedule` CTA.

---

## 7. Master Navigation & Architecture Map

```
[01 Splash] ─> [02 Register] ─> [03 2FA] ─> [04 KYC] ─> [05 Biometrics] ─> [06 Bank Link] ─> [07 Quiz]
                                                                                                 │
┌────────────────────────────────────────────────────────────────────────────────────────────────┘
▼
[08 Home Dashboard] <───> [24 Deposit/Withdraw] / [26 Notifications]
 ├── Tab 2: [09 Savings Hub] ──> [10 Create Vault] / [11 Vault Detail] / [12 Round-Ups] / [13 Auto-Save] / [14 APY] / [15 Shared Vault] / [16 Milestones]
 ├── Tab 3: [17 Spend & Budget] ─> [18 Analytics] / [19 Budget Edit] / [20 Tx Detail] / [21 Subscriptions] / [22 P2P] / [23 Cards]
 ├── Tab 4: [27 Investments Hub] ─> [28 Asset Detail] ─> [29 Order Ticket] ─> [30 Confirmation]
 │                                   └──> [31 Portfolio Analytics] / [32 Recurring Investments]
 └── Tab 5: [25 Profile & Settings]
```

---

## 8. Non-Functional, Security & Compliance Requirements

1. **Real-Time Market & Ledger Latency:**
   * Live stock/ETF quotes (Screens 27–29) must stream via WebSocket with $< 500\text{ms}$ latency during market hours (`9:30 AM – 4:00 PM EST`).
   * Card authorization push notifications (Screen 26) and ledger updates (Screens 08, 17, 20) must reflect within $< 2\text{ seconds}$ of authorization.
2. **Chart Frame Rate:** Interactive touch-scrubbing charts (Screens 08, 18, 27, 28, 31) must maintain $\ge 60\text{ FPS}$.
3. **Biometric & Step-Up Authentication:** FaceID/TouchID or 6-digit PIN is mandatory before revealing virtual card PAN/CVV (Screen 23), initiating external P2P/ACH withdrawals (Screens 22, 24), or submitting brokerage orders (Screen 30).
4. **Regulatory Disclosures & Accessibility (WCAG 2.1 AA):**
   * FDIC pass-through insurance disclosures must appear on Cash/Savings screens (Screens 08–15).
   * SIPC & FINRA/SEC risk disclosures must appear on all Investment screens (Screens 27–32).
   * All positive/negative financial deltas must pair color with explicit directional icons (`▲`/`▼`) and `+`/`-` signs.