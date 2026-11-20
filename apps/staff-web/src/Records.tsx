import { useCallback, useEffect, useState } from "react";
import {
  Alert,
  Badge,
  message,
  short,
  useSession,
  value,
} from "@piggybank/web-core";

/* WP-25 records workspace: statement versions, single-use retrieval, legal holds,
   permitted disposal and data-subject requests. PostgreSQL rechecks every role. */
type Access = { capabilities: string[] };
type Version = {
  id: string;
  version: number;
  reason: string;
  supersededBy: string | null;
  contentDigest: string;
  createdAt: string;
  retainUntil: string;
  disposed: boolean;
  disposedAt: string | null;
  legalHold: boolean;
  disposalEvidenceDigest: string | null;
  closingMinor?: string;
};
type Period = {
  periodStart: string;
  readiness: { ready: boolean; reasons: string[] };
  versions: Version[];
};
type Hold = {
  id: string;
  scope: "ACCOUNT" | "PERSON";
  personId: string | null;
  reason: string;
  reference: string;
  placedBy: string;
  placedAt: string;
  active: boolean;
  releasedAt: string | null;
  releaseReason: string | null;
};
type AccessRow = {
  id: string;
  versionId: string | null;
  trust: string;
  action: string;
  outcome: string;
  reason: string;
  occurredAt: string;
};
type Request = {
  id: string;
  kind: string;
  detail: string;
  state: string;
  outcome: string | null;
  submittedAt: string;
  personId: string;
  personLegalHold: boolean;
  items:
    | {
        recordClass: string;
        status: string;
        periodStart?: string;
        version?: number;
      }[]
    | null;
  exportRetainUntil: string | null;
  exportDisposed: boolean;
};
type View = {
  accountId: string;
  owners: string[];
  asOfDate: string;
  periods: Period[];
  legalHolds: Hold[];
  access: AccessRow[];
  dataRequests: Request[];
};
type Opened = {
  contentDigest: string;
  supersededBy: string | null;
  document: {
    periodStart: string;
    version: number;
    openingMinor: string;
    closingMinor: string;
    totals: { creditsMinor: string; debitsMinor: string; itemCount: number };
    linkedCorrections: unknown[];
  };
};
const tst = (minor?: string) => {
  if (minor === undefined) return "—";
  const n = BigInt(minor),
    a = n < 0n ? -n : n;
  return (
    (n < 0n ? "-" : "") +
    (a / 100n).toString() +
    "." +
    (a % 100n).toString().padStart(2, "0") +
    " TST"
  );
};
const month = (day: string) =>
  new Date(day + "T00:00:00Z").toLocaleDateString("en", {
    month: "short",
    year: "numeric",
    timeZone: "UTC",
  });

