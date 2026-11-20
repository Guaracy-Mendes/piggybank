import { useState, type ReactNode } from "react";
import { Alert, message, useSession, value } from "@piggybank/web-core";
import { useConsumer } from "./context";
import { Box, Head, Progress, Allocation } from "./components";
import {
  amount,
  minor,
  text,
  categories,
  type Activity,
  type Vault,
  decimal,
} from "./model";
import { navigate } from "../lib/navigation";
import { encodePaymentCode } from "../lib/paymentCode";
import { QrCode } from "../components/QrCode";
import {
  InterestSummary,
  Milestones,
  MonthlyDelta,
  NetWorthHistory,
  StreakBadge,
} from "./Milestones";
import { Messages } from "./Messages";

export function AccountSelect({ name = "accountId" }: { name?: string }) {
  const c = useConsumer();
  return (
    <label>
      Bank account
      <select name={name} required>
        {c.data.accounts
          .filter((a) => a.state === "ACTIVE")
          .map((a) => (
            <option key={a.id} value={a.id}>
              •••• {a.id.slice(-4)} · {amount(a.availableMinor)} available
            </option>
          ))}
      </select>
    </label>
  );
}
function CommandForm({
  action,
  build,
  children,
  label = "Save",
  after,
}: {
  action: string;
  build: (f: FormData) => Record<string, unknown>;
  children: ReactNode;
  label?: string;
  after?: (v: Record<string, unknown>) => void;
}) {
  const c = useConsumer(),
    [error, setError] = useState("");
  return (
    <form
      className="ui-form"
      onSubmit={(e) => {
        e.preventDefault();
        const form = e.currentTarget;
        setError("");
        void Promise.resolve()
          .then(() => build(new FormData(form)))
          .then((input) => c.command(action, input))
          .then((v) => {
            c.notify("Your instruction was saved.");
            after?.(v);
          })
          .catch((e) => setError(message(e)));
      }}
    >
      {children}
      {error && <Alert>{error}</Alert>}
      <button disabled={c.busy}>{label}</button>
    </form>
  );
}
export function Home({ name }: { name?: string }) {
  const c = useConsumer(),
    d = c.data,
    w = c.wealth;
  const bank = d.accounts.reduce((n, a) => n + BigInt(a.bookedMinor), 0n),
    saved = d.vaults
      .filter((v) => v.isOwner && v.members === 1)
      .reduce((n, v) => n + BigInt(v.bookedMinor), 0n),
    invest = w.positions.reduce(
      (n, p) => n + BigInt(p.valueMinor),
      BigInt(w.account?.cashMinor ?? "0"),
    ),
    total = bank + saved + invest;
  return (
    <>
      <Head
        title={`Good to see you${name ? ", " + name : ""}.`}
        sub="A clear view of your money and what you’re building."
      >
        <StreakBadge p={d.progress} />
        <button className="secondary" onClick={() => void c.refresh()}>
          Refresh
        </button>
        <button
          className="secondary"
          onClick={() =>
            void c
              .command("PREFERENCES", {
                hideBalance: !d.preferences.hideBalance,
              })
              .catch(() => {})
          }
        >
          {d.preferences.hideBalance ? "Show" : "Hide"} net worth
        </button>
      </Head>
      <div className="ui-grid">
        <Box className="ui-hero">
          <p className="ui-kicker">TOTAL PERSONAL NET WORTH</p>
          <div className="ui-number">
            {d.preferences.hideBalance ? "••••••" : amount(total)}
          </div>
          <MonthlyDelta p={d.progress} hidden={d.preferences.hideBalance} />
          <p>
            Banking, personal savings and synthetic investments in one place.
          </p>
          <div className="ui-actions">
            <a className="ui-pill" href="#/wallet">
              Add money ↗
            </a>
            <a className="ui-pill" href="#/payments">
              Send money →
            </a>
            <a className="ui-pill" href="#/save/new">
              Create a vault +
            </a>
          </div>
        </Box>
        <Box title="Your allocation">
          <Allocation
            label="Net worth"
            items={[
              { name: "Banking", value: bank },
              { name: "Personal vaults", value: saved },
              { name: "Investments", value: invest },
            ]}
          />
        </Box>
        <Box title="Save for what matters">
          <div className="spread">
            <strong>{amount(saved)}</strong>
            <a href="#/save">All vaults →</a>
          </div>
          {d.vaults
            .filter((v) => v.membership === "ACTIVE")
            .slice(0, 3)
            .map((v) => (
              <div className="ui-mini-card" key={v.id}>
                <a href={"#/save/" + v.id}>
                  {v.icon} {v.name}
                </a>
                <p>
                  {amount(v.bookedMinor)} of {amount(v.targetMinor)}
                </p>
                <Progress value={v.bookedMinor} max={v.targetMinor} />
              </div>
            ))}
          {!d.vaults.length && (
            <p className="ui-caption">
              Name a goal and start saving toward it.
            </p>
          )}
          <p className="ui-caption">
            Shared pools are shown in Savings; no arbitrary share is included in
            your personal net worth.
          </p>
        </Box>
        <Box title="This month’s spending">
          <div className="ui-number">{amount(d.analytics.spentMinor)}</div>
          <p>
            Income {amount(d.analytics.incomeMinor)} · {d.analytics.from} to{" "}
            {d.analytics.to} (exclusive)
          </p>
          <a href="#/spend">Explore budgets & insights →</a>
          <div className="ui-mini-card">
            <strong>Ready for your next chapter?</strong>
            <p>
              Build an investment portfolio with fractional synthetic shares.
            </p>
            <a href="#/invest">Explore investing →</a>
          </div>
        </Box>
        <Box title="Net worth by month">
          <NetWorthHistory p={d.progress} hidden={d.preferences.hideBalance} />
        </Box>
        <Box className="ui-wide" title="Recent activity">
          <ActivityList items={d.activity.slice(0, 5)} />
          <a href="#/spend/activity">View all transactions →</a>
        </Box>
      </div>
    </>
  );
}
export function Savings({ route }: { route: string }) {
  const c = useConsumer(),
    id = route.split("/")[2],
    v = c.data.vaults.find((v) => v.id === id),
    [error, setError] = useState("");
  if (id === "milestones") return <Milestones />;
  if (id === "new")
    return (
      <>
        <Head
          title="Create a savings vault"
          sub="A dedicated native balance for a goal that matters."
        />
        <Box>
          <VaultEditor />
        </Box>
      </>
    );
  if (v)
    return (
      <>
        <Head
          title={v.icon + " " + v.name}
          sub={`${v.members} active member(s) · ${v.locked ? "Locked" : "Unlocked"} · ${v.mutual ? "Unanimous shared withdrawals" : "Personal withdrawals"}`}
        />
        {error && <Alert>{error}</Alert>}
        {v.membership === "INVITED" ? (
          <Box title="Your invitation">
            <p>
              Join this savings pool to contribute and approve shared
              withdrawals.
            </p>
            <button
              disabled={c.busy}
              onClick={() =>
                void c
                  .command("VAULT_JOIN", { id: v.id })
                  .catch((e) => setError(message(e)))
              }
            >
              Accept invitation
            </button>
          </Box>
        ) : (
          <>
            <div className="ui-grid two">
              <Box>
                {c.data.vaultPhotos?.[v.id] && (
                  <img
                    className="ui-vault-photo"
                    src={c.data.vaultPhotos[v.id]}
                    alt={"Goal photo for " + v.name}
                  />
                )}
                <p className="ui-kicker">VAULT BALANCE</p>
                <div className="ui-number">{amount(v.bookedMinor)}</div>
                <p>
                  {amount(v.heldMinor)} reserved · Target{" "}
                  {amount(v.targetMinor)}
                  {v.targetDate ? " by " + v.targetDate : ""}
                </p>
                <Progress value={v.bookedMinor} max={v.targetMinor} />
                <VaultTransfer vault={v} />
              </Box>
              <Box title="Vault settings">
                {v.isOwner ? (
                  <>
                    <VaultEditor vault={v} />
                    <VaultPhoto vault={v} />
                    <button
                      className="secondary"
                      onClick={() =>
                        void c
                          .command("VAULT_UPDATE", {
                            id: v.id,
                            locked: !v.locked,
                          })
                          .catch((e) => setError(message(e)))
                      }
                    >
                      {v.locked ? "Unlock" : "Lock"} vault
                    </button>
                    <h3>Invite a contributor</h3>
                    <CommandForm
                      action="VAULT_INVITE"
                      build={(f) => ({
                        id: v.id,
                        receivingReference: text(f, "reference"),
                      })}
                      label="Invite member"
                    >
                      <label>
                        Their receiving reference
                        <input name="reference" required maxLength={100} />
                      </label>
                      <p className="ui-caption">
                        Shared withdrawals require every active member’s
                        approval.
                      </p>
                    </CommandForm>
                  </>
                ) : (
                  <p>
                    The owner manages this vault. You can contribute and approve
                    shared withdrawals.
                  </p>
                )}
              </Box>
            </div>
            {(c.data.vaultContributors ?? []).some(
              (x) => x.vaultId === v.id,
            ) && (
              <Box title="Member contributions">
                <p className="ui-caption">
                  Lifetime attributable posted contributions; withdrawals change
                  the pool balance, not these historical totals.
                </p>
                {c.data.vaultContributors
                  ?.filter((x) => x.vaultId === v.id)
                  .map((x) => (
                    <div className="ui-mini-card" key={x.label}>
                      <div className="spread">
                        <strong>{x.label}</strong>
                        <span>{amount(x.contributedMinor)}</span>
                      </div>
                      <Progress
                        value={x.contributedMinor}
                        max={v.targetMinor}
                        label={x.label + " contribution to vault target"}
                      />
                    </div>
                  ))}
              </Box>
            )}
            <GroupRequests vault={v} />
            <Box title="Vault ledger">
              <p className="ui-caption">
                Native vault-side contributions and withdrawals from its most
                recent posted movements. The vault balance includes every native
                entry.
              </p>
              <ActivityList
                items={(c.data.vaultActivity ?? []).filter(
                  (a) => a.vaultId === v.id,
                )}
              />
            </Box>
          </>
        )}
        <a href="#/save">← Back to Savings</a>
      </>
    );
  return (
    <>
      <Head title="Savings Hub" sub="Small habits. Meaningful milestones.">
        <a className="text-link" href="#/save/new">
          + New vault
        </a>
      </Head>
      <div className="ui-grid">
        <Box className="ui-hero">
          <p className="ui-kicker">YOUR PERSONAL SAVINGS</p>
          <div className="ui-number">
            {amount(
              c.data.vaults
                .filter((v) => v.isOwner && v.members === 1)
                .reduce((n, v) => n + BigInt(v.bookedMinor), 0n),
            )}
          </div>
          <p>Shared balances are listed separately in their pool cards.</p>
          <a className="ui-pill" href="#/invest/automate">
            Set up automatic savings →
          </a>
        </Box>
        <Box title="Savings achievements">
          {c.data.progress && (
            <>
              <p>
                <StreakBadge p={c.data.progress} />
              </p>
              <p>
                {c.data.progress.badges.filter((b) => b.unlocked).length} of{" "}
                {c.data.progress.badges.length} badges unlocked · longest streak{" "}
                {c.data.progress.savings.longestDays} day(s)
              </p>
            </>
          )}
          <p>
            {
              c.data.vaults.filter(
                (v) => BigInt(v.bookedMinor) >= BigInt(v.targetMinor),
              ).length
            }{" "}
            goals reached
          </p>
          <p>
            {c.data.vaults.filter((v) => BigInt(v.bookedMinor) > 0n).length}{" "}
            vaults funded
          </p>
          <p>
            {amount(c.data.roundups?.savedMinor ?? "0")} saved with round-ups
          </p>
          <p className="ui-caption">
            Achievements reflect actual balances and saving receipts.
          </p>
          <a href="#/save/milestones">View streaks & badges →</a>
        </Box>
      </div>
      <div className="ui-grid">
        {c.data.vaults.map((v) => (
          <Box key={v.id}>
            <div className="spread">
              <span className="ui-asset-mark">{v.icon}</span>
              <span className="ui-pill">
                {v.membership === "INVITED"
                  ? "Invitation"
                  : v.members > 1
                    ? "Shared pool"
                    : v.locked
                      ? "Locked"
                      : "Personal"}
              </span>
            </div>
            <h2>
              <a href={"#/save/" + v.id}>{v.name}</a>
            </h2>
            <div className="ui-number">{amount(v.bookedMinor)}</div>
            <Progress value={v.bookedMinor} max={v.targetMinor} />
            <p className="ui-caption">
              Goal {amount(v.targetMinor)}
              {v.targetDate ? " · " + v.targetDate : ""}
            </p>
          </Box>
        ))}
      </div>
      {!c.data.vaults.length && (
        <Box title="Make room for your plans">
          <p>Create a vault for travel, a rainy day or your next milestone.</p>
          <a href="#/save/new">Create your first vault →</a>
        </Box>
      )}
      <Box title="Spare change, bigger possibilities">
        <Roundups />
      </Box>
      <Box title="Merchant saving rules">
        <SaveRules />
      </Box>
      <Box title="Interest & disclosures">
        <InterestSummary p={c.data.progress} />
        <p>
          Interest is credited only under an activated financial offer with
          retained consent and an actual native periodic receipt. A savings goal
          does not automatically promise a rate.
        </p>
        <a href="#/accounts">
          Review your account offers & interest receipts →
        </a>
      </Box>
    </>
  );
}
function VaultEditor({ vault: v }: { vault?: Vault }) {
  const [target, setTarget] = useState(v ? decimal(v.targetMinor) : ""),
    [date, setDate] = useState(v?.targetDate ?? "");
  let guidance = "";
  try {
    const goal = BigInt(minor(target)),
      balance = BigInt(v?.bookedMinor ?? "0"),
      left = goal > balance ? goal - balance : 0n;
    const today = new Date().toISOString().slice(0, 10),
      days = Math.ceil(
        (Date.parse(date + "T00:00:00Z") - Date.parse(today + "T00:00:00Z")) /
          86400000,
      );
    if (left === 0n)
      guidance = "Your current balance already reaches this target.";
    else if (Number.isFinite(days) && days === 0)
      guidance = "Save " + amount(left) + " today to reach this target.";
    else if (Number.isFinite(days) && days > 0) {
      const horizon = BigInt(days),
        weekly = (left * 7n + horizon - 1n) / horizon,
        monthly = (left * 30n + horizon - 1n) / horizon;
      guidance =
        "About " +
        amount(weekly < left ? weekly : left) +
        " per week or " +
        amount(monthly < left ? monthly : left) +
        " per 30-day month to reach this goal.";
    }
  } catch {
    /* The submit path reports invalid exact monetary input. */
  }
  return (
    <CommandForm
      action={v ? "VAULT_UPDATE" : "VAULT_CREATE"}
      build={(f) => ({
        ...(v ? { id: v.id } : {}),
        name: text(f, "name"),
        targetMinor: minor(text(f, "target")),
        targetDate: text(f, "date") || null,
        ...(!v ? { icon: text(f, "icon") } : {}),
      })}
      label={v ? "Save vault settings" : "Create vault"}
      after={(r) => {
        if (!v) navigate("/save/" + r.id);
      }}
    >
      <label>
        Vault name
        <input name="name" defaultValue={v?.name} required maxLength={64} />
      </label>
      <div className="ui-two">
        <label>
          Target amount (TST)
          <input
            name="target"
            inputMode="decimal"
            value={target}
            onChange={(e) => setTarget(e.target.value)}
            required
          />
        </label>
        <label>
          Target date
          <input
            name="date"
            type="date"
            value={date}
            onChange={(e) => setDate(e.target.value)}
          />
        </label>
      </div>
      {guidance && (
        <p className="ui-safe-notice" role="status">
          {guidance}
          <br />
          <small>
            Based on the remaining goal and days to deadline, rounded up;
            excludes interest and future returns.
          </small>
        </p>
      )}
      {!v && (
        <label>
          Vault icon
          <select name="icon">
            {["✦", "☀", "⌂", "✈", "♥", "♧"].map((x) => (
              <option key={x}>{x}</option>
            ))}
          </select>
        </label>
      )}
    </CommandForm>
  );
}
function VaultPhoto({ vault: v }: { vault: Vault }) {
  const c = useConsumer(),
    [error, setError] = useState(""),
    has = Boolean(c.data.vaultPhotos?.[v.id]);
  return (
    <div className="ui-form">
      <label>
        Goal photo (PNG or JPEG, up to 32 KiB)
        <input
          type="file"
          accept="image/png,image/jpeg"
          onChange={(e) => {
            const f = e.target.files?.[0];
            setError("");
            if (!f) return;
            if (
              !["image/png", "image/jpeg"].includes(f.type) ||
              f.size > 32768
            ) {
              setError("Choose a PNG or JPEG of at most 32 KiB.");
              return;
            }
            const reader = new FileReader();
            reader.onload = () =>
              void c
                .command("VAULT_PHOTO", {
                  id: v.id,
                  photo: {
                    mime: f.type,
                    content: String(reader.result).split(",")[1],
                  },
                })
                .then(() => c.notify("Goal photo saved."))
                .catch((x) => setError(message(x)));
            reader.readAsDataURL(f);
          }}
        />
      </label>
      {has && (
        <button
          className="secondary"
          onClick={() =>
            void c
              .command("VAULT_PHOTO", { id: v.id, photo: null })
              .catch((x) => setError(message(x)))
          }
        >
          Remove goal photo
        </button>
      )}
      {error && <Alert>{error}</Alert>}
    </div>
  );
}
function VaultTransfer({ vault: v }: { vault: Vault }) {
  const c = useConsumer(),
    [direction, setDirection] = useState("VAULT_DEPOSIT");
  return (
    <CommandForm
      action={direction}
      build={(f) => ({
        id: v.id,
        accountId: text(f, "accountId"),
        amountMinor: minor(text(f, "amount")),
      })}
      label="Submit vault transfer"
    >
      <label>
        Direction
        <select
          value={direction}
          onChange={(e) => setDirection(e.target.value)}
        >
          <option value="VAULT_DEPOSIT">Add money to vault</option>
          <option
            value={
              v.mutual && v.members > 1 ? "GROUP_WITHDRAW" : "VAULT_WITHDRAW"
            }
          >
            Withdraw {v.mutual && v.members > 1 ? "with group approval" : ""}
          </option>
        </select>
      </label>
      <AccountSelect />
      <label>
        Amount (TST)
        <input name="amount" inputMode="decimal" required />
      </label>
      {v.locked && direction !== "VAULT_DEPOSIT" && (
        <p className="ui-caption">Unlock this vault before withdrawing.</p>
      )}
      {c.busy && <p role="status">Saving transfer…</p>}
    </CommandForm>
  );
}
function GroupRequests({ vault: v }: { vault: Vault }) {
  const c = useConsumer();
  return (
    <Box title="Shared withdrawal approvals">
      {c.data.groupWithdrawals
        .filter((w) => w.vaultId === v.id)
        .map((w) => (
          <div className="ui-mini-card" key={w.id}>
            <strong>
              {amount(w.amountMinor)} · {w.state}
            </strong>
            <p>
              {w.approved
                ? "You approved this request."
                : "Your approval is pending."}
            </p>
            {w.state === "PENDING" && (
              <div className="ui-actions">
                {!w.approved && (
                  <button
                    onClick={() =>
                      void c
                        .command("GROUP_APPROVE", { id: w.id })
                        .catch(() => {})
                    }
                  >
                    Approve shared withdrawal
                  </button>
                )}
                {w.isRequester && (
                  <button
                    className="secondary"
                    onClick={() =>
                      void c
                        .command("GROUP_CANCEL", { id: w.id })
                        .catch(() => {})
                    }
                  >
                    Cancel withdrawal request
                  </button>
                )}
              </div>
            )}
          </div>
        ))}
      <p className="ui-caption">
        A pending request reserves funds; a posted request has a native payout
        receipt.
      </p>
    </Box>
  );
}
function Roundups() {
  const c = useConsumer(),
    r = c.data.roundups,
    [error, setError] = useState("");
  return (
    <form
      className="ui-form"
      onSubmit={(e) => {
        e.preventDefault();
        const f = new FormData(e.currentTarget);
        void Promise.resolve()
          .then(() =>
            c.review("ROUNDUP_SET", {
              cardId: text(f, "card"),
              vaultId: text(f, "vault"),
              multiplier: Number(text(f, "multiplier")),
              weeklyCapMinor: minor(text(f, "cap")),
              enabled: f.has("enabled"),
            }),
          )
          .then((r) => navigate("/invest/review/" + r.id))
          .catch((e) => setError(message(e)));
      }}
    >
      {r?.saved30DaysMinor !== undefined && (
        <div className="ui-mini-card">
          <p className="ui-kicker">Saved in the last 30 days</p>
          <div className="ui-number">{amount(r.saved30DaysMinor)}</div>
          <p className="ui-caption">
            Lifetime {amount(r.savedMinor)} · {r.roundupCount} posted
            round-up(s)
          </p>
        </div>
      )}
      <div className="ui-two">
        <label>
          Spending card
          <select name="card" defaultValue={r?.cardId} required>
            {c.data.cards.map((x) => (
              <option key={x.id} value={x.id}>
                {x.label} •••• {x.lastFour}
              </option>
            ))}
          </select>
        </label>
        <label>
          Destination vault
          <select name="vault" defaultValue={r?.vaultId} required>
            {c.data.vaults
              .filter((v) => v.isOwner)
              .map((v) => (
                <option key={v.id} value={v.id}>
                  {v.name}
                </option>
              ))}
          </select>
        </label>
        <label>
          Multiplier
          <select name="multiplier" defaultValue={r?.multiplier ?? 1}>
            {[1, 2, 3, 5].map((x) => (
              <option key={x} value={x}>
                {x}×
              </option>
            ))}
          </select>
        </label>
        <label>
          Weekly cap (TST)
          <input
            name="cap"
            inputMode="decimal"
            defaultValue={r ? decimal(r.weeklyCapMinor) : "10.00"}
            required
          />
        </label>
      </div>
      <label className="review-checkbox">
        <input
          type="checkbox"
          name="enabled"
          defaultChecked={r?.enabled ?? true}
        />
        Enable automatic round-ups
      </label>
      <p className="ui-caption">
        Applies to synthetic card purchases. Exact whole amounts save zero.
        Failed savings do not reverse a completed purchase.
      </p>
      {error && <Alert>{error}</Alert>}
      <button
        disabled={c.busy || !c.data.cards.length || !c.data.vaults.length}
      >
        Review round-up settings
      </button>
    </form>
  );
}

