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

/* WP-36 synthetic batch rail: supplement, outbound files and entries; operators can close the window early. */
type View = {
  supplement: {
    rail: string;
    cutoffUtc: string;
    settlementLagDays: number;
    perEntryMaxMinor: string;
    returnWindowDays: number;
  };
  returnReasons: { code: string; description: string }[];
  files: {
    id: string;
    entryCount: number;
    totalMinor: string;
    state: string;
    closedBy: string;
    createdAt: string;
  }[];
  entries: {
    id: string;
    amountMinor: string;
    state: string;
    settlementDate: string | null;
    reasonCode: string | null;
    lateReturn: boolean;
  }[];
};
const tst = (minor: string) => `TST ${(Number(minor) / 100).toFixed(2)}`;
export function BatchView() {
  const { client } = useSession();
  const [view, setView] = useState<View | null>(null);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const load = () =>
    client
      .GET("/api/v1/staff/batch")
      .then(value)
      .then((v) => setView(v as unknown as View))
      .catch((e) => setError(message(e)));
  useEffect(() => {
    void load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [client]);
  async function close() {
    try {
      const r = (await client
        .POST("/api/v1/staff/batch/window-closure")
        .then(value)) as unknown as { file: { entryCount: number } | null };
      setNotice(
        r.file
          ? `Window closed: ${r.file.entryCount} entries sealed into a file.`
          : "Nothing was queued.",
      );
      void load();
    } catch (e) {
      setError(message(e));
    }
  }
  return (
    <section aria-labelledby="batch-heading">
      <h1 id="batch-heading">Batch rail</h1>
      <p className="muted">
        Synthetic ACH-style credit rail. Actual rails need selection and
        certification (DEC-04/09).
      </p>
      {error && <Alert>{error}</Alert>}
      {notice && (
        <p className="notice" role="status">
          {notice}
        </p>
      )}
      {view && (
        <>
          <p>
            Cutoff {view.supplement.cutoffUtc} UTC · settles T+
            {view.supplement.settlementLagDays} business day · up to{" "}
            {tst(view.supplement.perEntryMaxMinor)} per entry · returns within{" "}
            {view.supplement.returnWindowDays} business days (
            {view.returnReasons.map((r) => r.code).join(", ")})
          </p>
          <button onClick={() => void close()}>Close window now</button>
          <h2>Files</h2>
          {view.files.length === 0 ? (
            <p className="muted">No files yet.</p>
          ) : (
            <ul>
              {view.files.map((f) => (
                <li key={f.id}>
                  {short(f.id)} · {f.entryCount} entries · {tst(f.totalMinor)} ·{" "}
                  <Badge tone={f.state === "SETTLED" ? "good" : "attention"}>
                    {f.state.toLowerCase()}
                  </Badge>{" "}
                  ·{" "}
                  {f.closedBy.startsWith("STAFF")
                    ? "closed by staff"
                    : "closed at cutoff"}{" "}
                  · {time(f.createdAt)}
                </li>
              ))}
            </ul>
          )}
          <h2>Entries</h2>
          {view.entries.length === 0 ? (
            <p className="muted">No entries yet.</p>
          ) : (
            <ul>
              {view.entries.map((e) => (
                <li key={e.id}>
                  {short(e.id)} · {tst(e.amountMinor)} · {e.state.toLowerCase()}
                  {e.reasonCode ? ` · ${e.reasonCode}` : ""}
                  {e.lateReturn ? " · late return under review" : ""}
                </li>
              ))}
            </ul>
          )}
        </>
      )}
    </section>
  );
}
