import { useId, useState, type ReactNode } from "react";
import { ratio, amount } from "./model";
export function UIIcon({ kind }: { kind: string }) {
  const paths: Record<string, ReactNode> = {
    home: (
      <>
        <path d="m3 10 9-7 9 7v10H3z" />
        <path d="M9 20v-7h6v7" />
      </>
    ),
    save: (
      <>
        <path d="M3 9h18v10H3zM6 9V5h12v4M10 13h4" />
      </>
    ),
    spend: (
      <>
        <path d="M4 4h16v16H4zM7 8h10M7 12h5M7 16h7" />
      </>
    ),
    invest: (
      <>
        <path d="m3 17 6-6 4 3 8-10M15 4h6v6" />
      </>
    ),
    profile: (
      <>
        <circle cx="12" cy="8" r="4" />
        <path d="M4 21v-2a8 8 0 0 1 16 0v2" />
      </>
    ),
    bell: (
      <>
        <path d="M5 17h14l-2-3V9a5 5 0 0 0-10 0v5zM10 21h4" />
      </>
    ),
  };
  return (
    <svg className="ui-inline-icon" viewBox="0 0 24 24" aria-hidden="true">
      {paths[kind] ?? paths.spend}
    </svg>
  );
}
export function Head({
  title,
  sub,
  children,
}: {
  title: string;
  sub?: string;
  children?: ReactNode;
}) {
  return (
    <div className="ui-page-head">
      <div>
        <h1>{title}</h1>
        {sub && <p>{sub}</p>}
      </div>
      {children}
    </div>
  );
}
export function Box({
  title,
  children,
  className = "",
}: {
  title?: string;
  children: ReactNode;
  className?: string;
}) {
  return (
    <section className={"ui-card " + className}>
      {title && <h2>{title}</h2>}
      {children}
    </section>
  );
}
export function Progress({
  value,
  max,
  label = "Savings goal progress",
}: {
  value: string;
  max: string;
  label?: string;
}) {
  const percent = Math.min(100, ratio(BigInt(value), BigInt(max)));
  return (
    <div
      className="ui-progress"
      role="progressbar"
      aria-label={label}
      aria-valuenow={percent}
      aria-valuemin={0}
      aria-valuemax={100}
    >
      <span style={{ width: percent + "%" }} />
    </div>
  );
}
export function Chart({ points }: { points: { at: string; value: string }[] }) {
  const [hover, setHover] = useState<number>();
  const gradient = useId();
  if (points.length < 2)
    return (
      <div className="ui-empty-chart">
        Your chart grows with recorded observations.
      </div>
    );
  const values = points.map((p) => BigInt(p.value)),
    low = values.reduce((a, b) => (a < b ? a : b)),
    high = values.reduce((a, b) => (a > b ? a : b)),
    span = high - low || 1n;
  const coords = values.map(
      (v, i) =>
        `${10 + (i * 580) / (values.length - 1)},${190 - Number(((v - low) * 160n) / span)}`,
    ),
    path = "M" + coords.join(" L");
  return (
    <>
      <svg
        className="ui-line-chart"
        viewBox="0 0 600 220"
        role="img"
        aria-label="Recorded portfolio value observations"
        onPointerMove={(e) => {
          const r = e.currentTarget.getBoundingClientRect();
          setHover(
            Math.max(
              0,
              Math.min(
                points.length - 1,
                Math.round(
                  ((e.clientX - r.left) / r.width) * (points.length - 1),
                ),
              ),
            ),
          );
        }}
        onPointerLeave={() => setHover(undefined)}
      >
        <defs>
          <linearGradient id={gradient} x1="0" y1="0" x2="0" y2="1">
            <stop offset="0" stopColor="currentColor" stopOpacity=".2" />
            <stop offset="1" stopColor="currentColor" stopOpacity="0" />
          </linearGradient>
        </defs>
        {[40, 100, 160].map((y) => (
          <path
            key={y}
            d={`M0 ${y}H600`}
            stroke="currentColor"
            opacity=".1"
            strokeDasharray="4 4"
          />
        ))}
        <path d={path + ` L590 210 L10 210 Z`} fill={`url(#${gradient})`} />
        <path d={path} fill="none" stroke="currentColor" strokeWidth="3" />
        {hover !== undefined && (
          <circle
            cx={coords[hover].split(",")[0]}
            cy={coords[hover].split(",")[1]}
            r="5"
            fill="currentColor"
          />
        )}
      </svg>
      <label className="ui-caption">
        Inspect recorded observation
        <input
          type="range"
          aria-label="Chart observation"
          min={0}
          max={points.length - 1}
          value={hover ?? points.length - 1}
          onChange={(e) => setHover(Number(e.target.value))}
        />
      </label>
      <p className="ui-caption" role="status">
        {hover !== undefined
          ? `${new Date(points[hover].at).toLocaleString()} · ${amount(points[hover].value)}`
          : "Recorded values include contributions; they are not a return calculation."}
      </p>
    </>
  );
}
export const colors = [
  "#1d6e4e",
  "#2ea478",
  "#f5b841",
  "#647478",
  "#688cc0",
  "#bd8bd0",
];
export function Allocation({
  items,
  label = "Invested",
}: {
  items: { name: string; value: bigint }[];
  label?: string;
}) {
  const total = items.reduce((n, p) => n + p.value, 0n);
  let edge = 0;
  const stops = items
    .filter((i) => i.value > 0n)
    .map((p, i) => {
      const start = edge;
      edge += ratio(p.value, total);
      return `${colors[i % colors.length]} ${start}% ${edge}%`;
    })
    .join(",");
  return (
    <>
      <div
        className="ui-donut"
        style={{
          background: total > 0n ? `conic-gradient(${stops})` : "var(--soft)",
        }}
      >
        <span>
          <small>{label}</small>
          {amount(total)}
        </span>
      </div>
      <div className="ui-legend">
        {items.map((i, n) => (
          <div className="ui-legend-row" key={i.name}>
            <span>
              <i
                className="ui-dot"
                style={{ background: colors[n % colors.length] }}
              />
              {i.name}
            </span>
            <strong>
              {ratio(i.value, total).toFixed(1)}%{" "}
              <small>{amount(i.value)}</small>
            </strong>
          </div>
        ))}
      </div>
    </>
  );
}