function SaveRules() {
  const c = useConsumer(),
    [error, setError] = useState(""),
    rules = c.data.saveRules ?? [];
  const vault = (id: string) =>
    c.data.vaults.find((v) => v.id === id)?.name ?? "vault";
  return (
    <>
      {rules.map((r) => (
        <div className="ui-mini-card" key={r.id}>
          <strong>
            {amount(r.amountMinor)} to {vault(r.vaultId)} for each{" "}
            {r.matchKind === "MERCHANT" ? "purchase at" : "purchase in"}{" "}
            {r.matchValue}
          </strong>
          <p className="ui-caption">
            Weekly cap {amount(r.weeklyCapMinor)} · saved this week{" "}
            {amount(r.savedThisWeekMinor)} · lifetime{" "}
            {amount(r.savedLifetimeMinor)} · {r.enabled ? "active" : "disabled"}
          </p>
          {r.enabled && (
            <button
              className="secondary"
              onClick={() =>
                void c
                  .command("SAVE_RULE_DISABLE", { id: r.id })
                  .catch(() => {})
              }
            >
              Disable rule
            </button>
          )}
        </div>
      ))}
      <form
        className="ui-form"
        onSubmit={(e) => {
          e.preventDefault();
          const f = new FormData(e.currentTarget);
          setError("");
          void Promise.resolve()
            .then(() =>
              c.review("SAVE_RULE_SET", {
                cardId: text(f, "card"),
                vaultId: text(f, "vault"),
                matchKind: text(f, "kind"),
                matchValue: text(f, "match"),
                amountMinor: minor(text(f, "amount")),
                weeklyCapMinor: minor(text(f, "cap")),
              }),
            )
            .then((r) => navigate("/invest/review/" + r.id))
            .catch((e) => setError(message(e)));
        }}
      >
        <div className="ui-two">
          <label>
            Match
            <select name="kind">
              <option value="MERCHANT">Merchant</option>
              <option value="CATEGORY">Category</option>
            </select>
          </label>
          <label>
            Merchant or category name
            <input name="match" maxLength={64} required />
          </label>
          <label>
            Spending card
            <select name="card" required>
              {c.data.cards.map((x) => (
                <option key={x.id} value={x.id}>
                  {x.label} •••• {x.lastFour}
                </option>
              ))}
            </select>
          </label>
          <label>
            Destination vault
            <select name="vault" required>
              {c.data.vaults
                .filter((v) => v.isOwner)
                .map((v) => (
                  <option key={v.id} value={v.id}>
                    {v.name}
                  </option>
                ))}
            </select>
          </label>
          <label>
            Save per purchase (TST)
            <input name="amount" inputMode="decimal" required />
          </label>
          <label>
            Weekly cap (TST)
            <input name="cap" inputMode="decimal" required />
          </label>
        </div>
        <p className="ui-caption">
          Each matching synthetic card purchase saves the fixed amount once,
          until the weekly cap. A failed saving never reverses the purchase.
        </p>
        {error && <Alert>{error}</Alert>}
        <button
          disabled={c.busy || !c.data.cards.length || !c.data.vaults.length}
        >
          Review saving rule
        </button>
      </form>
    </>
  );
}
function SplitExpense({ movement: a }: { movement: Activity }) {
  const c = useConsumer(),
    existing = (c.data.splits ?? []).find((x) => x.batchId === a.batchId);
  if (existing)
    return (
      <p className="ui-safe-notice" data-testid="split-summary">
        Split {amount(existing.totalMinor)} with {existing.participants}{" "}
        {existing.participants === 1 ? "person" : "people"} · {existing.paid}{" "}
        paid · your share {amount(existing.selfShareMinor)}
      </p>
    );
  return (
    <CommandForm
      action="SPLIT_EXPENSE"
      label="Split this expense"
      build={(f) => ({
        accountId: a.accountId,
        batchId: a.batchId,
        references: text(f, "references")
          .split(/[\s,]+/)
          .filter(Boolean),
        includeSelf: f.has("self"),
        memo: text(f, "memo"),
      })}
    >
      <label>
        Receiving references (comma separated, up to 9)
        <input name="references" list="favorite-references" required />
      </label>
      <FavoriteOptions />
      <label className="review-checkbox">
        <input type="checkbox" name="self" defaultChecked />
        Include my own share
      </label>
      <label>
        Memo
        <input name="memo" maxLength={120} />
      </label>
      <p className="ui-caption">
        Equal shares in exact minor units. Each person gets a request; a request
        is not a transfer.
      </p>
    </CommandForm>
  );
}
function FavoriteOptions() {
  const c = useConsumer();
  return (
    <datalist id="favorite-references">
      {(c.data.favorites ?? []).map((x) => (
        <option key={x.reference} value={x.reference}>
          {x.label}
        </option>
      ))}
    </datalist>
  );
}
export function Spend({ route }: { route: string }) {
  const c = useConsumer(),
    [filter, setFilter] = useState(""),
    [category, setCategory] = useState("All"),
    [tab, setTab] = useState(
      route.endsWith("activity") ? "activity" : "insights",
    ),
    [selected, setSelected] = useState<Activity>();
  if (selected)
    return (
      <ActivityDetail movement={selected} done={() => setSelected(undefined)} />
    );
  const a = c.data.analytics,
    items = c.data.activity.filter(
      (x) =>
        (category === "All" || x.category === category) &&
        (x.kind + " " + x.note + " " + x.batchId)
          .toLowerCase()
          .includes(filter.toLowerCase()),
    );
  return (
    <>
      <Head
        title="Spending & Insights"
        sub="Understand your habits. Make space for your priorities."
      />
      <div className="ui-tabs">
        {["insights", "budgets", "activity", "bills"].map((t) => (
          <button
            key={t}
            className="secondary"
            aria-pressed={tab === t}
            onClick={() => setTab(t)}
          >
            {t[0].toUpperCase() + t.slice(1)}
          </button>
        ))}
      </div>
      {tab === "insights" && (
        <>
          <div className="ui-grid">
            <Box className="ui-hero">
              <p className="ui-kicker">THIS MONTH’S SPENDING</p>
              <div className="ui-number">{amount(a.spentMinor)}</div>
              <p>
                {a.from} to {a.to} (exclusive) · all native bank movements for
                the period
              </p>
            </Box>
            {c.data.safeToSpend && (
              <Box title="Safe to spend">
                <div className="ui-number" data-testid="safe-to-spend">
                  {amount(c.data.safeToSpend.safeToSpendMinor)}
                </div>
                <dl className="ui-stats">
                  <div>
                    <dt>Available now</dt>
                    <dd>{amount(c.data.safeToSpend.availableMinor)}</dd>
                  </div>
                  <div>
                    <dt>Automations due</dt>
                    <dd>{amount(c.data.safeToSpend.automationsDueMinor)}</dd>
                  </div>
                  <div>
                    <dt>Bills due</dt>
                    <dd>{amount(c.data.safeToSpend.billsDueMinor)}</dd>
                  </div>
                </dl>
                <p className="ui-caption">
                  Available balance minus your approved automations and bill
                  reminders due by {c.data.safeToSpend.through}. A simple local
                  rule, not a forecast or advice.
                </p>
              </Box>
            )}
            <Box title="Income & outgoings">
              <dl className="ui-stats">
                <div>
                  <dt>Income</dt>
                  <dd>{amount(a.incomeMinor)}</dd>
                </div>
                <div>
                  <dt>Spent</dt>
                  <dd>{amount(a.spentMinor)}</dd>
                </div>
                <div>
                  <dt>Net bank movement</dt>
                  <dd>
                    {amount(BigInt(a.incomeMinor) - BigInt(a.spentMinor))}
                  </dd>
                </div>
              </dl>
              <p className="ui-caption">
                Vault and brokerage transfers are excluded. Native periodic
                charges remain visible.
              </p>
            </Box>
          </div>
          <div className="ui-grid two">
            <Box title="Spending by category">
              <Allocation
                label="Spent"
                items={a.categories.map((x) => ({
                  name: x.category,
                  value: BigInt(x.spentMinor),
                }))}
              />
            </Box>
            <Box title="Top merchants">
              {a.merchants.map((m) => (
                <div className="ui-click-row" key={m.name}>
                  <span>
                    {m.name}
                    <small className="ui-caption">
                      {" "}
                      · {m.visits} movement(s)
                    </small>
                  </span>
                  <strong>{amount(m.spentMinor)}</strong>
                </div>
              ))}
              {!a.merchants.length && (
                <p className="ui-caption">
                  Your spending insights appear after posted purchases.
                </p>
              )}
            </Box>
          </div>
        </>
      )}
      {tab === "budgets" && (
        <div className="ui-grid two">
          <Box title="Monthly category budgets">
            {c.data.budgets.map((b) => {
              const s = a.categories.find((x) => x.category === b.category);
              return (
                <article className="ui-mini-card" key={b.category}>
                  <div className="spread">
                    <strong>{b.category}</strong>
                    <span>
                      {amount(s?.spentMinor ?? "0")} /{" "}
                      {amount(s?.budgetMinor ?? b.limitMinor)}
                    </span>
                  </div>
                  <Progress
                    value={s?.spentMinor ?? "0"}
                    max={s?.budgetMinor ?? b.limitMinor}
                  />
                  <p className="ui-caption">
                    {b.rollover
                      ? "Unused prior-month budget rolls into this month."
                      : "Resets monthly."}{" "}
                    Alert threshold {b.threshold}%
                  </p>
                </article>
              );
            })}
            {!c.data.budgets.length && (
              <p>Set your first budget to track a category.</p>
            )}
          </Box>
          <Box title="Set or update a budget">
            <CommandForm
              action="BUDGET"
              build={(f) => ({
                category: text(f, "category"),
                limitMinor: minor(text(f, "limit")),
                rollover: f.has("rollover"),
                threshold: Number(text(f, "threshold")),
              })}
            >
              <label>
                Category
                <select name="category">
                  {categories.map((x) => (
                    <option key={x}>{x}</option>
                  ))}
                </select>
              </label>
              <label>
                Monthly limit (TST)
                <input name="limit" inputMode="decimal" required />
              </label>
              <label>
                Alert threshold (%)
                <input
                  name="threshold"
                  type="number"
                  min="1"
                  max="100"
                  defaultValue="80"
                  required
                />
              </label>
              <label className="review-checkbox">
                <input type="checkbox" name="rollover" />
                Roll over unused previous-month budget
              </label>
            </CommandForm>
          </Box>
        </div>
      )}
      {tab === "activity" && (
        <Box title="Transactions">
          <div className="ui-two">
            <label>
              Search transactions
              <input
                value={filter}
                onChange={(e) => setFilter(e.target.value)}
                placeholder="Search note, type or receipt…"
              />
            </label>
            <label>
              Filter category
              <select
                value={category}
                onChange={(e) => setCategory(e.target.value)}
              >
                {["All", ...categories].map((x) => (
                  <option key={x}>{x}</option>
                ))}
              </select>
            </label>
          </div>
          <ActivityList items={items} select={setSelected} />
          <p className="ui-caption">
            Most recent 100 bank movements. See Statements & documents for a
            bounded complete period.
          </p>
        </Box>
      )}
      {tab === "bills" && (
        <div className="ui-grid two">
          <Box title="Upcoming bills & subscriptions">
            {c.data.bills.map((b) => (
              <article className="ui-mini-card" key={b.id}>
                <strong>
                  {b.merchant} · {amount(b.amountMinor)}
                </strong>
                <p>
                  {b.nextDate} · {b.state}
                </p>
                {b.state === "ACTIVE" && (
                  <button
                    className="secondary"
                    onClick={() =>
                      void c
                        .command("BILL_CANCEL", { id: b.id })
                        .catch(() => {})
                    }
                  >
                    Request cancellation
                  </button>
                )}
              </article>
            ))}
            <p className="ui-caption">
              Reminders and cancellation requests are retained locally. Merchant
              payment and cancellation confirmations require an external
              provider.
            </p>
          </Box>
          <Box title="Track a bill">
            <CommandForm
              action="BILL_CREATE"
              build={(f) => ({
                merchant: text(f, "merchant"),
                amountMinor: minor(text(f, "amount")),
                nextDate: text(f, "date"),
              })}
            >
              <label>
                Merchant
                <input name="merchant" maxLength={64} required />
              </label>
              <label>
                Amount (TST)
                <input name="amount" inputMode="decimal" required />
              </label>
              <label>
                Next due date
                <input name="date" type="date" required />
              </label>
            </CommandForm>
          </Box>
        </div>
      )}
    </>
  );
}
function ActivityList({
  items,
  select,
}: {
  items: Activity[];
  select?: (a: Activity) => void;
}) {
  return (
    <>
      {items.map((a) => (
        <div className="ui-click-row" key={a.batchId + a.accountId}>
          <div>
            <strong>{a.note || a.kind.replaceAll("_", " ")}</strong>
            <small className="ui-caption">
              {a.date} · {a.category} · •••• {a.accountId.slice(-4)}
            </small>
          </div>
          <div>
            <strong className={BigInt(a.deltaMinor) < 0n ? "" : "ui-positive"}>
              {amount(a.deltaMinor)}
            </strong>
            {select && (
              <button
                className="quiet"
                aria-label={"View transaction " + a.batchId}
                onClick={() => select(a)}
              >
                Details →
              </button>
            )}
          </div>
        </div>
      ))}
      {!items.length && (
        <p className="ui-caption">No matching posted activity.</p>
      )}
    </>
  );
}
function ActivityDetail({
  movement: a,
  done,
}: {
  movement: Activity;
  done: () => void;
}) {
  const c = useConsumer(),
    [error, setError] = useState(""),
    [file, setFile] = useState<{ mime: string; content: string }>();
  return (
    <>
      <Head title="Transaction details" sub={a.kind.replaceAll("_", " ")}>
        <button className="secondary" onClick={done}>
          ← Back to transactions
        </button>
      </Head>
      <div className="ui-grid two">
        <Box>
          <div className="ui-number">{amount(a.deltaMinor)}</div>
          <dl className="ui-receipt">
            <dt>Native batch receipt</dt>
            <dd>{a.batchId}</dd>
            <dt>Business date</dt>
            <dd>{a.date}</dd>
            <dt>Account</dt>
            <dd>{a.accountId}</dd>
          </dl>
          <p className="ui-caption">
            Metadata never changes the immutable financial journal.
          </p>
          {BigInt(a.deltaMinor) < 0n ? (
            <SplitExpense movement={a} />
          ) : (
            <a href="#/payments">Request money →</a>
          )}
        </Box>
        <Box title="Category, notes & receipt">
          <CommandForm
            action="CATEGORY"
            build={(f) => ({
              accountId: a.accountId,
              batchId: a.batchId,
              category: text(f, "category"),
              note: text(f, "note"),
              ...(file ? { receipt: file } : {}),
            })}
          >
            <label>
              Category
              <select name="category" defaultValue={a.category}>
                {categories.map((x) => (
                  <option key={x}>{x}</option>
                ))}
              </select>
            </label>
            <label>
              Note
              <input name="note" maxLength={240} defaultValue={a.note} />
            </label>
            <label>
              Attach a receipt (PNG, JPEG or PDF, up to 32 KiB)
              <input
                type="file"
                accept="image/png,image/jpeg,application/pdf"
                onChange={(e) => {
                  setError("");
                  setFile(undefined);
                  const f = e.target.files?.[0];
                  if (!f) return;
                  if (
                    f.size > 32768 ||
                    !["image/png", "image/jpeg", "application/pdf"].includes(
                      f.type,
                    )
                  ) {
                    setError("Choose PNG, JPEG or PDF up to 32 KiB.");
                    return;
                  }
                  const reader = new FileReader();
                  reader.onload = () =>
                    setFile({
                      mime: f.type,
                      content: String(reader.result).split(",")[1],
                    });
                  reader.readAsDataURL(f);
                }}
              />
            </label>
            {error && <Alert>{error}</Alert>}
            {a.receipt && (
              <button
                type="button"
                className="secondary"
                onClick={() => {
                  const bytes = Uint8Array.from(atob(a.receipt!.content), (x) =>
                    x.charCodeAt(0),
                  );
                  download(
                    new Blob([bytes], { type: a.receipt!.mime }),
                    "receipt-" +
                      a.batchId +
                      (a.receipt!.mime === "application/pdf"
                        ? ".pdf"
                        : ".image"),
                  );
                }}
              >
                Download attached receipt
              </button>
            )}
          </CommandForm>
        </Box>
      </div>
    </>
  );
}
export function download(blob: Blob, name: string) {
  const url = URL.createObjectURL(blob),
    a = document.createElement("a");
  a.href = url;
  a.download = name;
  a.click();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}

