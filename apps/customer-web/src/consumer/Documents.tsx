import { useCallback, useEffect, useState } from "react";
import { Alert, message, useSession, value } from "@piggybank/web-core";
import { Box, Head } from "./components";
import { amount } from "./model";
import { stableKey, doneKey } from "../lib/navigation";

/* WP-25 monthly statements. Every figure comes from an immutable server version; the
   browser only renders it. Retrieval always uses a fresh single-use reference. */
type Totals = {
  creditsMinor: string;
  debitsMinor: string;
  netMinor: string;
  feesMinor: string;
  interestMinor: string;
  correctionsMinor: string;
  itemCount: number;
  linkedCorrectionCount: number;
};
type Version = {
  id: string;
  version: number;
  reason: "INITIAL" | "LINKED_CORRECTION" | "SOURCE_CHANGED";
  supersedes: string | null;
  supersededBy: string | null;
  contentDigest: string;
  createdAt: string;
  retainUntil: string;
  disposed: boolean;
  openingMinor?: string;
  closingMinor?: string;
  totals?: Totals;
};
type Period = {
  periodStart: string;
  periodEnd: string;
  readiness: { ready: boolean; reasons: string[] };
  versions: Version[];
};
type Item = {
  batchId: string;
  businessDate: string;
  kind: string;
  deltaMinor: string;
  originalBatchId: string | null;
};
type Statement = {
  document: {
    id: string;
    accountId: string;
    periodStart: string;
    periodEnd: string;
    version: number;
    reason: string;
    openingMinor: string;
    closingMinor: string;
    totals: Totals;
    items: Item[];
    linkedCorrections: Item[];
    snapshotDigest: string;
    retention: { retainUntil: string; policyVersion: number };
    generatedAt: string;
  };
  contentDigest: string;
  supersededBy: string | null;
};
type DataRequest = {
  id: string;
  kind: "ACCESS" | "CORRECTION" | "DELETION" | "RESTRICTION" | "EXPORT";
  detail: string;
  state: "SUBMITTED" | "COMPLETED";
  submittedAt: string;
  outcome: string | null;
  items:
    | {
        recordClass: string;
        status: string;
        periodStart?: string;
        version?: number;
        note?: string;
      }[]
    | null;
  export: { statements: { versionId: string; periodStart: string }[] } | null;
  exportRetainUntil: string | null;
  exportDisposed: boolean;
};
type Documents = {
  asOfDate: string;
  accounts: { accountId: string; periods: Period[] }[];
  dataRequests: DataRequest[];
};

export const month = (day: string) =>
  new Date(day + "T00:00:00Z").toLocaleDateString("en", {
    month: "long",
    year: "numeric",
    timeZone: "UTC",
  });
export const kindLabel = (kind: string) =>
  kind.charAt(0) + kind.slice(1).toLowerCase().replaceAll("_", " ");
const reasonText: Record<string, string> = {
  PERIOD_OPEN: "Available after the month closes (UTC).",
  PERIODIC_FINALIZATION_PENDING:
    "Waiting for this month’s fees or interest to be finalized.",
};
const versionText: Record<Version["reason"], string> = {
  INITIAL: "Original",
  LINKED_CORRECTION: "Reissued with a later correction noted",
  SOURCE_CHANGED: "Reissued after a source change",
};
const outcomeText: Record<string, string> = {
  INCLUDED: "Included",
  DISPOSED: "Deleted",
  ALREADY_DISPOSED: "Already deleted",
  RESTRICTED_RETENTION: "Kept: retention period applies",
  RESTRICTED_LEGAL_HOLD: "Kept: legal hold applies",
  RESTRICTED_FINANCIAL_RECORD: "Kept: financial records can’t be edited",
  RESTRICTED_POLICY: "Kept: policy does not permit deletion",
  APPLIED: "Applied",
  RESTRICTED_LEGAL_BASIS: "Still sent: needed to run your account",
  RESTRICTED_EVIDENCE: "Available through a support review",
};

