import { useCallback, useEffect, useState } from "react";
import {
  Alert,
  Badge,
  message,
  short,
  useSession,
  value,
} from "@piggybank/web-core";

/* WP-28 security workspace: staff-assisted recovery with independent approval, access recertification and the
   findings register. Every decision is attributable; PostgreSQL rechecks roles and independence. */
type Recovery = {
  id: string;
  kind: string;
  channel: string;
  state: string;
  createdAt: string;
  personId: string;
  reason: string;
  verification: string;
  risk: { level: string; signals: string[] };
  requestedBy: string;
  decidedBy: string | null;
  decisionReason: string | null;
};
type Item = {
  grantId: string;
  personId: string;
  role: string;
  validUntil: string;
  decision: string | null;
  reason: string | null;
  self: boolean;
};
type Review = {
  id: string;
  openedAt: string;
  closedAt: string | null;
  items: Item[];
} | null;
type Finding = {
  id: string;
  reference: string;
  title: string;
  severity: string;
  area: string;
  description: string;
  status: string;
  disposition: string | null;
  reportedBy: string;
  disposedBy: string | null;
};
const words = (s: string) => s.replaceAll("_", " ").toLowerCase();

export function SecurityView() {
  const { client } = useSession();
  const [recoveries, setRecoveries] = useState<Recovery[]>();
  const [review, setReview] = useState<Review>();
  const [findings, setFindings] = useState<{
    releaseBlocking: number;
    findings: Finding[];
  }>();
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [busy, setBusy] = useState(false);
  const load = useCallback(async () => {
    const get = async (path: string) => {
      const r = await fetch(path, { credentials: "same-origin" });
      return r.ok ? r.json() : undefined;
    };
    setRecoveries(await get("/api/v1/staff/security/recoveries"));
    const rv = await get("/api/v1/staff/security/access-review");
    setReview(rv ? rv.review : undefined);
    setFindings(await get("/api/v1/staff/security/findings"));
  }, []);
  useEffect(() => {
    void load();
  }, [load]);
  async function act(run: () => Promise<string>) {
    setBusy(true);
    setError("");
    setNotice("");
    try {
      setNotice(await run());
      await load();
    } catch (e) {
      setError(message(e));
    } finally {
      setBusy(false);
    }
  }
  const post = (path: string, body?: unknown, key = false) =>
    // Paths are typed in the generated contract; bodies are validated server-side.
    (
      client.POST as unknown as (
        p: string,
        o: unknown,
      ) => Promise<{ data?: unknown; response: Response }>
    )(path, {
      ...(body === undefined ? {} : { body }),
      params: key ? { header: { "Idempotency-Key": crypto.randomUUID() } } : {},
    }).then((r) => value(r as never)) as Promise<Record<string, unknown>>;
  return (
    <>
      <div className="page-title">
        <div>
          <p className="eyebrow">SECURITY</p>
          <h1>Recovery, access reviews and findings.</h1>
          <p>
            Ownership verification, independent decisions and attributable
            evidence.
          </p>
        </div>
        <button className="secondary" onClick={() => void load()}>
          Refresh security
        </button>
      </div>
      {error && <Alert>{error}</Alert>}
      {notice && (
        <p className="notice" role="status">
          {notice}
        </p>
      )}
      {recoveries && (
        <section className="panel" aria-label="Account recovery">
          <h2>Account recovery</h2>
          <form
            className="stack-form"
            onSubmit={(e) => {
              e.preventDefault();
              const f = new FormData(e.currentTarget);
              void act(async () => {
                const r = await post(
                  "/api/v1/staff/security/recoveries",
                  {
                    personId: String(f.get("person")).trim(),
                    kind: String(f.get("kind")),
                    verification: String(f.get("verification")),
                    reason: String(f.get("reason")),
                  },
                  true,
                );
                return r.state === "PENDING_APPROVAL"
                  ? "Recovery needs independent security approval."
                  : "Recovery executed: every session was revoked.";
              });
            }}
          >
            <label>
              Customer person identifier
              <input name="person" required pattern="[a-fA-F0-9-]{36}" />
            </label>
            <label>
              Recovery
              <select name="kind" defaultValue="LOST_DEVICE">
                <option value="LOST_DEVICE">
                  Lost device: sign out every session
                </option>
                <option value="CREDENTIAL_RESET">
                  Credential reset: new password at next sign-in
                </option>
              </select>
            </label>
            <label>
              Ownership verification
              <select name="verification" defaultValue="VERIFIED_CONTACT">
                <option value="VERIFIED_CONTACT">
                  Verified contact confirmed
                </option>
                <option value="VERIFIED_EVIDENCE_REVIEW">
                  Evidence reviewed
                </option>
              </select>
            </label>
            <label>
              Recovery reason
              <input name="reason" required minLength={3} maxLength={500} />
            </label>
            <button disabled={busy}>Request recovery</button>
          </form>
          {recoveries.map((r) => (
            <article className="case-row" key={r.id}>
              <div>
                <strong>
                  {words(r.kind)} · person {short(r.personId)}
                </strong>
                <Badge
                  tone={
                    r.state === "EXECUTED"
                      ? "good"
                      : r.state === "REJECTED"
                        ? "neutral"
                        : "attention"
                  }
                >
                  {words(r.state)}
                </Badge>
                {r.risk.level === "HIGH" && (
                  <Badge tone="danger">High risk</Badge>
                )}
              </div>
              <p>
                {r.reason} · {words(r.verification)} ·{" "}
                {r.channel === "CUSTOMER_SELF"
                  ? "customer self-service"
                  : "staff-assisted"}
                {r.risk.signals.length
                  ? " · signals: " + r.risk.signals.map(words).join(", ")
                  : ""}
              </p>
              {r.decisionReason && <small>Decision: {r.decisionReason}</small>}
              {r.state === "PENDING_APPROVAL" && (
                <form
                  className="inline-form"
                  onSubmit={(e) => {
                    e.preventDefault();
                    const f = new FormData(e.currentTarget);
                    const approve =
                      (e.nativeEvent as SubmitEvent).submitter?.getAttribute(
                        "value",
                      ) === "approve";
                    void act(async () => {
                      const d = await post(
                        `/api/v1/staff/security/recoveries/${r.id}/decision`,
                        { approve, reason: String(f.get("reason")) },
                      );
                      return d.state === "EXECUTED"
                        ? "Approved and executed."
                        : "Recovery " + words(String(d.state)) + ".";
                    });
                  }}
                >
                  <label>
                    Decision reason
                    <input
                      name="reason"
                      required
                      minLength={3}
                      maxLength={500}
                    />
                  </label>
                  <button name="decision" value="approve" disabled={busy}>
                    Approve recovery
                  </button>
                  <button
                    className="secondary"
                    name="decision"
                    value="reject"
                    disabled={busy}
                  >
                    Reject recovery
                  </button>
                </form>
              )}
            </article>
          ))}
        </section>
      )}
      {review !== undefined && (
        <section className="panel" aria-label="Access recertification">
          <h2>Access recertification</h2>
          {!review || review.closedAt ? (
            <button
              disabled={busy}
              onClick={() =>
                void act(
                  async () => (
                    await post("/api/v1/staff/security/access-review"),
                    "Access review opened."
                  ),
                )
              }
            >
              Open access review
            </button>
          ) : null}
          {review && (
            <>
              <p>
                Review opened {review.openedAt.slice(0, 16)}
                {review.closedAt
                  ? " · closed " + review.closedAt.slice(0, 16)
                  : " · in progress"}
              </p>
              <div className="table-scroll">
                <table className="data-table">
                  <caption className="visually-hidden">
                    Role grants under review
                  </caption>
                  <thead>
                    <tr>
                      <th scope="col">Role</th>
                      <th scope="col">Person</th>
                      <th scope="col">Decision</th>
                    </tr>
                  </thead>
                  <tbody>
                    {review.items.map((i) => (
                      <tr key={i.grantId}>
                        <td>{words(i.role)}</td>
                        <td>
                          {short(i.personId)}
                          {i.self ? " (you)" : ""}
                        </td>
                        <td>
                          {i.decision ? (
                            <>
                              {words(i.decision)} <small>{i.reason}</small>
                            </>
                          ) : i.self ? (
                            "Another reviewer must decide"
                          ) : (
                            <span className="inline-form">
                              <button
                                className="secondary"
                                disabled={busy}
                                aria-label={`Keep ${words(i.role)} for ${short(i.personId)}`}
                                onClick={() =>
                                  void act(
                                    async () => (
                                      await post(
                                        `/api/v1/staff/security/access-review/${review.id}/items`,
                                        {
                                          grantId: i.grantId,
                                          decision: "KEEP",
                                          reason:
                                            "Still required for current duties",
                                        },
                                      ),
                                      "Access kept."
                                    ),
                                  )
                                }
                              >
                                Keep
                              </button>
                              <button
                                className="secondary"
                                disabled={busy}
                                aria-label={`Revoke ${words(i.role)} for ${short(i.personId)}`}
                                onClick={() =>
                                  void act(
                                    async () => (
                                      await post(
                                        `/api/v1/staff/security/access-review/${review.id}/items`,
                                        {
                                          grantId: i.grantId,
                                          decision: "REVOKE",
                                          reason: "No longer required",
                                        },
                                      ),
                                      "Access revoked."
                                    ),
                                  )
                                }
                              >
                                Revoke
                              </button>
                            </span>
                          )}
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            </>
          )}
        </section>
      )}
      {findings && (
        <section className="panel" aria-label="Security findings">
          <h2>Security findings</h2>
          <p>
            Release-blocking open findings:{" "}
            <strong>{findings.releaseBlocking}</strong>. High and critical
            findings must be resolved; only lower severities may be accepted.
          </p>
          <form
            className="stack-form"
            onSubmit={(e) => {
              e.preventDefault();
              const form = e.currentTarget;
              const f = new FormData(form);
              void act(async () => {
                await post("/api/v1/staff/security/findings", {
                  reference: String(f.get("reference")),
                  title: String(f.get("title")),
                  severity: String(f.get("severity")),
                  area: String(f.get("area")),
                  description: String(f.get("description")),
                });
                form.reset();
                return "Finding recorded.";
              });
            }}
          >
            <label>
              Reference (SF-000)
              <input name="reference" required pattern="SF-[0-9]{3,6}" />
            </label>
            <label>
              Finding title
              <input name="title" required minLength={3} maxLength={200} />
            </label>
            <label>
              Severity
              <select name="severity" defaultValue="MEDIUM">
                {["LOW", "MEDIUM", "HIGH", "CRITICAL"].map((x) => (
                  <option key={x} value={x}>
                    {words(x)}
                  </option>
                ))}
              </select>
            </label>
            <label>
              Area
              <input name="area" required minLength={2} maxLength={80} />
            </label>
            <label>
              Description
              <input
                name="description"
                required
                minLength={3}
                maxLength={2000}
              />
            </label>
            <button disabled={busy}>Record finding</button>
          </form>
          {findings.findings.map((x) => (
            <article className="case-row" key={x.id}>
              <div>
                <strong>
                  {x.reference} · {x.title}
                </strong>
                <Badge
                  tone={
                    x.status === "OPEN"
                      ? ["HIGH", "CRITICAL"].includes(x.severity)
                        ? "danger"
                        : "attention"
                      : "good"
                  }
                >
                  {words(x.severity)} · {words(x.status)}
                </Badge>
              </div>
              <p>
                {x.area} · {x.description}
              </p>
              {x.disposition && <small>Disposition: {x.disposition}</small>}
              {x.status === "OPEN" && (
                <form
                  className="inline-form"
                  onSubmit={(e) => {
                    e.preventDefault();
                    const f = new FormData(e.currentTarget);
                    void act(async () => {
                      await post(
                        `/api/v1/staff/security/findings/${x.id}/disposition`,
                        {
                          status: String(f.get("status")),
                          disposition: String(f.get("disposition")),
                          evidenceRef:
                            String(f.get("evidence") || "") || undefined,
                        },
                      );
                      return "Finding disposed.";
                    });
                  }}
                >
                  <label>
                    Outcome for {x.reference}
                    <select name="status" defaultValue="RESOLVED">
                      <option value="RESOLVED">Resolved</option>
                      {!["HIGH", "CRITICAL"].includes(x.severity) && (
                        <option value="ACCEPTED_RISK">Accepted risk</option>
                      )}
                    </select>
                  </label>
                  <label>
                    Disposition for {x.reference}
                    <input
                      name="disposition"
                      required
                      minLength={3}
                      maxLength={2000}
                    />
                  </label>
                  <label>
                    Evidence reference for {x.reference}
                    <input name="evidence" maxLength={200} />
                  </label>
                  <button className="secondary" disabled={busy}>
                    Record disposition
                  </button>
                </form>
              )}
            </article>
          ))}
        </section>
      )}
    </>
  );
}
