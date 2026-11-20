import { useEffect, useState } from "react";
import {
  Alert,
  Badge,
  message,
  time,
  useSession,
  value,
} from "@piggybank/web-core";

/* WP-41 outcome KPIs (local capacity template): every KPI shows its numerator, denominator and exclusions. Snapshots
   are local synthetic evidence; production outcomes and ratified targets need DEC-07 and DEC-12. */
type Kpi = {
  code: string;
  definition: string;
  numerator: number | null;
  denominator: number | null;
  value: string | null;
  target: string;
  meetsTarget: boolean | null;
  status: string;
  exclusions?: Record<string, number>;
  breakdown?: Record<string, number>;
  slaBasis?: string;
  deadline?: string;
};
type Snapshot = {
  id: string;
  digest: string;
  windowFrom: string;
  windowTo: string;
  createdAt: string;
  requestedBy: string;
  evidence: string;
  kpis: Kpi[];
};
const words = (code: string) => code.toLowerCase().replaceAll("_", " ");
const pairs = (o?: Record<string, number>) =>
  o
    ? Object.entries(o)
        .map(([k, v]) => `${k.replace(/([A-Z])/g, " $1").toLowerCase()} ${v}`)
        .join(", ")
    : "";

export function OutcomesView() {
  const { client } = useSession();
  const [snapshots, setSnapshots] = useState<Snapshot[]>([]);
  const [days, setDays] = useState("30");
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const load = () =>
    client
      .GET("/api/v1/staff/analytics/kpi-snapshots")
      .then(value)
      .then((v) => setSnapshots(v as unknown as Snapshot[]))
      .catch((e) => setError(message(e)));
  useEffect(() => {
    void load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [client]);
  async function compute() {
    setError("");
    setNotice("");
    const n = Number(days);
    if (!Number.isInteger(n) || n < 1 || n > 93) {
      setError("Choose a window of 1 to 93 days.");
      return;
    }
    const to = new Date(Date.now() + 60_000);
    const from = new Date(to.getTime() - n * 86_400_000);
    try {
      await client
        .POST("/api/v1/staff/analytics/kpi-snapshots", {
          body: { from: from.toISOString(), to: to.toISOString() },
        })
        .then(value);
      setNotice("KPI snapshot recorded.");
      void load();
    } catch (e) {
      setError(message(e));
    }
  }
  const latest = snapshots[0];
  return (
    <section aria-labelledby="outcomes-heading">
      <h1 id="outcomes-heading">Outcome KPIs</h1>
      <p className="muted">
        Computed from local synthetic records. These are not production
        outcomes; targets and SLAs need ratification (DEC-07, DEC-12).
      </p>
      {error && <Alert>{error}</Alert>}
      {notice && (
        <p className="notice" role="status">
          {notice}
        </p>
      )}
      <form
        onSubmit={(e) => {
          e.preventDefault();
          void compute();
        }}
      >
        <label>
          Window (days)
          <input
            inputMode="numeric"
            value={days}
            onChange={(e) => setDays(e.target.value)}
          />
        </label>
        <button>Compute KPI snapshot</button>
      </form>
      {latest && (
        <>
          <h2>Latest snapshot</h2>
          <p>
            {time(latest.windowFrom)} to {time(latest.windowTo)} · by{" "}
            {latest.requestedBy} · {time(latest.createdAt)} ·{" "}
            <Badge>{words(latest.evidence)}</Badge>
          </p>
          <div className="table-scroll">
            <table data-testid="kpi-table">
              <thead>
                <tr>
                  <th>KPI</th>
                  <th>Numerator / denominator</th>
                  <th>Value</th>
                  <th>Target</th>
                  <th>Details</th>
                </tr>
              </thead>
              <tbody>
                {latest.kpis.map((k) => (
                  <tr key={k.code}>
                    <td>
                      <strong>{k.code}</strong>
                      <br />
                      {k.definition}
                    </td>
                    <td>
                      {k.numerator === null
                        ? "—"
                        : `${k.numerator} / ${k.denominator ?? "—"}`}
                    </td>
                    <td>
                      {k.value ?? words(k.status)}{" "}
                      {k.meetsTarget === true && (
                        <Badge tone="good">meets target</Badge>
                      )}
                      {k.meetsTarget === false && (
                        <Badge tone="attention">below target</Badge>
                      )}
                    </td>
                    <td>{k.target}</td>
                    <td>
                      {[
                        pairs(k.exclusions) &&
                          `Excluded: ${pairs(k.exclusions)}`,
                        pairs(k.breakdown) &&
                          `Breakdown: ${pairs(k.breakdown)}`,
                        k.slaBasis && `SLA: ${words(k.slaBasis)}`,
                        k.deadline && `Deadline: ${words(k.deadline)}`,
                      ]
                        .filter(Boolean)
                        .join(" · ")}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </>
      )}
      <h2>Snapshot history</h2>
      {snapshots.length === 0 ? (
        <p className="muted">No KPI snapshots yet.</p>
      ) : (
        <ul>
          {snapshots.map((s) => (
            <li key={s.id}>
              {time(s.createdAt)} · {time(s.windowFrom)} to {time(s.windowTo)} ·
              digest {s.digest.slice(0, 12)}
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
