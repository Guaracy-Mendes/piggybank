import { useEffect, useState } from "react";
import {
  Alert,
  Badge,
  message,
  short,
  time,
  useSession,
  value,
} from "@piggybank/web-core";

/* WP-37 governed warehouse (one local adapter): datasets and watermarks, backfill, purpose-checked snapshots with
   lineage, completeness flags and Core control totals, reproduction and row tracing. */
type Totals = { entries: number; creditMinor: string; debitMinor: string };
type Snapshot = {
  id: string;
  transform: string;
  transformVersion: number;
  transformDigest: string;
  adapter: string;
  purpose: string;
  businessFrom: string;
  businessTo: string;
  factCut: number;
  masterCut: string;
  status: "COMPLETE" | "INCOMPLETE";
  completeness: { code: string; dataset?: string }[];
  control: {
    core: Totals;
    input: Totals;
    excluded: Totals & { parties: number; reason: string };
    output: Totals;
    reconciled: boolean;
  };
  inputs: {
    dataset: string;
    schemaVersions: number[];
    runs: string[];
    facts: number;
  }[];
  resultDigest: string;
  supersedes: string | null;
  supersededBy: string | null;
  explanation: { corrections: unknown[]; masterChanges: number } | null;
  createdAt: string;
  rows?: {
    rowNo: number;
    dimensions: Record<string, string>;
    entries: number;
    creditMinor: string;
    debitMinor: string;
  }[];
};
type View = {
  adapters: { code: string; status: string; description: string }[];
  purposes: {
    code: string;
    mandatory: boolean;
    permitted: boolean;
    description: string;
  }[];
  datasets: {
    code: string;
    source: string;
    facts: number;
    conflicts: number;
    rejects: number;
    schemas: { version: number }[];
    watermark: { lastRunAt: string | null; stale: boolean } | null;
  }[];
  transforms: { code: string; version: number; definition: string }[];
  snapshots: Snapshot[];
  denials: {
    id: string;
    purpose: string;
    reason: string;
    requestedBy: string;
    deniedAt: string;
  }[];
};
type Trace = {
  facts: {
    seq: number;
    sourceRef: string;
    schemaVersion: number;
    runId: string;
    amountMinor: string;
  }[];
};
const tst = (minor: string | number) =>
  `TST ${(Number(minor) / 100).toFixed(2)}`;
const words = (code: string) => code.toLowerCase().replaceAll("_", " ");
const today = () => new Date().toISOString().slice(0, 10);

