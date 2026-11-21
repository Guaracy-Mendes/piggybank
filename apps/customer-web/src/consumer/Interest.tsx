import { useState } from "react";
import { useConsumer } from "./context";
import { Box, Head, Progress } from "./components";
import {
  amount,
  percent,
  type RateComponent,
  type Savings,
  type SavingsWeek,
} from "./model";

/* UI-14 local template: savings interest under user-approved synthetic rules (schema 80). Every figure is read from
   native Core receipts; this page only explains them. */
const monthName = (month: string) =>
  new Date(month.slice(0, 7) + "-01T00:00:00Z").toLocaleString("en-US", {
    month: "long",
    year: "numeric",
    timeZone: "UTC",
  });
const fmt = (day: string) =>
  new Date(day + "T00:00:00Z").toLocaleDateString("en-US", {
    month: "short",
    day: "numeric",
    timeZone: "UTC",
  });

function Component({ c, s }: { c: RateComponent; s: Savings }) {
  if (c.code === "BASE")
    return (
      <li>
        <strong>Base rate {percent(c.bps)}</strong>
        <span className="ui-caption"> · on your own vaults</span>
      </li>
    );
  if (c.code === "SAVING_STREAK")
    return (
      <li data-applied={c.applied}>
        <strong>
          Saving streak +{percent(c.bps)} {c.applied ? "✓" : "—"}
        </strong>
        <span className="ui-caption">
          {" "}
          ·{" "}
          {c.applied
            ? `you saved in every week of ${monthName(c.basisMonth!)}`
            : `not earned: ${monthName(c.basisMonth!)} had a week without saving`}
        </span>
      </li>
    );
  const open = !c.firstMonth
    ? "starts after your first account opens"
    : c.applied
      ? `applies through ${monthName(c.lastMonth!)}`
      : s.rate.month < c.firstMonth
        ? `starts ${monthName(c.firstMonth)}, your first full month`
        : `ended ${monthName(c.lastMonth!)}`;
  return (
    <li data-applied={c.applied}>
      <strong>
        New customer +{percent(c.bps)} {c.applied ? "✓" : "—"}
      </strong>
      <span className="ui-caption"> · {open}</span>
    </li>
  );
}

function Weeks({ weeks }: { weeks: SavingsWeek[] }) {
  const label: Record<SavingsWeek["state"], string> = {
    SAVED: "saved",
    MISSED: "missed",
    OPEN: "this week",
    UPCOMING: "upcoming",
  };
  return (
    <ol className="ui-weeks" aria-label="Saving weeks this month">
      {weeks.map((w) => (
        <li key={w.from} data-state={w.state}>
          <span>
            {fmt(w.from)}–{fmt(w.to)}
          </span>
          <strong>{label[w.state]}</strong>
        </li>
      ))}
    </ol>
  );
}

function Join({ s }: { s: Savings }) {
  const c = useConsumer(),
    [agreed, setAgreed] = useState(false);
  return (
    <Box className="ui-hero" title={s.program.title}>
      <p className="ui-kicker">SYNTHETIC EVALUATION</p>
      <div className="ui-number">{s.program.baseApyPercent}% APY</div>
      <p>
        Your vaults earn {percent(s.program.baseRateBps)} a year, up to{" "}
        {percent(s.program.maxRateBps)} ({s.program.maxApyPercent}% APY) with
        boosts.
      </p>
      <ul className="ui-terms">
        {s.program.terms.map((t) => (
          <li key={t}>{t}</li>
        ))}
      </ul>
      {s.enrollment && !s.enrollment.active ? (
        <p>
          You left the program on {s.enrollment.leftOn}. Interest accrued
          through the day before and is credited after each month closes.
        </p>
      ) : s.canJoin ? (
        <form
          className="ui-form"
          onSubmit={(e) => {
            e.preventDefault();
            void c
              .command("SAVINGS_JOIN", {
                digest: s.program.digest,
                accepted: true,
              })
              .then(() =>
                c.notify("You joined. Your vaults now earn interest."),
              )
              .catch(() => {});
          }}
        >
          <label className="ui-checkbox">
            <input
              type="checkbox"
              checked={agreed}
              onChange={(e) => setAgreed(e.target.checked)}
            />
            I agree to these synthetic savings interest terms (version{" "}
            {s.program.version})
          </label>
          <button type="submit" disabled={!agreed || c.busy}>
            Join and start earning
          </button>
        </form>
      ) : (
        <p>
          Open an account first. <a href="#/catalog">See accounts →</a>
        </p>
      )}
    </Box>
  );
}