export function Wallet() {
  const c = useConsumer(),
    [error, setError] = useState(""),
    [cardId, setCardId] = useState(c.data.cards[0]?.id ?? "");
  const card = c.data.cards.find((x) => x.id === cardId);
  async function review(action: string, input: Record<string, unknown>) {
    try {
      const r = await c.review(action, input);
      navigate("/invest/review/" + r.id);
    } catch (e) {
      setError(message(e));
    }
  }
  return (
    <>
      <Head
        title="Cards & Wallet"
        sub="Your spending controls and connected accounts in one place."
      />
      {error && <Alert>{error}</Alert>}
      <p className="ui-safe-notice">
        Cards and bank links use local simulation. No real PAN, card network
        authorization or external bank connection is created.
      </p>
      <div className="ui-grid two">
        <Box title="Your virtual cards">
          <label className="ui-form">
            Select card
            <select value={cardId} onChange={(e) => setCardId(e.target.value)}>
              <option value="">Choose a card</option>
              {c.data.cards.map((x) => (
                <option key={x.id} value={x.id}>
                  {x.label} •••• {x.lastFour}
                </option>
              ))}
            </select>
          </label>
          {card ? (
            <>
              <div className="ui-bank-card">
                <div className="spread">
                  <strong>piggybank.</strong>
                  <span>{card.frozen ? "FROZEN" : "SYNTHETIC"}</span>
                </div>
                <p className="ui-card-chip">▣</p>
                <p className="ui-card-pan">•••• •••• •••• {card.lastFour}</p>
                <div className="spread">
                  <span>{card.label}</span>
                  <span>VIRTUAL</span>
                </div>
              </div>
              <div className="ui-actions">
                <button
                  className="secondary"
                  onClick={() =>
                    void c
                      .command("CARD_UPDATE", {
                        id: card.id,
                        frozen: !card.frozen,
                      })
                      .catch(() => {})
                  }
                >
                  {card.frozen ? "Unfreeze" : "Freeze"} card
                </button>
                <button
                  className="secondary"
                  onClick={() => void review("CARD_REVEAL", { id: card.id })}
                >
                  Reveal synthetic token
                </button>
              </div>
              <label className="review-checkbox">
                <input
                  type="checkbox"
                  checked={card.onlineEnabled}
                  onChange={(e) =>
                    void c
                      .command("CARD_UPDATE", {
                        id: card.id,
                        onlineEnabled: e.target.checked,
                      })
                      .catch(() => {})
                  }
                />
                Online purchases enabled
              </label>
              <label className="review-checkbox">
                <input
                  type="checkbox"
                  checked={card.atmEnabled}
                  onChange={(e) =>
                    void c
                      .command("CARD_UPDATE", {
                        id: card.id,
                        atmEnabled: e.target.checked,
                      })
                      .catch(() => {})
                  }
                />
                ATM preference (no real ATM adapter)
              </label>
            </>
          ) : (
            <p>Create a virtual card for an active bank account.</p>
          )}
          <h3>Create a virtual card</h3>
          <CommandForm
            action="CARD_CREATE"
            build={(f) => ({
              accountId: text(f, "accountId"),
              label: text(f, "label"),
            })}
            after={(r) => setCardId(String(r.id))}
            label="Create synthetic card"
          >
            <AccountSelect />
            <label>
              Card label
              <input name="label" maxLength={64} required />
            </label>
          </CommandForm>
        </Box>
        <Box title="Try a synthetic purchase">
          <form
            className="ui-form"
            onSubmit={(e) => {
              e.preventDefault();
              const f = new FormData(e.currentTarget);
              void Promise.resolve()
                .then(() =>
                  review("CARD_PURCHASE", {
                    id: cardId,
                    merchant: text(f, "merchant"),
                    category: text(f, "category"),
                    amountMinor: minor(text(f, "amount")),
                  }),
                )
                .catch((e) => setError(message(e)));
            }}
          >
            <label>
              Merchant
              <input name="merchant" maxLength={64} required />
            </label>
            <label>
              Category
              <select name="category">
                {categories.map((x) => (
                  <option key={x}>{x}</option>
                ))}
              </select>
            </label>
            <label>
              Purchase amount (TST)
              <input name="amount" inputMode="decimal" required />
            </label>
            <p className="ui-caption">
              Uses the selected card’s native account balance. The issuer
              approval applies to this one exact purchase.
            </p>
            <button disabled={c.busy || !card || card.frozen}>
              Review synthetic purchase
            </button>
          </form>
          <hr />
          <h3>Automatic savings</h3>
          <p>
            Round spare change into a goal vault after each eligible purchase.
          </p>
          <a href="#/save">Manage round-ups →</a>
        </Box>
        <Box title="Connected banks">
          {c.data.links.map((l) => (
            <article className="ui-mini-card" key={l.id}>
              <strong>{l.institution}</strong>
              <p>
                {l.label} •••• {l.lastFour} · {l.state}
              </p>
              {l.state !== "REVOKED" && (
                <>
                  <button
                    className="quiet"
                    onClick={() =>
                      void c
                        .command("UNLINK_BANK", { id: l.id })
                        .catch(() => {})
                    }
                  >
                    Disconnect
                  </button>
                  <form
                    className="ui-form"
                    onSubmit={(e) => {
                      e.preventDefault();
                      const f = new FormData(e.currentTarget);
                      void Promise.resolve()
                        .then(() =>
                          review("LINK_DEPOSIT", {
                            id: l.id,
                            accountId: text(f, "accountId"),
                            amountMinor: minor(text(f, "amount")),
                          }),
                        )
                        .catch((e) => setError(message(e)));
                    }}
                  >
                    <AccountSelect />
                    <label>
                      Add test money (TST)
                      <input name="amount" inputMode="decimal" required />
                    </label>
                    <button className="secondary" disabled={c.busy}>
                      Review test deposit
                    </button>
                  </form>
                </>
              )}
            </article>
          ))}
          <p className="ui-caption">
            The local simulator allows up to TST 1,000 total funding per person
            across all links. Funding uses a balanced native clearing journal.
          </p>
        </Box>
        <Box title="Link a simulated bank">
          <CommandForm
            action="LINK_BANK"
            build={(f) => ({
              institution: text(f, "institution"),
              label: text(f, "label"),
              lastFour: text(f, "lastFour"),
            })}
            label="Connect synthetic account"
          >
            <label>
              Institution name
              <input name="institution" minLength={2} maxLength={64} required />
            </label>
            <label>
              Account label
              <input name="label" maxLength={64} required />
            </label>
            <label>
              Last four digits
              <input
                name="lastFour"
                pattern="[0-9]{4}"
                inputMode="numeric"
                maxLength={4}
                required
              />
            </label>
            <p className="ui-caption">
              Enter a label and masked digits only. A real bank-link provider
              has not been connected.
            </p>
          </CommandForm>
        </Box>
      </div>
    </>
  );
}
export function Notifications() {
  const c = useConsumer(),
    [filter, setFilter] = useState("All"),
    [unread, setUnread] = useState(false),
    items = c.data.notifications.filter(
      (n) =>
        (filter === "All" || n.category === filter) && (!unread || !n.readAt),
    );
  return (
    <>
      <Head title="Notifications" sub="Keep up with your money and your goals.">
        <button
          className="secondary"
          onClick={() =>
            void c.command("NOTIFICATIONS_READ", {}).catch(() => {})
          }
        >
          Mark all as read
        </button>
      </Head>
      <Box>
        <div className="ui-two">
          <label>
            Notification category
            <select value={filter} onChange={(e) => setFilter(e.target.value)}>
              {[
                "All",
                "Transactions",
                "Savings",
                "Investments",
                "Security",
              ].map((x) => (
                <option key={x}>{x}</option>
              ))}
            </select>
          </label>
          <label className="review-checkbox">
            <input
              type="checkbox"
              checked={unread}
              onChange={(e) => setUnread(e.target.checked)}
            />
            Unread only
          </label>
        </div>
        {items.map((n) => (
          <article className="ui-click-row" key={n.id}>
            <div>
              <span className="ui-pill">{n.category}</span>
              <h3>{n.title}</h3>
              <p>{n.body}</p>
              <small className="ui-caption">
                {new Date(n.createdAt).toLocaleString()}
              </small>
            </div>
            <div className="ui-actions">
              <a
                href={"#" + n.route}
                onClick={() => {
                  if (!n.readAt)
                    void c
                      .command("NOTIFICATIONS_READ", { id: n.id })
                      .catch(() => {});
                }}
              >
                Open →
              </a>
              {!n.readAt && (
                <button
                  className="quiet"
                  aria-label={"Mark " + n.title + " as read"}
                  onClick={() =>
                    void c
                      .command("NOTIFICATIONS_READ", { id: n.id })
                      .catch(() => {})
                  }
                >
                  Mark read
                </button>
              )}
            </div>
          </article>
        ))}
        {!items.length && <p className="ui-caption">You’re all caught up.</p>}
      </Box>
      <Messages />
    </>
  );
}
export function Preferences() {
  const c = useConsumer(),
    { client } = useSession(),
    [error, setError] = useState("");
  async function statement(id: string, format: string) {
    try {
      const d = (await client
        .GET("/api/v1/consumer/statements/{id}", { params: { path: { id } } })
        .then(value)) as {
        items: {
          batchId: string;
          date: string;
          kind: string;
          deltaMinor: string;
        }[];
        accountId: string;
        from: string;
        to: string;
        openingMinor: string;
        closingMinor: string;
        digest: string;
      };
      if (format === "json") {
        download(
          new Blob([JSON.stringify(d, null, 2)], { type: "application/json" }),
          "statement-" + id + ".json",
        );
        return;
      }
      const cell = (s: string) =>
        '"' +
        (/^[=+\-@\t\r]/.test(s) ? "\u0027" : "") +
        s.replaceAll('"', '""') +
        '"';
      const rows = [
        ["Account", d.accountId],
        ["From inclusive", d.from, "To exclusive", d.to],
        [
          "Opening minor TST",
          d.openingMinor,
          "Closing minor TST",
          d.closingMinor,
        ],
        ["Source SHA-256", d.digest],
        ["Native batch", "Business date", "Kind", "Exact minor TST"],
        ...d.items.map((x) => [x.batchId, x.date, x.kind, x.deltaMinor]),
      ];
      download(
        new Blob(
          [
            "Synthetic TST statement; not a tax form\r\n" +
              rows.map((r) => r.map(cell).join(",")).join("\r\n"),
          ],
          { type: "text/csv;charset=utf-8" },
        ),
        "statement-" + id + ".csv",
      );
    } catch (e) {
      setError(message(e));
    }
  }
  return (
    <>
      <Head
        title="Profile & Preferences"
        sub="Make Piggybank feel like yours."
      />
      {error && <Alert>{error}</Alert>}
      <div className="ui-grid two">
        <Box title="Your preferences">
          <CommandForm
            action="PREFERENCES"
            build={(f) => ({
              theme: text(f, "theme"),
              hideBalance: f.has("hide"),
              notifications: f.has("notifications"),
              marketing: f.has("marketing"),
            })}
          >
            <label>
              Appearance
              <select
                name="theme"
                defaultValue={c.data.preferences.theme ?? "light"}
              >
                <option value="light">Light</option>
                <option value="dark">Dark</option>
                <option value="system">Device setting</option>
              </select>
            </label>
            <label className="review-checkbox">
              <input
                type="checkbox"
                name="hide"
                defaultChecked={c.data.preferences.hideBalance}
              />
              Hide net worth on overview
            </label>
            <label className="review-checkbox">
              <input
                type="checkbox"
                name="notifications"
                defaultChecked={c.data.preferences.notifications !== false}
              />
              Financial activity notifications
            </label>
            <label className="review-checkbox">
              <input
                type="checkbox"
                name="marketing"
                defaultChecked={c.data.preferences.marketing}
              />
              Product announcements
            </label>
          </CommandForm>
          <p>
            <a href="#/profile">Identity, personal details & security →</a>
          </p>
          <p>
            <a href="#/support">Contact support →</a>
          </p>
        </Box>
        <Box title="Personalize your goals">
          <CommandForm
            action="PREFERENCES"
            build={(f) => ({
              goals: f.getAll("goal").map(String),
              onboardingComplete: true,
            })}
          >
            <p>What would you like to build?</p>
            {[
              "Emergency fund",
              "Travel",
              "Home",
              "Investing",
              "Everyday budgeting",
            ].map((g) => (
              <label className="review-checkbox" key={g}>
                <input
                  type="checkbox"
                  name="goal"
                  value={g}
                  defaultChecked={c.data.preferences.goals?.includes(g)}
                />
                {g}
              </label>
            ))}
          </CommandForm>
          <p className="ui-caption">
            Personalization never moves money or submits investment
            instructions.
          </p>
        </Box>
        <Box title="Custom activity report">
          <p>
            <a className="text-link" href="#/documents">
              Monthly statements & documents →
            </a>
          </p>
          <CommandForm
            action="STATEMENT"
            build={(f) => ({
              accountId: text(f, "accountId"),
              from: text(f, "from"),
              to: text(f, "to"),
            })}
            label="Create activity report"
          >
            <AccountSelect />
            <div className="ui-two">
              <label>
                From (inclusive)
                <input name="from" type="date" required />
              </label>
              <label>
                To (exclusive)
                <input name="to" type="date" required />
              </label>
            </div>
            <p className="ui-caption">
              Up to 366 days and 1,000 complete native facts. Narrow the period
              if the bound is exceeded.
            </p>
          </CommandForm>
          {c.data.statements.map((s) => (
            <div className="ui-mini-card" key={s.id}>
              <strong>
                {s.from} → {s.to}
              </strong>
              <p className="ui-code">SHA-256 {s.digest}</p>
              <div className="ui-actions">
                <button
                  className="secondary"
                  onClick={() => void statement(s.id, "csv")}
                >
                  Download CSV
                </button>
                <button
                  className="quiet"
                  onClick={() => void statement(s.id, "json")}
                >
                  Native JSON
                </button>
                <a href={"/api/v1/consumer/statements/" + s.id + "/pdf"}>
                  Download PDF
                </a>
              </div>
            </div>
          ))}
        </Box>
        <Box title="Developer API access">
          <form
            className="ui-form"
            onSubmit={(e) => {
              e.preventDefault();
              const f = new FormData(e.currentTarget);
              void c
                .review("TOKEN_CREATE", {
                  label: text(f, "label"),
                  scope: text(f, "scope"),
                })
                .then((r) => navigate("/invest/review/" + r.id))
                .catch((e) => setError(message(e)));
            }}
          >
            <label>
              Token label
              <input name="label" maxLength={64} required />
            </label>
            <label>
              Read scope
              <select name="scope">
                <option>accounts:read</option>
                <option>transactions:read</option>
                <option>portfolio:read</option>
              </select>
            </label>
            <p className="ui-caption">
              Expires after 30 days. Copy once after issuer authorization. Only
              a hash is stored.
            </p>
            <button disabled={c.busy}>Review scoped API token</button>
          </form>
          {c.data.tokens.map((t) => (
            <article className="ui-mini-card" key={t.id}>
              <strong>
                {t.label} · {t.scope}
              </strong>
              <p>
                {t.revokedAt
                  ? "Revoked"
                  : "Expires " + new Date(t.expiresAt).toLocaleDateString()}
              </p>
              {!t.revokedAt && (
                <button
                  className="secondary"
                  onClick={() =>
                    void c.command("TOKEN_REVOKE", { id: t.id }).catch(() => {})
                  }
                >
                  Revoke token
                </button>
              )}
            </article>
          ))}
          <p className="ui-code">
            GET /api/v1/consumer/integrations/accounts:read
            <br />
            Authorization: Bearer pb_test_…
          </p>
          <p className="ui-caption">
            Tokens authorize this read endpoint only. They cannot authorize
            payments or trade commands.
          </p>
        </Box>
      </div>
    </>
  );
}
type SettledPayment = {
  id: string;
  operationId: string | null;
  state: string;
  beneficiaryLabel: string;
  details: { amount: { minorUnits: string }; rail: string };
};
function LinkPayment({
  request,
}: {
  request: { id: string; amountMinor: string };
}) {
  const { client } = useSession(),
    c = useConsumer(),
    [payments, setPayments] = useState<SettledPayment[]>(),
    [error, setError] = useState("");
  if (!payments)
    return (
      <button
        className="secondary"
        onClick={() =>
          void client
            .GET("/api/v1/payments")
            .then(value)
            .then((all) =>
              setPayments(
                (all as unknown as SettledPayment[]).filter(
                  (p) =>
                    p.state === "SETTLED" &&
                    p.details.rail === "INTERNAL" &&
                    p.details.amount.minorUnits === request.amountMinor,
                ),
              ),
            )
            .catch((e) => setError(message(e)))
        }
      >
        I already paid this
      </button>
    );
  return (
    <div>
      {payments.length === 0 && (
        <p className="ui-caption">
          No settled internal payment of exactly {amount(request.amountMinor)}{" "}
          was found.
        </p>
      )}
      {payments.map((p) => (
        <button
          key={p.id}
          className="secondary"
          onClick={() =>
            void c
              .command("REQUEST_MARK_PAID", {
                id: request.id,
                paymentId: p.operationId ?? p.id,
              })
              .catch((e) => setError(message(e)))
          }
        >
          Link payment to {p.beneficiaryLabel} ({short8(p.id)})
        </button>
      ))}
      {error && <Alert>{error}</Alert>}
    </div>
  );
}
const short8 = (id: string) => id.slice(-8).toUpperCase();
function Favorites() {
  const c = useConsumer(),
    favorites = c.data.favorites ?? [];
  return (
    <Box title="Favourite people">
      {favorites.length === 0 ? (
        <p className="ui-caption">No favourites yet.</p>
      ) : (
        <ul className="ui-chip-list" data-testid="favorites">
          {favorites.map((x) => (
            <li key={x.reference}>
              <strong>{x.label}</strong> <code>{x.reference}</code>{" "}
              <button
                className="secondary"
                onClick={() =>
                  void c
                    .command("FAVORITE_REMOVE", { reference: x.reference })
                    .catch(() => {})
                }
              >
                Remove {x.label}
              </button>
            </li>
          ))}
        </ul>
      )}
      <CommandForm
        action="FAVORITE_SET"
        label="Save favourite"
        build={(f) => ({
          reference: text(f, "reference"),
          label: text(f, "label"),
        })}
      >
        <div className="ui-two">
          <label>
            Receiving reference
            <input name="reference" maxLength={100} required />
          </label>
          <label>
            Name
            <input name="label" maxLength={40} required />
          </label>
        </div>
      </CommandForm>
    </Box>
  );
}
export function MoneyRequests() {
  const c = useConsumer();
  return (
    <>
      <Favorites />
      <Box title="Request or split an expense">
        <CommandForm
          action="REQUEST_MONEY"
          build={(f) => ({
            accountId: text(f, "accountId"),
            receivingReference: text(f, "reference"),
            amountMinor: minor(text(f, "amount")),
            memo: text(f, "memo"),
          })}
          label="Send payment request"
        >
          <AccountSelect />
          <label>
            Other person’s receiving reference
            <input
              name="reference"
              maxLength={100}
              list="favorite-references"
              required
            />
          </label>
          <FavoriteOptions />
          <div className="ui-two">
            <label>
              Their share (TST)
              <input name="amount" inputMode="decimal" required />
            </label>
            <label>
              Memo
              <input name="memo" maxLength={120} />
            </label>
          </div>
        </CommandForm>
        {c.data.requests.map((r) => (
          <article className="ui-mini-card" key={r.id}>
            <strong>
              {r.incoming ? "Incoming request" : "Your request"} ·{" "}
              {amount(r.amountMinor)}
            </strong>
            <p>
              {r.memo} · {r.state}
              {c.data.requestLinks?.[r.id]?.splitId ? " · part of a split" : ""}
            </p>
            {r.incoming && r.state === "REQUESTED" ? (
              <>
                <p className="ui-code">
                  Recipient receiving reference: {r.reference}
                </p>
                <p className="ui-caption">
                  Use Send money with this receiving reference and exact share.
                  Payment requires its own quote and issuer authorization; a
                  request is not a transfer.
                </p>
                <LinkPayment request={r} />
              </>
            ) : (
              !r.incoming &&
              r.state === "REQUESTED" && (
                <>
                  {r.reference && (
                    <details className="ui-qr-share">
                      <summary>Share this request as a QR code</summary>
                      <QrCode
                        payload={encodePaymentCode({
                          reference: r.reference,
                          amountMinor: r.amountMinor,
                          memo: r.memo || undefined,
                        })}
                        label={`QR code requesting ${amount(r.amountMinor)}`}
                      />
                      <p className="ui-caption">
                        The code carries your receiving reference, the amount
                        and the memo. It cannot move money by itself.
                      </p>
                    </details>
                  )}
                  <button
                    className="secondary"
                    onClick={() =>
                      void c
                        .command("REQUEST_CANCEL", { id: r.id })
                        .catch(() => {})
                    }
                  >
                    Cancel request
                  </button>
                </>
              )
            )}
          </article>
        ))}
      </Box>
    </>
  );
}