export function AnalyticsView() {
  const { client } = useSession();
  const [view, setView] = useState<View | null>(null);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [selected, setSelected] = useState<Snapshot | null>(null);
  const [trace, setTrace] = useState<{ row: number; data: Trace } | null>(null);
  const [version, setVersion] = useState(1);
  const [purpose, setPurpose] = useState("FINANCIAL_REPORTING");
  const [from, setFrom] = useState(today);
  const [to, setTo] = useState(today);
  const [dataset, setDataset] = useState("ledger_entries");
  const [busy, setBusy] = useState(false);
  const load = () =>
    client
      .GET("/api/v1/staff/analytics")
      .then(value)
      .then((v) => setView(v as unknown as View))
      .catch((e) => setError(message(e)));
  useEffect(() => {
    void load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [client]);
  async function open(id: string) {
    setTrace(null);
    try {
      const s = await client
        .GET("/api/v1/staff/analytics/snapshots/{id}", {
          params: { path: { id } },
        })
        .then(value);
      setSelected(s as unknown as Snapshot);
    } catch (e) {
      setError(message(e));
    }
  }
  async function create() {
    setBusy(true);
    setError("");
    setNotice("");
    try {
      const r = await client.POST("/api/v1/staff/analytics/snapshots", {
        body: {
          transform: "daily_ledger_movement",
          version,
          purpose: purpose as "FINANCIAL_REPORTING",
          from,
          to,
        },
      });
      if (r.response.status === 403) {
        setSelected(null);
        setNotice(
          "Request denied and recorded. The reason is listed under recorded denials.",
        );
      } else {
        const s = (await value(r)) as unknown as Snapshot;
        setTrace(null);
        setSelected(s);
        setNotice(
          s.status === "COMPLETE"
            ? "Snapshot created. Every source is current and totals reconcile with Core."
            : "Snapshot created with completeness flags. Review them before using the figures.",
        );
      }
      void load();
    } catch (e) {
      setError(message(e));
    } finally {
      setBusy(false);
    }
  }
  async function backfill() {
    setBusy(true);
    setError("");
    setNotice("");
    try {
      const r = (await client
        .POST("/api/v1/staff/analytics/ingestions", {
          body: {
            dataset: dataset as "ledger_entries",
            from: new Date(Date.now() - 24 * 3600 * 1000).toISOString(),
            to: new Date().toISOString(),
          },
        })
        .then(value)) as unknown as {
        read: number;
        inserted: number;
        duplicates: number;
        conflicts: number;
      };
      setNotice(
        `Backfill read ${r.read} source rows: ${r.inserted} new, ${r.duplicates} already ingested, ${r.conflicts} conflicts.`,
      );
      void load();
    } catch (e) {
      setError(message(e));
    } finally {
      setBusy(false);
    }
  }
  async function reproduce(id: string) {
    try {
      const r = (await client
        .POST("/api/v1/staff/analytics/snapshots/{id}/reproduction", {
          params: { path: { id } },
        })
        .then(value)) as unknown as { reproduced: boolean };
      setNotice(
        r.reproduced
          ? "Reproduced: recomputing from the stored cuts gives the same digest."
          : "Not reproduced: the recomputed digest differs. Investigate before use.",
      );
    } catch (e) {
      setError(message(e));
    }
  }
  async function traceRow(id: string, row: number) {
    try {
      const data = (await client
        .GET("/api/v1/staff/analytics/snapshots/{id}/rows/{row}/trace", {
          params: { path: { id, row } },
        })
        .then(value)) as unknown as Trace;
      setTrace({ row, data });
    } catch (e) {
      setError(message(e));
    }
  }
  const steward = view?.purposes.some((p) => p.permitted) ?? false;
  return (
    <section aria-labelledby="analytics-heading">
      <h1 id="analytics-heading">Governed warehouse</h1>
      <p className="muted">
        One local warehouse adapter with synthetic TST data. Analytical products
        need selection (DEC-05/06/08). Reports read Core, they never post to it.
      </p>
      {error && <Alert>{error}</Alert>}
      {notice && (
        <p className="notice" role="status">
          {notice}
        </p>
      )}
      {view && (
        <>
          <h2>Datasets</h2>
          <div className="table-scroll">
            <table>
              <thead>
                <tr>
                  <th>Dataset</th>
                  <th>Schema versions</th>
                  <th>Facts</th>
                  <th>Last ingestion</th>
                  <th>Quarantine</th>
                </tr>
              </thead>
              <tbody>
                {view.datasets.map((d) => (
                  <tr key={d.code}>
                    <td>{d.code}</td>
                    <td>{d.schemas.map((s) => `v${s.version}`).join(", ")}</td>
                    <td>{d.facts}</td>
                    <td>
                      {d.watermark?.lastRunAt
                        ? time(d.watermark.lastRunAt)
                        : "never"}{" "}
                      {d.watermark?.stale && (
                        <Badge tone="attention">stale</Badge>
                      )}
                    </td>
                    <td>
                      {d.conflicts} conflicts · {d.rejects} rejected
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          <form
            className="card"
            onSubmit={(e) => {
              e.preventDefault();
              void backfill();
            }}
          >
            <h2>Backfill</h2>
            <p className="muted">
              Re-reads the last 24 hours of the source. Economic duplicates are
              skipped and the run is recorded.
            </p>
            <label>
              Dataset to backfill
              <select
                value={dataset}
                onChange={(e) => setDataset(e.target.value)}
              >
                {view.datasets.map((d) => (
                  <option key={d.code} value={d.code}>
                    {d.code}
                  </option>
                ))}
              </select>
            </label>
            <button disabled={busy}>Run backfill</button>
          </form>
          <form
            className="card"
            onSubmit={(e) => {
              e.preventDefault();
              void create();
            }}
          >
            <h2>Create a snapshot</h2>
            <label>
              Transform version
              <select
                value={version}
                onChange={(e) => setVersion(Number(e.target.value))}
              >
                {view.transforms.map((t) => (
                  <option key={t.version} value={t.version}>
                    {t.code} v{t.version}
                  </option>
                ))}
              </select>
            </label>
            <label>
              Purpose
              <select
                value={purpose}
                onChange={(e) => setPurpose(e.target.value)}
              >
                {view.purposes.map((p) => (
                  <option key={p.code} value={p.code}>
                    {words(p.code)}
                    {p.mandatory ? " (mandatory)" : ""}
                  </option>
                ))}
              </select>
            </label>
            <label>
              Business date from
              <input
                type="date"
                value={from}
                onChange={(e) => setFrom(e.target.value)}
                required
              />
            </label>
            <label>
              Business date to
              <input
                type="date"
                value={to}
                onChange={(e) => setTo(e.target.value)}
                required
              />
            </label>
            <button disabled={busy || !steward}>Create snapshot</button>
          </form>
          {selected && (
            <SnapshotDetail
              snapshot={selected}
              trace={trace}
              onTrace={(row) => void traceRow(selected.id, row)}
              onReproduce={() => void reproduce(selected.id)}
            />
          )}
          <h2>Recent snapshots</h2>
          {view.snapshots.length === 0 ? (
            <p className="muted">No snapshots yet.</p>
          ) : (
            <ul>
              {view.snapshots.map((s) => (
                <li key={s.id}>
                  <button className="secondary" onClick={() => void open(s.id)}>
                    Open snapshot {short(s.id)}
                  </button>{" "}
                  {words(s.purpose)} · {s.businessFrom} to {s.businessTo} ·{" "}
                  <Badge tone={s.status === "COMPLETE" ? "good" : "attention"}>
                    {words(s.status)}
                  </Badge>{" "}
                  · {time(s.createdAt)}
                </li>
              ))}
            </ul>
          )}
          <h2>Recorded denials</h2>
          {view.denials.length === 0 ? (
            <p className="muted">No denied requests.</p>
          ) : (
            <ul>
              {view.denials.map((d) => (
                <li key={d.id}>
                  {words(d.purpose)} · {words(d.reason)} · {d.requestedBy} ·{" "}
                  {time(d.deniedAt)}
                </li>
              ))}
            </ul>
          )}
        </>
      )}
    </section>
  );
}

function SnapshotDetail({
  snapshot: s,
  trace,
  onTrace,
  onReproduce,
}: {
  snapshot: Snapshot;
  trace: { row: number; data: Trace } | null;
  onTrace: (row: number) => void;
  onReproduce: () => void;
}) {
  const totals: [string, Totals][] = [
    ["Core journal", s.control.core],
    ["Warehouse input", s.control.input],
    ["Excluded by purpose", s.control.excluded],
    ["Report output", s.control.output],
  ];
  return (
    <article className="card" aria-labelledby="snapshot-heading">
      <h2 id="snapshot-heading">
        Snapshot {short(s.id)}{" "}
        <Badge tone={s.status === "COMPLETE" ? "good" : "attention"}>
          {words(s.status)}
        </Badge>
      </h2>
      <p>
        {s.transform} v{s.transformVersion} (definition{" "}
        {s.transformDigest.slice(0, 12)}) · {words(s.purpose)} · adapter{" "}
        {s.adapter} · business dates {s.businessFrom} to {s.businessTo} · fact
        cut {s.factCut} · master data as of {time(s.masterCut)}
      </p>
      {s.supersedes && <p>Supersedes snapshot {short(s.supersedes)}.</p>}
      {s.explanation && s.explanation.corrections.length > 0 && (
        <p>
          Includes {s.explanation.corrections.length} linked correction
          {s.explanation.corrections.length === 1 ? "" : "s"} since the previous
          snapshot. Earlier facts are unchanged.
        </p>
      )}
      <h3>Completeness</h3>
      {s.completeness.length === 0 ? (
        <p>No flags: sources are current and totals reconcile.</p>
      ) : (
        <ul>
          {s.completeness.map((f, i) => (
            <li key={i}>
              {words(f.code)}
              {f.dataset ? ` · ${f.dataset}` : ""}
            </li>
          ))}
        </ul>
      )}
      <h3>Control totals</h3>
      <div className="table-scroll">
        <table>
          <thead>
            <tr>
              <th>Stage</th>
              <th>Entries</th>
              <th>Credits</th>
              <th>Debits</th>
            </tr>
          </thead>
          <tbody>
            {totals.map(([label, t]) => (
              <tr key={label}>
                <td>{label}</td>
                <td>{t.entries}</td>
                <td>{tst(t.creditMinor)}</td>
                <td>{tst(t.debitMinor)}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      <p>
        {s.control.reconciled
          ? "Reconciled: Core equals warehouse input, and input equals output plus exclusions."
          : "Not reconciled: see the completeness flags."}
      </p>
      <h3>Lineage</h3>
      <ul>
        {s.inputs.map((i) => (
          <li key={i.dataset}>
            {i.dataset}: {i.facts} facts · schema{" "}
            {i.schemaVersions.map((v) => `v${v}`).join(", ")} · {i.runs.length}{" "}
            ingestion run{i.runs.length === 1 ? "" : "s"}
          </li>
        ))}
      </ul>
      <button className="secondary" onClick={onReproduce}>
        Reproduce from stored cuts
      </button>
      <h3>Rows</h3>
      <div className="table-scroll">
        <table>
          <thead>
            <tr>
              <th>Business date</th>
              <th>Posting kind</th>
              <th>Party</th>
              <th>Entries</th>
              <th>Credits</th>
              <th>Debits</th>
              <th>Lineage</th>
            </tr>
          </thead>
          <tbody>
            {(s.rows ?? []).map((r) => (
              <tr key={r.rowNo}>
                <td>{r.dimensions.businessDate}</td>
                <td>{words(r.dimensions.postingKind)}</td>
                <td>
                  {r.dimensions.partyRef.includes(":") ||
                  r.dimensions.partyRef === "UNRESOLVED"
                    ? words(r.dimensions.partyRef)
                    : short(r.dimensions.partyRef)}
                </td>
                <td>{r.entries}</td>
                <td>{tst(r.creditMinor)}</td>
                <td>{tst(r.debitMinor)}</td>
                <td>
                  <button
                    className="secondary"
                    onClick={() => onTrace(r.rowNo)}
                  >
                    Trace row {r.rowNo}
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      {trace && (
        <>
          <h3>Row {trace.row} sources</h3>
          <ul>
            {trace.data.facts.map((f) => (
              <li key={f.seq}>
                {f.sourceRef.split(":")[0]} · schema v{f.schemaVersion} · run{" "}
                {short(f.runId)} · {tst(f.amountMinor)}
              </li>
            ))}
          </ul>
        </>
      )}
    </article>
  );
}
