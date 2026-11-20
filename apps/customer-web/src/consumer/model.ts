export type AccountCash = {
  id: string;
  state: string;
  bookedMinor: string;
  heldMinor: string;
  availableMinor: string;
};
export type Vault = {
  id: string;
  name: string;
  targetMinor: string;
  targetDate: string | null;
  icon: string;
  locked: boolean;
  mutual: boolean;
  bookedMinor: string;
  heldMinor: string;
  membership: string;
  isOwner: boolean;
  members: number;
};
export type Card = {
  id: string;
  accountId: string;
  label: string;
  lastFour: string;
  frozen: boolean;
  onlineEnabled: boolean;
  atmEnabled: boolean;
  network: string;
};
export type Activity = {
  batchId: string;
  accountId: string;
  kind: string;
  date: string;
  at: string;
  deltaMinor: string;
  category: string;
  note: string;
  receipt?: { mime: string; content: string };
  originalBatchId?: string;
};
export type NetWorthPoint = {
  at: string;
  month?: string;
  bankMinor: string;
  vaultMinor: string;
  investmentMinor: string;
  totalMinor: string;
  complete: boolean;
  missingPrices: string[];
};
export type Badge = {
  code: string;
  title: string;
  description: string;
  unit: "COUNT" | "DAYS" | "MONTHS" | "MINOR";
  progress: string;
  target: string;
  unlocked: boolean;
  unlockedOn: string | null;
};
export type ProgressReport = {
  asOf: string;
  netWorth: {
    current: NetWorthPoint;
    monthStart: NetWorthPoint;
    monthStartDate: string;
    deltaMinor: string | null;
    deltaBasisPoints: string | null;
    history: NetWorthPoint[];
  };
  savings: {
    currentDays: number;
    longestDays: number;
    currentMonths: number;
    longestMonths: number;
    savingDays: number;
    lastSavingDate: string | null;
    contributions: number;
    lifetimeContributedMinor: string;
    heatmapFrom: string;
    heatmap: { date: string; amountMinor: string; count: number }[];
  };
  badges: Badge[];
  interest: {
    lifetimeMinor: string;
    postings: number;
    lastPostedOn: string | null;
    months: { month: string; amountMinor: string }[];
  };
};
export type Workspace = {
  progress?: ProgressReport;
  vaultContributors?: {
    vaultId: string;
    label: string;
    isYou: boolean;
    contributedMinor: string;
  }[];
  vaultActivity?: (Activity & { vaultId: string })[];
  analytics: Analytics;
  preferences: {
    theme?: string;
    hideBalance?: boolean;
    goals?: string[];
    riskLevel?: number;
    notifications?: boolean;
    marketing?: boolean;
  };
  accounts: AccountCash[];
  activity: Activity[];
  vaults: Vault[];
  cards: Card[];
  budgets: {
    category: string;
    limitMinor: string;
    rollover: boolean;
    threshold: number;
  }[];
  links: {
    id: string;
    institution: string;
    label: string;
    lastFour: string;
    state: string;
  }[];
  bills: {
    id: string;
    merchant: string;
    amountMinor: string;
    nextDate: string;
    state: string;
  }[];
  notifications: {
    id: string;
    category: string;
    title: string;
    body: string;
    route: string;
    readAt: string | null;
    createdAt: string;
  }[];
  tokens: {
    id: string;
    label: string;
    scope: string;
    expiresAt: string;
    revokedAt: string | null;
  }[];
  requests: {
    id: string;
    accountId: string;
    reference: string;
    amountMinor: string;
    memo: string;
    state: string;
    incoming: boolean;
  }[];
  groupWithdrawals: {
    id: string;
    vaultId: string;
    amountMinor: string;
    state: string;
    isRequester: boolean;
    approved: boolean;
  }[];
  roundups?: {
    cardId: string;
    vaultId: string;
    multiplier: number;
    weeklyCapMinor: string;
    enabled: boolean;
    savedMinor: string;
    saved30DaysMinor?: string;
    roundupCount?: number;
  };
  statements: {
    id: string;
    accountId: string;
    from: string;
    to: string;
    digest: string;
    createdAt: string;
  }[];
  favorites?: { reference: string; label: string }[];
  vaultPhotos?: Record<string, string>;
  splits?: {
    id: string;
    batchId: string;
    totalMinor: string;
    selfShareMinor: string;
    participants: number;
    memo: string;
    paid: number;
    requests: { id: string; amountMinor: string; state: string }[];
  }[];
  requestLinks?: Record<
    string,
    { splitId: string | null; paymentId: string | null }
  >;
  saveRules?: {
    id: string;
    cardId: string;
    vaultId: string;
    matchKind: "MERCHANT" | "CATEGORY";
    matchValue: string;
    amountMinor: string;
    weeklyCapMinor: string;
    enabled: boolean;
    savedThisWeekMinor: string;
    savedLifetimeMinor: string;
  }[];
  safeToSpend?: {
    safeToSpendMinor: string;
    availableMinor: string;
    automationsDueMinor: string;
    billsDueMinor: string;
    through: string;
  };
};
export type Asset = {
  symbol: string;
  name: string;
  class: string;
  sector: string;
  priceMinor: string;
  dividendBps: number;
  description: string;
  watched: boolean;
  drip: boolean;
  alertMinor: string | null;
};
export type Position = {
  symbol: string;
  units: string;
  reservedUnits: string;
  costMinor: string;
  realizedMinor: string;
  valueMinor: string;
  returnMinor: string;
};
export type Order = {
  id: string;
  symbol: string;
  side: string;
  orderType: string;
  units: string;
  limitMinor: string;
  state: string;
  createdAt: string;
  expiresAt: string;
  fill: null | {
    batchId: string;
    priceMinor: string;
    amountMinor: string;
    filledAt: string;
  };
};
export type Wealth = {
  priceHistory?: { symbol: string; at: string; priceMinor: string }[];
  realizedMinor?: string;
  upcomingDividends?: {
    symbol: string;
    recordAt: string;
    payAt: string;
    perShareMinor: string;
  }[];
  notice: string;
  dataSource: string;
  quoteObservedAt: string;
  account: null | {
    id: string;
    state: string;
    cashMinor: string;
    heldMinor: string;
    buyingPowerMinor: string;
  };
  assets: Asset[];
  positions: Position[];
  orders: Order[];
  observations: {
    at: string;
    cashMinor: string;
    equityMinor: string;
    netFlowsMinor: string;
  }[];
  automationSummary?: {
    activeCount: number;
    nextDueDate: string | null;
    saveMonthlyMinor: string;
    investMonthlyMinor: string;
    totalSavedMinor: string;
    totalInvestedMinor: string;
  };
  schedules: {
    id: string;
    kind: string;
    accountId: string;
    vaultId: string | null;
    symbol: string | null;
    amountMinor: string;
    frequency: string;
    nextRun: string;
    state: string;
    lastError: string;
    familyId?: string;
    anchorDay?: number;
    replacesId?: string | null;
    replacedBy?: string | null;
    completedRuns?: number;
    executedMinor?: string;
    familyCompletedRuns?: number;
    familyExecutedMinor?: string;
    lastCompletedDate?: string | null;
    recentRuns?: {
      dueDate: string;
      batchId: string;
      executedMinor: string;
      postedAt: string;
    }[];
  }[];
  dividends: {
    id: string;
    symbol: string;
    recordDate: string;
    amountMinor: string;
    batchId: string;
  }[];
};
export type Review = {
  receipt?: Record<string, unknown> | null;
  id: string;
  action: string;
  details: Record<string, unknown>;
  digest: string;
  expiresAt: string;
  approvalUrl: string;
  approvalReady: boolean;
};
export const categories = [
  "Dining",
  "Groceries",
  "Transport",
  "Shopping",
  "Utilities",
  "Health",
  "Travel",
  "Other",
];
export function minor(s: string, scale = 2) {
  if (!new RegExp("^\\d{1,16}(\\.\\d{1," + scale + "})?$").test(s))
    throw new Error(
      "Enter a positive exact amount with at most " + scale + " decimals.",
    );
  const [whole, part = ""] = s.split(".");
  return (
    BigInt(whole) * 10n ** BigInt(scale) +
    BigInt(part.padEnd(scale, "0"))
  ).toString();
}
export function amount(n: string | bigint) {
  const b = BigInt(n),
    a = b < 0n ? -b : b;
  return `${b < 0n ? "−" : ""}TST ${(a / 100n).toLocaleString("en-US")}.${(a % 100n).toString().padStart(2, "0")}`;
}
export function shares(s: string) {
  const n = BigInt(s);
  return `${n / 1000000n}.${(n % 1000000n).toString().padStart(6, "0").replace(/0+$/, "") || "0"}`;
}
export function ratio(n: bigint, d: bigint) {
  return d === 0n ? 0 : Number((n * 10000n) / d) / 100;
}
export const text = (f: FormData, name: string) => String(f.get(name) ?? "");
export type Analytics = {
  from: string;
  to: string;
  incomeMinor: string;
  spentMinor: string;
  categories: {
    category: string;
    spentMinor: string;
    averageMinor: string;
    budgetMinor: string | null;
  }[];
  merchants: { name: string; spentMinor: string; visits: number }[];
};

export type Schedule = Wealth["schedules"][number];
export function decimal(n: string | bigint, scale = 2) {
  const b = BigInt(n),
    base = 10n ** BigInt(scale);
  return (
    (b / base).toString() + "." + (b % base).toString().padStart(scale, "0")
  );
}