const recordName: Record<string, string> = {
  CORE_JOURNAL: "Account transaction history",
  MARKETING_USE: "Marketing messages",
  ANALYTICS_USE: "Analytics use",
  SERVICE_AND_SECURITY_NOTICES: "Service and security notices",
  PERSONAL_DATA: "Profile, contacts, consents and preferences",
  IDENTITY_EVIDENCE: "Identity evidence",
};
export function DocumentsPage() {
  const { client } = useSession(),
    [data, setData] = useState<Documents>(),
    [open, setOpen] = useState<Statement>(),
    [busy, setBusy] = useState(""),
    [error, setError] = useState(""),
    [notice, setNotice] = useState("");
  const load = useCallback(async () => {
    try {
      setData(
        (await client
          .GET("/api/v1/documents")
          .then(value)) as unknown as Documents,
      );
    } catch (e) {
      setError(message(e));
    }
  }, [client]);
  useEffect(() => {
    void load();
  }, [load]);
  async function act(label: string, run: () => Promise<void>) {
    setBusy(label);
    setError("");
    setNotice("");
    try {
      await run();
    } catch (e) {
      setError(message(e));
    } finally {
      setBusy("");
    }
  }
  async function link(id: string) {
    const r = (await client
      .POST("/api/v1/documents/statements/{id}/links", {
        params: { path: { id } },
      })
      .then(value)) as unknown as { token: string };
    return r.token;
  }
  const generate = (accountId: string, periodStart: string) =>
    act("generate:" + periodStart, async () => {
      const r = (await client
        .POST("/api/v1/accounts/{id}/statements", {
          params: { path: { id: accountId } },
          body: { periodStart },
        })
        .then(value)) as unknown as {
        state: string;
        readiness?: { reasons: string[] };
      };
      if (r.state === "NOT_READY")
        setNotice(
          (r.readiness?.reasons ?? []).map((x) => reasonText[x] ?? x).join(" "),
        );
      else setNotice(month(periodStart) + " statement is ready.");
      await load();
    });
  const view = (id: string) =>
    act("view:" + id, async () => {
      const token = await link(id);
      setOpen(
        (await client
          .GET("/api/v1/documents/downloads/{id}", {
            params: { path: { id: token }, query: { format: "json" } },
          })
          .then(value)) as unknown as Statement,
      );
      await load();
    });
  const pdf = (id: string) =>
    act("pdf:" + id, async () => {
      const token = await link(id);
      window.location.assign(
        "/api/v1/documents/downloads/" + token + "?format=pdf",
      );
    });
  async function request(form: HTMLFormElement) {
    const f = new FormData(form),
      body = {
        kind: String(f.get("kind")) as DataRequest["kind"],
        detail: String(f.get("detail") ?? "").trim(),
      },
      scope = "pb:data-request:" + JSON.stringify(body);
    await act("request", async () => {
      await client
        .POST("/api/v1/documents/data-requests", {
          params: { header: { "Idempotency-Key": stableKey(scope) } },
          body,
        })
        .then(value);
      doneKey(scope);
      form.reset();
      setNotice("Request received. We’ll show the outcome here.");
      await load();
    });
  }
  return (
    <>
      <Head
        title="Statements & documents"
        sub="Monthly statements built from your posted transactions, with every version kept."
      />
      {error && <Alert>{error}</Alert>}
      {notice && (
        <p className="toast" role="status">
          {notice}
        </p>
      )}
      {open && <StatementView s={open} close={() => setOpen(undefined)} />}
      {!data ? (
        <p className="ui-caption">Loading statements…</p>
      ) : !data.accounts.length ? (
        <Box title="No statements yet">
          <p>Statements appear here once you have an open account.</p>
        </Box>
      ) : (
        data.accounts.map((a) => (
          <Box
            key={a.accountId}
            title={"Account •••• " + a.accountId.slice(-4)}
          >
            <ul className="ui-statements" aria-label="Monthly statements">
              {a.periods
                .filter(
                  (p) =>
                    !p.readiness.reasons.includes("ACCOUNT_NOT_OPEN_IN_PERIOD"),
                )
                .map((p) => {
                  const current = p.versions[0],
                    older = p.versions.slice(1);
                  return (
                    <li key={p.periodStart} className="ui-statement">
                      <div className="ui-statement-head">
                        <strong>{month(p.periodStart)}</strong>
                        {current ? (
                          <span className="ui-pill">
                            {current.disposed
                              ? "Deleted under retention policy"
                              : "Version " + current.version}
                          </span>
                        ) : (
                          <span className="ui-caption">
                            {p.readiness.ready
                              ? "Ready to create"
                              : p.readiness.reasons
                                  .map((x) => reasonText[x] ?? x)
                                  .join(" ")}
                          </span>
                        )}
                      </div>
                      {current && !current.disposed && current.totals && (
                        <dl className="ui-statement-figures">
                          <div>
                            <dt>Opening</dt>
                            <dd>{amount(current.openingMinor ?? "0")}</dd>
                          </div>
                          <div>
                            <dt>Money in</dt>
                            <dd className="ui-positive">
                              {amount(current.totals.creditsMinor)}
                            </dd>
                          </div>
                          <div>
                            <dt>Money out</dt>
                            <dd>{amount(current.totals.debitsMinor)}</dd>
                          </div>
                          <div>
                            <dt>Closing</dt>
                            <dd>
                              <strong>
                                {amount(current.closingMinor ?? "0")}
                              </strong>
                            </dd>
                          </div>
                        </dl>
                      )}
                      {current && current.reason !== "INITIAL" && (
                        <p className="ui-caption">
                          {versionText[current.reason]}. Earlier versions are
                          kept below.
                        </p>
                      )}
                      <div className="ui-actions">
                        {current && !current.disposed ? (
                          <>
                            <button
                              className="secondary"
                              disabled={!!busy}
                              onClick={() => void view(current.id)}
                              aria-label={"View " + month(p.periodStart)}
                            >
                              View
                            </button>
                            <button
                              className="quiet"
                              disabled={!!busy}
                              onClick={() => void pdf(current.id)}
                              aria-label={
                                "Download " + month(p.periodStart) + " PDF"
                              }
                            >
                              Download PDF
                            </button>
                          </>
                        ) : (
                          !current &&
                          p.readiness.ready && (
                            <button
                              disabled={!!busy}
                              onClick={() =>
                                void generate(a.accountId, p.periodStart)
                              }
                              aria-label={
                                "Create " + month(p.periodStart) + " statement"
                              }
                            >
                              {busy === "generate:" + p.periodStart
                                ? "Creating…"
                                : "Create statement"}
                            </button>
                          )
                        )}
                      </div>
                      {older.length > 0 && (
                        <details>
                          <summary>Earlier versions ({older.length})</summary>
                          <ul className="ui-version-list">
                            {older.map((v) => (
                              <li key={v.id}>
                                <span>
                                  Version {v.version} · {versionText[v.reason]}{" "}
                                  · {v.createdAt.slice(0, 10)}
                                </span>
                                {v.disposed ? (
                                  <span className="ui-caption">Deleted</span>
                                ) : (
                                  <button
                                    className="quiet"
                                    disabled={!!busy}
                                    onClick={() => void view(v.id)}
                                    aria-label={
                                      "View " +
                                      month(p.periodStart) +
                                      " version " +
                                      v.version
                                    }
                                  >
                                    View
                                  </button>
                                )}
                              </li>
                            ))}
                          </ul>
                        </details>
                      )}
                    </li>
                  );
                })}
            </ul>
          </Box>
        ))
      )}
      <div className="ui-grid two">
        <Box title="Your data requests">
          <form
            className="ui-form"
            onSubmit={(e) => {
              e.preventDefault();
              void request(e.currentTarget);
            }}
          >
            <label>
              Request
              <select name="kind" defaultValue="ACCESS">
                <option value="ACCESS">Get a copy of my statements</option>
                <option value="CORRECTION">Correct a statement</option>
                <option value="DELETION">Delete my statements</option>
                <option value="EXPORT">Export my personal data</option>
                <option value="RESTRICTION">
                  Stop marketing and analytics use
                </option>
              </select>
            </label>
            <label>
              Details (optional)
              <textarea name="detail" maxLength={500} rows={2} />
            </label>
            <button disabled={!!busy}>Submit request</button>
          </form>
          <p className="ui-caption">
            Signed-in requests count as verified. Some records must be kept by
            law or because of a legal hold; we tell you exactly which.
          </p>
          {data?.dataRequests.map((r) => (
            <div className="ui-mini-card" key={r.id}>
              <strong>
                {
                  {
                    ACCESS: "Copy of statements",
                    CORRECTION: "Correction",
                    DELETION: "Deletion",
                    RESTRICTION: "Restriction",
                    EXPORT: "Personal data export",
                  }[r.kind]
                }
              </strong>{" "}
              <span className="ui-pill">
                {r.state === "SUBMITTED"
                  ? "In review"
                  : (r.outcome ?? "").replaceAll("_", " ").toLowerCase()}
              </span>
              <p>Submitted {r.submittedAt.slice(0, 10)}</p>
              {r.items && (
                <ul className="ui-version-list">
                  {r.items.map((i, n) => (
                    <li key={n}>
                      <span>
                        {i.recordClass === "STATEMENT"
                          ? month(i.periodStart ?? "") + " v" + i.version
                          : (recordName[i.recordClass] ??
                            "Account transaction history")}
                      </span>
                      <span className="ui-caption">
                        {outcomeText[i.status] ?? i.status}
                      </span>
                    </li>
                  ))}
                </ul>
              )}
              {r.export && (
                <p>
                  Copy lists {r.export.statements.length} statement
                  {r.export.statements.length === 1 ? "" : "s"}; available until{" "}
                  {r.exportRetainUntil}.
                </p>
              )}
              {r.exportDisposed && <p>The copy has expired and was deleted.</p>}
            </div>
          ))}
        </Box>
        <Box title="How we keep documents">
          <p>
            Statements cover one UTC calendar month. If a transaction is
            corrected later, we issue a new version that notes the correction
            and keep the original. Figures for a closed month never change.
          </p>
          <p>
            Each download link works once and expires after two minutes. We
            record every view and download.
          </p>
          <p className="ui-caption">
            Synthetic fixture policy: statements are kept seven years after the
            month ends. TST has no monetary value; not a tax form.
          </p>
        </Box>
      </div>
    </>
  );
}

