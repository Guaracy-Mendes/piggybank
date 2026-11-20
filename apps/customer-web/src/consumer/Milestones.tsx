import { useEffect, useMemo, useState } from "react";
import { useConsumer } from "./context";
import { Box, Head, Progress } from "./components";
import {
  amount,
  decimal,
  type Badge,
  type NetWorthPoint,
  type ProgressReport,
} from "./model";

const DAY = 86_400_000;
const addDays = (iso: string, n: number) =>
  new Date(Date.parse(iso + "T00:00:00Z") + n * DAY).toISOString().slice(0, 10);
const monthName = (month: string) =>
  new Date(month + "-01T00:00:00Z").toLocaleString("en-US", {
    month: "short",
    year: "numeric",
    timeZone: "UTC",
  });
const abs = (n: bigint) => (n < 0n ? -n : n);

export function StreakBadge({ p }: { p?: ProgressReport }) {
  if (!p) return null;
  const n = p.savings.currentDays;
  return (
    <a
      className="ui-pill ui-streak"
      href="#/save/milestones"
      aria-label={
        n
          ? `Current saving streak ${n} day${n === 1 ? "" : "s"}. View milestones`
          : "No active saving streak. View milestones"
      }
    >
      <span aria-hidden="true">{n ? "🔥" : "✦"}</span>{" "}
      {n ? `${n}-Day Saver` : "Start a saving streak"}
    </a>
  );
}

/** Change since the UTC month start, from retained native facts; never estimated. */
export function MonthlyDelta({
  p,
  hidden,
}: {
  p?: ProgressReport;
  hidden?: boolean;
}) {
  if (!p) return null;
  const n = p.netWorth;
  if (n.deltaMinor === null)
    return (
      <p className="ui-caption">
        Monthly change unavailable: no retained price for{" "}
        {n.monthStart.missingPrices.join(", ")} at {n.monthStartDate}.
      </p>
    );
  const d = BigInt(n.deltaMinor),
    up = d >= 0n,
    pct =
      n.deltaBasisPoints === null
        ? ""
        : ` (${up ? "+" : "−"}${decimal(abs(BigInt(n.deltaBasisPoints)))}%)`;
  return (
    <p>
      <span className={"ui-pill " + (up ? "ui-positive" : "ui-negative")}>
        <span aria-hidden="true">{up ? "▲" : "▼"}</span>{" "}
        {hidden ? "••••••" : (up ? "+" : "") + amount(d) + pct}
      </span>{" "}
      <span className="ui-caption">since {n.monthStartDate} (UTC)</span>
    </p>
  );
}

function Bars({
  rows,
  hidden,
  label,
}: {
  rows: { key: string; label: string; value: bigint | null; note?: string }[];
  hidden?: boolean;
  label: string;
}) {
  const max = rows.reduce(
    (m, r) => (r.value !== null && abs(r.value) > m ? abs(r.value) : m),
    1n,
  );
  return (
    <ul className="ui-bars" aria-label={label}>
      {rows.map((r) => (
        <li key={r.key}>
          <span>{r.label}</span>
          <span className="ui-bar-track" aria-hidden="true">
            <span
              style={{
                width:
                  r.value === null
                    ? "0%"
                    : Number((abs(r.value) * 1000n) / max) / 10 + "%",
              }}
            />
          </span>
          <strong>
            {r.value === null ? r.note : hidden ? "••••••" : amount(r.value)}
          </strong>
        </li>
      ))}
    </ul>
  );
}

