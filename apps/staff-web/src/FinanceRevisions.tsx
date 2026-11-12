import { useCallback, useEffect, useState } from "react";
import {
  Alert,
  Badge,
  message,
  money,
  short,
  time,
  useSession,
  value,
  type Schema,
} from "@piggybank/web-core";
import type { Access } from "./App";
const delta = (minorUnits: string) =>
  money({ minorUnits, currency: { code: "TST", scale: 2 } });
export function FinanceRevisions({
  access,
  reports,
  onPublished,
}: {
  access: Access;
  reports: Schema["FinanceReport"][];
  onPublished: () => Promise<void>;
}) {
  const { client } = useSession();
  const [items, setItems] = useState<Schema["ReportRevision"][]>([]);
  const [original, setOriginal] = useState(""),
    [replacement, setReplacement] = useState(""),
    [reason, setReason] = useState("");
  const [reviewed, setReviewed] = useState<Record<string, boolean>>({}),
    [busy, setBusy] = useState(false),
    [error, setError] = useState(""),
    [notice, setNotice] = useState("");
  const load = useCallback(async () => {
    try {
      setItems(
        await client.GET("/api/v1/staff/finance/report-revisions").then(value),
      );
      setError("");
    } catch (e) {
      setError(message(e));
    }
  }, [client]);
  useEffect(() => {
    void load();
    const timer = setInterval(() => void load(), 4000);
    return () => clearInterval(timer);
  }, [load]);
  async function propose() {
    setBusy(true);
    setError("");
    setNotice("");
    try {
      const body = {
        originalReportId: original,
        replacementReportId: replacement,
        reason,
      };
      const digest = await crypto.subtle.digest(
        "SHA-256",
        new TextEncoder().encode(JSON.stringify(body)),
      );
      const scope =
        "pb:report-revision:" +
        Array.from(new Uint8Array(digest), (b) =>
          b.toString(16).padStart(2, "0"),
        ).join("");
      const key = sessionStorage.getItem(scope) ?? crypto.randomUUID();
      sessionStorage.setItem(scope, key);
      await client
        .POST("/api/v1/staff/finance/report-revisions", {
          params: { header: { "Idempotency-Key": key } },
          body,
        })
        .then(value);
      sessionStorage.removeItem(scope);
      setNotice(
        "Revision accepted. Registration and independent approval are pending.",
      );
      await load();
    } catch (e) {
      setError(message(e));
    } finally {
      setBusy(false);
    }
  }
  async function act(
    r: Schema["ReportRevision"],
    action: "approval" | "publication",
  ) {
    setBusy(true);
    setError("");
    try {
      const options = {
        params: { path: { id: r.id } },
        body: { detailsDigest: r.detailsDigest },
      };
      if (action === "approval")
        await client
          .POST("/api/v1/staff/finance/report-revisions/{id}/approval", options)
          .then(value);
      else
        await client
          .POST(
            "/api/v1/staff/finance/report-revisions/{id}/publication",
            options,
          )
          .then(value);
      setNotice(
        action === "approval"
          ? "Exact report revision approved. The original maker can publish the link."
          : "Approved report revision published. Both frozen artifacts are retained.",
      );
      await load();
      await onPublished();
    } catch (e) {
      setError(message(e));
    } finally {
      setBusy(false);
    }
  }
  return (
    <section className="panel">
      <div className="panel-title">
        <h2>Controlled report revisions</h2>
        <button className="secondary" onClick={() => void load()}>
          Refresh report revisions
        </button>
      </div>
      <p>
        Choose two frozen snapshots for the same period. An independent Finance
        approver reviews both artifacts, their digests and the exact reason
        before the original maker publishes the link.
      </p>
      {error && <Alert>{error}</Alert>}
      {notice && (
        <p className="notice" role="status">
          {notice}
        </p>
      )}
      {access.capabilities.includes("finance.recover") && (
        <form
          className="configuration-form"
          onSubmit={(e) => {
            e.preventDefault();
            void propose();
          }}
        >
          <label htmlFor="revision-original">Original snapshot</label>
          <select
            id="revision-original"
            value={original}
            onChange={(e) => setOriginal(e.target.value)}
            required
          >
            <option value="">Select original snapshot</option>
            {reports.map((r) => (
              <option key={r.id} value={r.id}>
                {short(r.id)} · {r.range.fromDate} to {r.range.toDate} ·{" "}
                {r.completeness}
              </option>
            ))}
          </select>
          <label htmlFor="revision-replacement">Replacement snapshot</label>
          <select
            id="revision-replacement"
            value={replacement}
            onChange={(e) => setReplacement(e.target.value)}
            required
          >
            <option value="">Select replacement snapshot</option>
            {reports
              .filter((r) => r.id !== original)
              .map((r) => (
                <option key={r.id} value={r.id}>
                  {short(r.id)} · {r.range.fromDate} to {r.range.toDate} ·{" "}
                  {r.completeness}
                </option>
              ))}
          </select>
          <div className="recovery-field">
            <label htmlFor="report-revision-reason">
              Report revision reason
            </label>
            <textarea
              id="report-revision-reason"
              value={reason}
              onChange={(e) => setReason(e.target.value)}
              minLength={4}
              maxLength={240}
              required
            />
          </div>
          <button
            disabled={
              busy ||
              !original ||
              !replacement ||
              original === replacement ||
              reason.trim().length < 4
            }
          >
            Propose report revision
          </button>
        </form>
      )}
      {!items.length ? (
        <p>No report revisions proposed.</p>
      ) : (
        items.map((r) => {
          const own = r.makerPerson === access.personId;
          const approve =
            access.capabilities.includes("finance.approve") &&
            !own &&
            ["STAGED", "PENDING"].includes(r.state) &&
            Date.parse(r.expiresAt) > Date.now();
          const publish =
            access.capabilities.includes("finance.recover") &&
            own &&
            r.state === "APPROVED";
          return (
            <article
              className="case-row"
              data-testid={"report-revision-" + r.id}
              key={r.id}
            >
              <div>
                <h3>Report revision {short(r.id)}</h3>
                <Badge tone={r.state === "PUBLISHED" ? "good" : "attention"}>
                  {r.state.replaceAll("_", " ")}
                </Badge>
              </div>
              <p>{r.details.reason}</p>
              <dl className="evidence-details">
                <dt>Original artifact</dt>
                <dd>
                  <a href={"#/finance/reports/" + r.details.originalReportId}>
                    Open original snapshot {short(r.details.originalReportId)}
                  </a>
                </dd>
                <dt>Original digest</dt>
                <dd>{r.details.originalDigest}</dd>
                <dt>Replacement artifact</dt>
                <dd>
                  <a
                    href={"#/finance/reports/" + r.details.replacementReportId}
                  >
                    Open replacement snapshot{" "}
                    {short(r.details.replacementReportId)}
                  </a>
                </dd>
                <dt>Replacement digest</dt>
                <dd>{r.details.replacementDigest}</dd>
                <dt>Bound revision digest</dt>
                <dd>{r.detailsDigest}</dd>
                <dt>Debit / credit change</dt>
                <dd>
                  {delta(r.details.debitDelta)} / {delta(r.details.creditDelta)}
                </dd>
                <dt>Matched GL debit change</dt>
                <dd>{delta(r.details.matchedDebitDelta)}</dd>
                <dt>Source count change</dt>
                <dd>{r.details.sourceCountDelta}</dd>
                <dt>Independent checker</dt>
                <dd>{r.checkerPerson ?? "Awaiting approval"}</dd>
                <dt>Approval expiry</dt>
                <dd>{time(r.expiresAt)}</dd>
                <dt>Publication</dt>
                <dd>{r.publishedAt ? time(r.publishedAt) : "Pending"}</dd>
              </dl>
              {approve && (
                <>
                  <label className="revision-review">
                    <input
                      type="checkbox"
                      checked={reviewed[r.id] ?? false}
                      onChange={(e) =>
                        setReviewed((p) => ({ ...p, [r.id]: e.target.checked }))
                      }
                    />
                    I reviewed both frozen artifacts and the bound reason.
                  </label>
                  <button
                    disabled={busy || !reviewed[r.id]}
                    onClick={() => void act(r, "approval")}
                  >
                    Approve report revision
                  </button>
                </>
              )}
              {publish && (
                <button
                  disabled={busy}
                  onClick={() => void act(r, "publication")}
                >
                  Publish approved revision
                </button>
              )}
            </article>
          );
        })
      )}
    </section>
  );
}