function Rate({ s }: { s: Savings }) {
  const r = s.rate,
    mtd = s.monthToDate;
  return (
    <Box className="ui-hero">
      <p className="ui-kicker">
        YOUR RATE IN {monthName(r.month).toUpperCase()}
      </p>
      <div className="ui-number">{r.apyPercent}% APY</div>
      <p>{percent(r.totalBps)} a year on your vaults, credited monthly.</p>
      <ul className="ui-rate-parts" aria-label="Rate breakdown">
        {r.components.map((x) => (
          <Component key={x.code} c={x} s={s} />
        ))}
      </ul>
      {r.uncappedBps > r.capBps && (
        <p className="ui-caption">
          Boosts are capped at {percent(r.capBps)} in total.
        </p>
      )}
      {mtd && (
        <p>
          Accrued so far this month: <strong>{amount(mtd.totalMinor)}</strong>{" "}
          <span className="ui-caption">
            (estimate through {mtd.to < mtd.from ? "today" : mtd.to}; credited
            after {monthName(r.month)} closes)
          </span>
        </p>
      )}
      <p className="ui-caption">
        Interest applies to at most {amount(s.program.balanceCapMinor)} across
        your own vaults, oldest vault first.
      </p>
    </Box>
  );
}

function Challenge({ s }: { s: Savings }) {
  const c = useConsumer(),
    own = c.data.vaults.filter((v) => v.isOwner),
    ch = s.challenge,
    p = s.program.challenge;
  const name = (id: string) =>
    c.data.vaults.find((v) => v.id === id)?.name ?? "your vault";
  return (
    <Box title="One-off challenge">
      <p>
        Save {amount(p.targetMinor)} into one vault within {p.days} days and
        receive a {amount(p.bonusMinor)} bonus in that vault.
      </p>
      {!ch ? (
        own.length ? (
          <form
            className="ui-form"
            onSubmit={(e) => {
              e.preventDefault();
              const vaultId = String(
                new FormData(e.currentTarget).get("vaultId"),
              );
              void c
                .command("SAVINGS_CHALLENGE", { vaultId })
                .then(() => c.notify("Challenge started. You have 30 days."))
                .catch(() => {});
            }}
          >
            <label>
              Challenge vault
              <select name="vaultId">
                {own.map((v) => (
                  <option key={v.id} value={v.id}>
                    {v.name}
                  </option>
                ))}
              </select>
            </label>
            <button type="submit" disabled={c.busy}>
              Start challenge
            </button>
            <p className="ui-caption">You can take this challenge once.</p>
          </form>
        ) : (
          <a href="#/save/new">Create a vault to start →</a>
        )
      ) : (
        <>
          <p>
            <strong>{name(ch.vaultId)}</strong> · {amount(ch.progressMinor)} of{" "}
            {amount(ch.targetMinor)} ·{" "}
            {ch.state === "ACTIVE"
              ? `ends ${ch.endsOn}`
              : ch.state === "COMPLETED"
                ? `completed ${ch.completedOn}; ${amount(ch.bonusMinor)} bonus added`
                : `ended ${ch.endsOn} without reaching the target`}
          </p>
          <Progress
            value={ch.progressMinor}
            max={ch.targetMinor}
            label="Challenge progress"
          />
        </>
      )}
    </Box>
  );
}