export function NetWorthHistory({
  p,
  hidden,
}: {
  p?: ProgressReport;
  hidden?: boolean;
}) {
  if (!p) return null;
  const point = (x: NetWorthPoint) =>
    x.complete ? BigInt(x.totalMinor) : null;
  const rows = [
    ...p.netWorth.history.map((h) => ({
      key: h.month ?? h.at,
      label: "Start of " + monthName(h.month ?? h.at.slice(0, 7)),
      value: point(h),
      note: "Price unavailable",
    })),
    {
      key: "now",
      label: "Today",
      value: BigInt(p.netWorth.current.totalMinor),
    },
  ];
  return (
    <>
      <Bars rows={rows} hidden={hidden} label="Personal net worth by month" />
      <p className="ui-caption">
        Replayed from posted journals and position events under today’s
        ownership. Holdings use the retained price at each month start;
        contributions are included, so this is not a return.
      </p>
    </>
  );
}

export function InterestSummary({ p }: { p?: ProgressReport }) {
  if (!p) return null;
  const i = p.interest;
  return (
    <>
      <p className="ui-kicker">LIFETIME INTEREST EARNED</p>
      <div className="ui-number">{amount(i.lifetimeMinor)}</div>
      <p className="ui-caption">
        {i.postings
          ? `${i.postings} native periodic posting(s), net of corrections${i.lastPostedOn ? "; last on " + i.lastPostedOn : ""}.`
          : "No interest has been credited yet. Interest posts only under an activated offer with retained consent."}
      </p>
      {i.months.length > 0 && (
        <Bars
          label="Interest credited by month"
          rows={i.months.map((m) => ({
            key: m.month,
            label: monthName(m.month),
            value: BigInt(m.amountMinor),
          }))}
        />
      )}
    </>
  );
}

export function Heatmap({ p }: { p: ProgressReport }) {
  const s = p.savings;
  const byDay = new Map(s.heatmap.map((d) => [d.date, d]));
  const max = s.heatmap.reduce(
    (m, d) => (BigInt(d.amountMinor) > m ? BigInt(d.amountMinor) : m),
    1n,
  );
  // Columns are ISO weeks starting Monday; the first column may begin before the window.
  const offset = (new Date(s.heatmapFrom + "T00:00:00Z").getUTCDay() + 6) % 7;
  const start = addDays(s.heatmapFrom, -offset);
  const cells: { date: string; level: number; inWindow: boolean }[] = [];
  for (let date = start; date <= p.asOf; date = addDays(date, 1)) {
    const d = byDay.get(date);
    const level = d
      ? Math.max(1, Math.ceil(Number((BigInt(d.amountMinor) * 4n) / max)))
      : 0;
    cells.push({ date, level, inWindow: date >= s.heatmapFrom });
  }
  return (
    <>
      <div
        className="ui-heatmap-scroll"
        role="img"
        aria-label={`Saving activity: ${s.heatmap.length} saving day(s) between ${s.heatmapFrom} and ${p.asOf}`}
      >
        <div className="ui-heatmap">
          {cells.map((c) => {
            const d = byDay.get(c.date);
            return (
              <span
                key={c.date}
                className="ui-heat-cell"
                data-level={c.level}
                data-outside={c.inWindow ? undefined : "true"}
                title={
                  c.date +
                  (d
                    ? ` · ${amount(d.amountMinor)} in ${d.count} contribution(s)`
                    : " · no saving")
                }
              />
            );
          })}
        </div>
      </div>
      <p className="ui-caption ui-heat-legend">
        Less
        {[0, 1, 2, 3, 4].map((l) => (
          <span key={l} className="ui-heat-cell" data-level={l} />
        ))}
        More · last 26 weeks, UTC days
      </p>
    </>
  );
}

function badgeProgress(b: Badge) {
  if (b.unit === "MINOR") return `${amount(b.progress)} of ${amount(b.target)}`;
  const unit = b.unit === "DAYS" ? "day" : b.unit === "MONTHS" ? "month" : "";
  return `${b.progress} of ${b.target}${unit ? " " + unit + (b.target === "1" ? "" : "s") : ""}`;
}
const badgeIcon: Record<string, string> = {
  FIRST_SAVE: "🌱",
  AUTO_SAVER: "🔁",
  STREAK_7: "🔥",
  GOAL_REACHED: "🎯",
  STREAK_30: "⚡",
  SAVED_1K: "💎",
  ROUNDUPS_100: "🪙",
  STREAK_6M: "🏆",
};