function StatementView({ s, close }: { s: Statement; close: () => void }) {
  const d = s.document;
  return (
    <Box
      title={month(d.periodStart) + " statement"}
      className="ui-statement-view"
    >
      <div className="ui-statement-head">
        <span className="ui-pill">
          Version {d.version}
          {s.supersededBy ? " · replaced by a newer version" : " · current"}
        </span>
        <button className="quiet" onClick={close}>
          Close statement
        </button>
      </div>
      <dl className="ui-statement-figures">
        <div>
          <dt>Opening balance</dt>
          <dd>{amount(d.openingMinor)}</dd>
        </div>
        <div>
          <dt>Money in</dt>
          <dd className="ui-positive">{amount(d.totals.creditsMinor)}</dd>
        </div>
        <div>
          <dt>Money out</dt>
          <dd>{amount(d.totals.debitsMinor)}</dd>
        </div>
        <div>
          <dt>Fees · interest</dt>
          <dd>
            {amount(d.totals.feesMinor)} · {amount(d.totals.interestMinor)}
          </dd>
        </div>
        <div>
          <dt>Closing balance</dt>
          <dd>
            <strong>{amount(d.closingMinor)}</strong>
          </dd>
        </div>
      </dl>
      <div className="ui-table-scroll">
        <table className="ui-table">
          <caption className="visually-hidden">
            Transactions in {month(d.periodStart)}
          </caption>
          <thead>
            <tr>
              <th scope="col">Date</th>
              <th scope="col">Type</th>
              <th scope="col">Amount</th>
            </tr>
          </thead>
          <tbody>
            {d.items.map((i) => (
              <tr key={i.batchId}>
                <td>{i.businessDate}</td>
                <td>
                  {kindLabel(i.kind)}
                  {i.originalBatchId ? " (correction)" : ""}
                </td>
                <td className={BigInt(i.deltaMinor) < 0n ? "" : "ui-positive"}>
                  {amount(i.deltaMinor)}
                </td>
              </tr>
            ))}
            {!d.items.length && (
              <tr>
                <td colSpan={3}>No transactions this month.</td>
              </tr>
            )}
          </tbody>
        </table>
      </div>
      {d.linkedCorrections.length > 0 && (
        <div className="ui-mini-card">
          <strong>Corrections booked after this month</strong>
          <p>
            These appear in a later month’s statement and don’t change the
            figures above.
          </p>
          <ul className="ui-version-list">
            {d.linkedCorrections.map((i) => (
              <li key={i.batchId}>
                <span>{i.businessDate}</span>
                <span>{amount(i.deltaMinor)}</span>
              </li>
            ))}
          </ul>
        </div>
      )}
      <p className="ui-code">Content SHA-256 {s.contentDigest}</p>
      <p className="ui-caption">
        Created {d.generatedAt.slice(0, 10)} · kept until{" "}
        {d.retention.retainUntil} · synthetic TST, not a tax form
      </p>
    </Box>
  );
}