export function RecordsView({
  access,
  accountId,
}: {
  access: Access;
  accountId?: string;
}) {
  const { client } = useSession(),
    can = (c: string) => access.capabilities.includes(c),
    [view, setView] = useState<View>(),
    [queue, setQueue] = useState<Request[]>(),
    [opened, setOpened] = useState<Opened>(),
    [error, setError] = useState(""),
    [notice, setNotice] = useState(""),
    [busy, setBusy] = useState(false);
  const load = useCallback(async () => {
    try {
      const [v, q] = await Promise.all([
        accountId
          ? client
              .GET("/api/v1/staff/documents/accounts/{id}", {
                params: { path: { id: accountId } },
              })
              .then(value)
          : Promise.resolve(undefined),
        can("records.manage") || can("records.approve")
          ? client.GET("/api/v1/staff/documents/data-requests").then(value)
          : Promise.resolve(undefined),
      ]);
      setView(v as unknown as View | undefined);
      setQueue(q as unknown as Request[] | undefined);
    } catch (e) {
      setError(message(e));
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [client, accountId]);
  useEffect(() => {
    setOpened(undefined);
    void load();
  }, [load]);
  async function act(run: () => Promise<string | void>) {
    setBusy(true);
    setError("");
    setNotice("");
    try {
      const done = await run();
      if (done) setNotice(done);
      await load();
    } catch (e) {
      setError(message(e));
    } finally {
      setBusy(false);
    }
  }
  const generate = (periodStart: string) =>
    act(async () => {
      const r = (await client
        .POST("/api/v1/staff/documents/accounts/{id}/statements", {
          params: { path: { id: accountId! } },
          body: { periodStart },
        })
        .then(value)) as unknown as {
        state: string;
        readiness?: { reasons: string[] };
      };
      return r.state === "NOT_READY"
        ? "Not ready: " + (r.readiness?.reasons ?? []).join(", ")
        : "Statement version ready for " + month(periodStart) + ".";
    });
  const open = (id: string) =>
    act(async () => {
      const link = (await client
        .POST("/api/v1/staff/documents/statements/{id}/links", {
          params: { path: { id } },
        })
        .then(value)) as unknown as { token: string };
      setOpened(
        (await client
          .GET("/api/v1/staff/documents/downloads/{id}", {
            params: { path: { id: link.token }, query: { format: "json" } },
          })
          .then(value)) as unknown as Opened,
      );
    });
  const dispose = (id: string, reason: string) =>
    act(async () => {
      const r = (await client
        .POST("/api/v1/staff/documents/statements/{id}/disposal", {
          params: { path: { id } },
          body: { reason },
        })
        .then(value)) as unknown as { status: string; retainUntil?: string };
      return r.status === "DISPOSED"
        ? "Object disposed; disposal evidence retained."
        : "Disposal restricted: " +
            r.status
              .replace("RESTRICTED_", "")
              .replaceAll("_", " ")
              .toLowerCase() +
            (r.retainUntil ? " until " + r.retainUntil : "") +
            ".";
    });
  const hold = (f: FormData) =>
    act(async () => {
      const person = String(f.get("person") || "");
      await client
        .POST("/api/v1/staff/documents/legal-holds", {
          params: { header: { "Idempotency-Key": crypto.randomUUID() } },
          body: person
            ? {
                scope: "PERSON",
                personId: person,
                reason: String(f.get("reason")),
                reference: String(f.get("reference")),
              }
            : {
                scope: "ACCOUNT",
                accountId: accountId!,
                reason: String(f.get("reason")),
                reference: String(f.get("reference")),
              },
        })
        .then(value);
      return "Legal hold placed. Disposal is blocked until an independent release.";
    });
  const release = (id: string, reason: string) =>
    act(async () => {
      await client
        .POST("/api/v1/staff/documents/legal-holds/{id}/release", {
          params: { path: { id } },
          body: { reason },
        })
        .then(value);
      return "Legal hold released.";
    });
  const processRequest = (id: string) =>
    act(async () => {
      const r = (await client
        .POST("/api/v1/staff/documents/data-requests/{id}/processing", {
          params: { path: { id } },
        })
        .then(value)) as unknown as { outcome: string };
      return (
        "Request processed: " +
        r.outcome.replaceAll("_", " ").toLowerCase() +
        "."
      );
    });
  return (
    <>
      <div className="page-title">
        <div>
          <p className="eyebrow">RECORDS</p>
          <h1>Statements, holds and retention.</h1>
          <p>
            Immutable statement versions from native Core journals, single-use
            retrieval and evidenced disposal.
          </p>
        </div>
        <button className="secondary" onClick={() => void load()}>
          Refresh records
        </button>
      </div>
      {error && <Alert>{error}</Alert>}
      {notice && (
        <p className="notice" role="status">
          {notice}
        </p>
      )}
      <section className="panel">
        <h2>Find an account</h2>
        <form
          className="inline-form"
          onSubmit={(e) => {
            e.preventDefault();
            const id = String(
              new FormData(e.currentTarget).get("account") || "",
            )
              .trim()
              .toLowerCase();
            if (/^[a-f0-9-]{36}$/.test(id)) location.hash = "/records/" + id;
            else setError("Enter a full account identifier.");
          }}
        >
          <label>
            Account identifier
            <input
              name="account"
              defaultValue={accountId}
              required
              pattern="[a-fA-F0-9-]{36}"
            />
          </label>
          <button>Open records</button>
        </form>
      </section>
      {opened && (
        <section className="panel" aria-label="Retrieved statement version">
          <h2>
            {month(opened.document.periodStart)} · version{" "}
            {opened.document.version}
          </h2>
          <Badge tone={opened.supersededBy ? "attention" : "good"}>
            {opened.supersededBy ? "Superseded" : "Current"}
          </Badge>
          <dl className="evidence-details">
            <dt>Opening</dt>
            <dd>{tst(opened.document.openingMinor)}</dd>
            <dt>Credits / debits</dt>
            <dd>
              {tst(opened.document.totals.creditsMinor)} /{" "}
              {tst(opened.document.totals.debitsMinor)}
            </dd>
            <dt>Closing</dt>
            <dd>{tst(opened.document.closingMinor)}</dd>
            <dt>Native facts / linked corrections</dt>
            <dd>
              {opened.document.totals.itemCount} /{" "}
              {opened.document.linkedCorrections.length}
            </dd>
            <dt>Content digest</dt>
            <dd className="mono">{opened.contentDigest}</dd>
          </dl>
          <p>
            This single-use workforce reference was consumed and recorded as
            access evidence.
          </p>
          <button className="secondary" onClick={() => setOpened(undefined)}>
            Close version
          </button>
        </section>
      )}
      {view && (
        <>
          <section className="panel" aria-label="Statement periods">
            <h2>Statement periods</h2>
            <p>
              Account <span className="mono">{view.accountId}</span> · owners{" "}
              {view.owners.map(short).join(", ") || "none"} · Core date{" "}
              {view.asOfDate}
            </p>
            <div className="table-scroll">
              <table className="data-table">
                <caption className="visually-hidden">
                  Statement versions by month
                </caption>
                <thead>
                  <tr>
                    <th scope="col">Month</th>
                    <th scope="col">Version</th>
                    <th scope="col">Closing</th>
                    <th scope="col">Retention</th>
                    <th scope="col">Actions</th>
                  </tr>
                </thead>
                <tbody>
                  {view.periods.flatMap((p) =>
                    p.versions.length
                      ? p.versions.map((v) => (
                          <tr key={v.id}>
                            <td>{month(p.periodStart)}</td>
                            <td>
                              v{v.version}{" "}
                              {v.reason.replaceAll("_", " ").toLowerCase()}{" "}
                              <Badge
                                tone={
                                  v.disposed
                                    ? "neutral"
                                    : v.supersededBy
                                      ? "attention"
                                      : "good"
                                }
                              >
                                {v.disposed
                                  ? "Disposed"
                                  : v.supersededBy
                                    ? "Superseded"
                                    : "Current"}
                              </Badge>
                              {v.legalHold && (
                                <Badge tone="attention">Legal hold</Badge>
                              )}
                            </td>
                            <td>{v.disposed ? "—" : tst(v.closingMinor)}</td>
                            <td>
                              {v.disposed
                                ? "Disposed " +
                                  (v.disposedAt ?? "").slice(0, 10)
                                : "Until " + v.retainUntil}
                            </td>
                            <td>
                              {!v.disposed && (
                                <button
                                  className="secondary"
                                  disabled={busy}
                                  onClick={() => void open(v.id)}
                                  aria-label={
                                    "Retrieve " +
                                    month(p.periodStart) +
                                    " version " +
                                    v.version
                                  }
                                >
                                  Retrieve
                                </button>
                              )}
                              {!v.disposed && can("records.manage") && (
                                <form
                                  className="inline-form"
                                  onSubmit={(e) => {
                                    e.preventDefault();
                                    void dispose(
                                      v.id,
                                      String(
                                        new FormData(e.currentTarget).get(
                                          "reason",
                                        ),
                                      ),
                                    );
                                  }}
                                >
                                  <label>
                                    Disposal reason
                                    <input
                                      name="reason"
                                      required
                                      minLength={3}
                                      maxLength={500}
                                    />
                                  </label>
                                  <button className="secondary" disabled={busy}>
                                    Request disposal
                                  </button>
                                </form>
                              )}
                            </td>
                          </tr>
                        ))
                      : p.readiness.reasons.includes(
                            "ACCOUNT_NOT_OPEN_IN_PERIOD",
                          )
                        ? []
                        : [
                            <tr key={p.periodStart}>
                              <td>{month(p.periodStart)}</td>
                              <td colSpan={3}>
                                {p.readiness.ready
                                  ? "No version yet"
                                  : "Not ready: " +
                                    p.readiness.reasons.join(", ")}
                              </td>
                              <td>
                                {p.readiness.ready && (
                                  <button
                                    disabled={busy}
                                    onClick={() => void generate(p.periodStart)}
                                    aria-label={
                                      "Generate " + month(p.periodStart)
                                    }
                                  >
                                    Generate
                                  </button>
                                )}
                              </td>
                            </tr>,
                          ],
                  )}
                </tbody>
              </table>
            </div>
          </section>
          <div className="queue-layout">
            <section className="panel" aria-label="Legal holds">
              <h2>Legal holds</h2>
              {can("records.manage") && (
                <form
                  className="stack-form"
                  onSubmit={(e) => {
                    e.preventDefault();
                    const form = e.currentTarget;
                    void hold(new FormData(form)).then(() => form.reset());
                  }}
                >
                  <label>
                    Scope
                    <select name="person" defaultValue="">
                      <option value="">This account</option>
                      {view.owners.map((o) => (
                        <option key={o} value={o}>
                          Owner {short(o)} (all accounts)
                        </option>
                      ))}
                    </select>
                  </label>
                  <label>
                    Reason
                    <input
                      name="reason"
                      required
                      minLength={3}
                      maxLength={500}
                    />
                  </label>
                  <label>
                    Case or matter reference
                    <input name="reference" required maxLength={64} />
                  </label>
                  <button disabled={busy}>Place legal hold</button>
                </form>
              )}
              {!view.legalHolds.length && (
                <p>No legal holds for this account or its owners.</p>
              )}
              {view.legalHolds.map((h) => (
                <article className="case-row" key={h.id}>
                  <div>
                    <strong>
                      {h.reference} ·{" "}
                      {h.scope === "ACCOUNT"
                        ? "Account"
                        : "Owner " + short(h.personId ?? "")}
                    </strong>
                    <Badge tone={h.active ? "attention" : "neutral"}>
                      {h.active ? "Active" : "Released"}
                    </Badge>
                  </div>
                  <p>{h.reason}</p>
                  <small>
                    Placed {h.placedAt.slice(0, 16)} by {short(h.placedBy)}
                    {h.releasedAt
                      ? " · released " +
                        h.releasedAt.slice(0, 16) +
                        ": " +
                        h.releaseReason
                      : ""}
                  </small>
                  {h.active && can("records.approve") && (
                    <form
                      className="inline-form"
                      onSubmit={(e) => {
                        e.preventDefault();
                        void release(
                          h.id,
                          String(new FormData(e.currentTarget).get("reason")),
                        );
                      }}
                    >
                      <label>
                        Release reason
                        <input
                          name="reason"
                          required
                          minLength={3}
                          maxLength={500}
                        />
                      </label>
                      <button className="secondary" disabled={busy}>
                        Release hold
                      </button>
                    </form>
                  )}
                </article>
              ))}
              <p>The person who placed a hold cannot release it.</p>
            </section>
            <section className="panel" aria-label="Access evidence">
              <h2>Access evidence</h2>
              <div className="table-scroll">
                <table className="data-table">
                  <caption className="visually-hidden">
                    Latest 50 document access records
                  </caption>
                  <thead>
                    <tr>
                      <th scope="col">When (UTC)</th>
                      <th scope="col">Action</th>
                      <th scope="col">Outcome</th>
                      <th scope="col">Reason</th>
                    </tr>
                  </thead>
                  <tbody>
                    {view.access.map((a) => (
                      <tr key={a.id}>
                        <td>{a.occurredAt.slice(0, 19).replace("T", " ")}</td>
                        <td>
                          {a.action} <small>{a.trust.toLowerCase()}</small>
                        </td>
                        <td>{a.outcome}</td>
                        <td>{a.reason}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            </section>
          </div>
        </>
      )}
      {queue && (
        <section className="panel" aria-label="Data requests">
          <h2>Data-subject requests</h2>
          <p>
            Outcomes are evaluated per record under the assigned retention
            policy and current legal holds.
          </p>
          {!queue.length && <p>No data requests.</p>}
          {queue.map((r) => (
            <article className="case-row" key={r.id}>
              <div>
                <strong>
                  {r.kind} · person {short(r.personId)}
                </strong>
                <Badge tone={r.state === "COMPLETED" ? "good" : "attention"}>
                  {r.state === "COMPLETED"
                    ? (r.outcome ?? "COMPLETED")
                    : "SUBMITTED"}
                </Badge>
                {r.personLegalHold && (
                  <Badge tone="attention">Legal hold</Badge>
                )}
              </div>
              <p>{r.detail || "No details provided."}</p>
              {r.items && (
                <ul className="plain-list">
                  {r.items.map((i, n) => (
                    <li key={n}>
                      {i.recordClass === "STATEMENT"
                        ? month(i.periodStart ?? "") + " v" + i.version
                        : i.recordClass}
                      : {i.status}
                    </li>
                  ))}
                </ul>
              )}
              <small>Submitted {r.submittedAt.slice(0, 16)}</small>
              {r.state === "SUBMITTED" && can("records.manage") && (
                <button
                  disabled={busy}
                  onClick={() => void processRequest(r.id)}
                >
                  Process request
                </button>
              )}
            </article>
          ))}
        </section>
      )}
    </>
  );
}