export function BadgeGrid({ badges }: { badges: Badge[] }) {
  return (
    <ul className="ui-badges">
      {badges.map((b) => (
        <li key={b.code} className={b.unlocked ? "unlocked" : "locked"}>
          <span className="ui-badge-icon" aria-hidden="true">
            {badgeIcon[b.code] ?? "✦"}
          </span>
          <div>
            <strong>{b.title}</strong>
            <p className="ui-caption">{b.description}</p>
            {b.unlocked ? (
              <p className="ui-badge-status">Unlocked {b.unlockedOn}</p>
            ) : (
              <>
                <Progress
                  value={b.progress}
                  max={b.target}
                  label={b.title + " progress"}
                />
                <p className="ui-caption">Locked · {badgeProgress(b)}</p>
              </>
            )}
          </div>
        </li>
      ))}
    </ul>
  );
}

function wrap(
  ctx: CanvasRenderingContext2D,
  text: string,
  x: number,
  y: number,
  width: number,
  line: number,
) {
  let row = "";
  for (const word of text.split(" ")) {
    const next = row ? row + " " + word : word;
    if (ctx.measureText(next).width > width && row) {
      ctx.fillText(row, x, y);
      row = word;
      y += line;
    } else row = next;
  }
  if (row) ctx.fillText(row, x, y);
}

/** Renders a balance-free card locally; nothing is uploaded or posted. */
function drawCard(b: Badge, streak: number): Promise<Blob | null> {
  const canvas = document.createElement("canvas");
  canvas.width = 1080;
  canvas.height = 1080;
  const ctx = canvas.getContext("2d");
  if (!ctx) return Promise.resolve(null);
  const font = '"Plus Jakarta Sans", "Inter", system-ui, sans-serif';
  ctx.fillStyle = "#1D6E4E";
  ctx.fillRect(0, 0, 1080, 1080);
  ctx.fillStyle = "#1E2D2F";
  ctx.beginPath();
  ctx.arc(980, 120, 300, 0, Math.PI * 2);
  ctx.fill();
  ctx.fillStyle = "#FFFFFF";
  ctx.font = `800 54px ${font}`;
  ctx.fillText("Piggybank", 90, 140);
  ctx.font = `160px ${font}`;
  ctx.fillText(badgeIcon[b.code] ?? "✦", 90, 420);
  ctx.font = `800 96px ${font}`;
  wrap(ctx, b.title, 90, 580, 900, 104);
  ctx.font = `500 44px ${font}`;
  ctx.fillStyle = "#E8F3EE";
  wrap(ctx, b.description, 90, 760, 900, 56);
  ctx.fillText(
    `Unlocked ${b.unlockedOn}${streak ? ` · ${streak}-day streak` : ""}`,
    90,
    900,
  );
  ctx.font = `500 30px ${font}`;
  ctx.fillText("Synthetic evaluation · no balances shown", 90, 1000);
  return new Promise((resolve) => canvas.toBlob(resolve, "image/png"));
}