function Referral({ s }: { s: Savings }) {
  const c = useConsumer(),
    r = s.referral,
    p = s.program.referral;
  const outcome: Record<string, string> = {
    PAID: "you received " + amount(p.bonusMinor),
    ANNUAL_LIMIT: "no bonus for you: annual limit reached",
    NO_ACTIVE_ACCOUNT: "no bonus for you: no active account",
  };
  return (
    <Box title="Refer a friend">
      <p>
        When someone uses your code within {p.claimWithinDays} days of opening
        their first account and keeps at least {amount(p.fundMinor)} in their
        accounts for {p.holdDays} days, you each receive {amount(p.bonusMinor)}.
      </p>
      {r.code ? (
        <>
          <p className="ui-kicker">YOUR CODE</p>
          <p className="ui-referral-code" data-testid="referral-code">
            {r.code}
          </p>
          <p className="ui-caption">
            {r.paidThisYear} of {r.maxPaidPerYear} paid referrals this year.
          </p>
        </>
      ) : (
        <p className="ui-caption">Join the program to get a referral code.</p>
      )}
      {r.referrals.length > 0 && (
        <ul aria-label="Your referrals">
          {r.referrals.map((x) => (
            <li key={x.id}>
              Code used {x.claimedOn} ·{" "}
              {x.state === "PENDING"
                ? "waiting for the 30-day balance"
                : x.state === "EXPIRED"
                  ? "expired"
                  : `qualified ${x.qualifiedOn}; ${outcome[x.yourBonus ?? ""] ?? ""}`}
            </li>
          ))}
        </ul>
      )}
      {r.claimed ? (
        <p>
          You used a referral code on {r.claimed.claimedOn}.{" "}
          {r.claimed.state === "PAID"
            ? `Your ${amount(p.bonusMinor)} bonus was paid on ${r.claimed.qualifiedOn}.`
            : r.claimed.state === "EXPIRED"
              ? "It expired."
              : r.claimed.holdUntil
                ? `Keep at least ${amount(p.fundMinor)} in your accounts through ${r.claimed.holdUntil}.`
                : "Open your first account and keep the minimum balance for 30 days."}
        </p>
      ) : (
        r.canClaim && (
          <form
            className="ui-form"
            onSubmit={(e) => {
              e.preventDefault();
              const code = String(new FormData(e.currentTarget).get("code"));
              void c
                .command("REFERRAL_CLAIM", { code })
                .then(() => c.notify("Referral code saved."))
                .catch(() => {});
            }}
          >
            <label>
              Were you referred? Enter the code
              <input
                name="code"
                placeholder="PB1A2B3C4D"
                required
                maxLength={12}
              />
            </label>
            <button type="submit" disabled={c.busy}>
              Use referral code
            </button>
            {r.claimUntil && (
              <p className="ui-caption">
                Codes can be entered until {r.claimUntil}.
              </p>
            )}
          </form>
        )
      )}
    </Box>
  );
}

function Leave() {
  const c = useConsumer(),
    [sure, setSure] = useState(false);
  return (
    <details className="ui-leave">
      <summary>Leave the savings interest program</summary>
      <p>
        Interest accrues until the day before you leave and is credited after
        the month closes. Leaving is final for this program version.
      </p>
      <label className="ui-checkbox">
        <input
          type="checkbox"
          checked={sure}
          onChange={(e) => setSure(e.target.checked)}
        />
        I understand that leaving is final
      </label>
      <button
        className="secondary"
        disabled={!sure || c.busy}
        onClick={() =>
          void c
            .command("SAVINGS_LEAVE", { confirm: true })
            .then(() => c.notify("You left the savings interest program."))
            .catch(() => {})
        }
      >
        Leave program
      </button>
    </details>
  );
}

export function SavingsInterest() {
  const c = useConsumer(),
    s = c.data.savings;
  const head = (
    <Head
      title="Interest & boosts"
      sub="Your vaults earn interest under approved synthetic rules."
    >
      <a className="text-link" href="#/save">
        ← Savings
      </a>
    </Head>
  );
  if (!s)
    return (
      <>
        {head}
        <Box>
          <p>Savings interest becomes available after you open an account.</p>
        </Box>
      </>
    );
  const active = !!s.enrollment?.active;
  return (
    <>
      {head}
      <div className="ui-grid">
        {active ? <Rate s={s} /> : <Join s={s} />}
        {active && (
          <Box title="Next month's streak boost">
            <p>
              Save into any vault in every week below to earn +
              {percent(
                s.rate.components.find((x) => x.code === "SAVING_STREAK")
                  ?.bps ?? 0,
              )}{" "}
              next month.
            </p>
            <Weeks weeks={s.streakWeeks} />
          </Box>
        )}
      </div>
      {s.enrollment && (
        <Box title="Interest by month">
          <p className="ui-kicker">LIFETIME SAVINGS INTEREST</p>
          <div className="ui-number">{amount(s.lifetimeInterestMinor)}</div>
          {s.months.length ? (
            <div className="ui-table-scroll">
              <table>
                <caption className="ui-caption">
                  Each month is finalized once after it closes and credited into
                  your vaults.
                </caption>
                <thead>
                  <tr>
                    <th scope="col">Month</th>
                    <th scope="col">Rate</th>
                    <th scope="col">Interest</th>
                  </tr>
                </thead>
                <tbody>
                  {s.months.map((m) => (
                    <tr key={m.month}>
                      <td>{monthName(m.month)}</td>
                      <td>
                        {percent(m.rateBps)} ({m.apyPercent}% APY)
                      </td>
                      <td>{amount(m.totalMinor)}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          ) : (
            <p className="ui-caption">
              Your first month is credited after {monthName(s.rate.month)}{" "}
              closes.
            </p>
          )}
        </Box>
      )}
      <div className="ui-grid">
        {active && <Challenge s={s} />}
        <Referral s={s} />
      </div>
      {active && <Leave />}
      <p className="ui-caption">{s.program.decision}</p>
    </>
  );
}