export function MilestoneCard({ p }: { p: ProgressReport }) {
  const c = useConsumer();
  const unlocked = p.badges.filter((b) => b.unlocked);
  const [code, setCode] = useState(unlocked[0]?.code ?? "");
  const [image, setImage] = useState<{ url: string; blob: Blob }>();
  const [drawing, setDrawing] = useState(false);
  const badge = unlocked.find((b) => b.code === code) ?? unlocked[0];
  useEffect(() => () => image && URL.revokeObjectURL(image.url), [image]);
  if (!badge)
    return (
      <p className="ui-caption">
        Unlock your first badge to create a shareable milestone card.
      </p>
    );
  const name = `piggybank-${badge.code.toLowerCase()}.png`;
  const file = image && new File([image.blob], name, { type: "image/png" });
  const canShare =
    !!file &&
    typeof navigator.canShare === "function" &&
    navigator.canShare({ files: [file] });
  return (
    <form
      className="ui-form"
      onSubmit={(e) => {
        e.preventDefault();
        if (drawing) return;
        setDrawing(true);
        void drawCard(badge, p.savings.currentDays)
          .then((blob) => {
            if (!blob) return c.notify("This browser cannot render the card.");
            setImage({ url: URL.createObjectURL(blob), blob });
          })
          .finally(() => setDrawing(false));
      }}
    >
      <label>
        Milestone
        <select
          name="badge"
          value={badge.code}
          onChange={(e) => {
            setCode(e.target.value);
            setImage(undefined);
          }}
        >
          {unlocked.map((b) => (
            <option key={b.code} value={b.code}>
              {b.title}
            </option>
          ))}
        </select>
      </label>
      <p className="ui-caption">
        The card shows the milestone and date only — no balances, account or
        identity details. It is generated in this browser.
      </p>
      <button type="submit" disabled={drawing}>
        {drawing ? "Creating card…" : "Create milestone card"}
      </button>
      {image && (
        <>
          <img
            className="ui-milestone-preview"
            src={image.url}
            alt={`Milestone card: ${badge.title}, unlocked ${badge.unlockedOn}`}
          />
          <div className="ui-actions">
            {canShare && (
              <button
                type="button"
                onClick={() =>
                  void navigator
                    .share({ files: [file!], title: badge.title })
                    .catch(() => {})
                }
              >
                Share card
              </button>
            )}
            <a className="ui-pill" href={image.url} download={name}>
              Save image ↓
            </a>
          </div>
        </>
      )}
    </form>
  );
}

export function Milestones() {
  const c = useConsumer(),
    p = c.data.progress;
  const unlocked = useMemo(
    () => p?.badges.filter((b) => b.unlocked).length ?? 0,
    [p],
  );
  if (!p)
    return (
      <>
        <Head title="Milestones" sub="Your saving habits, from posted facts." />
        <Box>
          <p>Progress becomes available after your workspace loads.</p>
        </Box>
      </>
    );
  const s = p.savings;
  return (
    <>
      <Head
        title="Milestones"
        sub="Streaks and badges earned from your actual vault contributions."
      >
        <a className="text-link" href="#/save">
          ← Savings
        </a>
      </Head>
      <div className="ui-grid">
        <Box className="ui-hero">
          <p className="ui-kicker">CURRENT SAVING STREAK</p>
          <div className="ui-number">
            {s.currentDays} day{s.currentDays === 1 ? "" : "s"}
          </div>
          <p>
            {s.currentMonths} consecutive month(s) · longest {s.longestDays}{" "}
            day(s) and {s.longestMonths} month(s)
          </p>
          <p className="ui-caption">
            {s.lastSavingDate
              ? `Last contribution ${s.lastSavingDate}. A streak stays alive until the end of the next UTC day.`
              : "Make a vault deposit, automatic saving, round-up or rule saving to start."}
          </p>
        </Box>
        <Box title="Saved so far">
          <div className="ui-number">{amount(s.lifetimeContributedMinor)}</div>
          <p>
            {s.contributions} contribution(s) on {s.savingDays} day(s) ·{" "}
            {unlocked} of {p.badges.length} badges
          </p>
          <p className="ui-caption">
            Counts your own deposits, automatic savings, round-ups and merchant
            saving rules into any vault. Withdrawals never erase an earned
            badge.
          </p>
        </Box>
        <Box className="ui-wide" title="Saving activity">
          <Heatmap p={p} />
        </Box>
        <Box className="ui-wide" title="Badges">
          <BadgeGrid badges={p.badges} />
        </Box>
        <Box title="Share a milestone">
          <MilestoneCard p={p} />
        </Box>
      </div>
    </>
  );
}
