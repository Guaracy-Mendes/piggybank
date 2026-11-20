import { useCallback, useEffect, useState } from "react";
import {
  Alert,
  Badge,
  message,
  short,
  useSession,
  value,
} from "@piggybank/web-core";

/* WP-26 operations monitor. Counts, alerts and repairs are native records; this page never writes balances. */
type Access = { capabilities: string[] };
type Snapshot = {
  id: string;
  asOf: string;
  digest: string;
  counts: {
    payments: {
      intentsByState: Record<string, number>;
      submissionsAwaitingOutcome: number;
      openRecoveryRequests: number;
      openReconciliationBreaks: number;
    };
    controls: { pendingChecks: number; overdueChecks: number };
    cases: {
      openByQueue: Record<string, number>;
      overdue: number;
      escalated: number;
    };
    integrations: {
      outbox: Record<string, { undelivered: number }>;
      deadLetters: Record<string, number>;
    };
    monitoring: { openAlerts: number };
    dataQuality: { openAlerts: number };
    notifications: Record<string, number>;
  };
  overdueCases: {
    id: string;
    kind: string;
    queue: string;
    nextAction: string;
    deadline: string;
  }[];
};
type Quality = {
  freshness: {
    status: "CURRENT" | "WITHIN_OBJECTIVE" | "STALE";
    lagSeconds: number;
    objectiveSeconds: number;
  };
  alerts: {
    id: string;
    checkCode: string;
    subject: string;
    state: "OPEN" | "CLEARED";
    nextAction: string;
    observation: { batchId?: string };
    caseId: string;
    caseState: string;
    firstDetectedAt: string;
    acknowledgedAt: string | null;
  }[];
};
type MonitoringAlert = {
  id: string;
  ruleCode: string;
  description: string;
  accountId: string;
  windowStart: string;
  windowEnd: string;
  evidence: { count: number; totalMinor: string };
  caseId: string;
  caseState: string;
  disposition: {
    disposition: string;
    rationale: string;
    makerPerson: string;
    checkerPerson: string | null;
  } | null;
};
type Notifications = {
  channels: { channel: string; available: boolean }[];
  counts: Record<string, number>;
  recent: {
    id: string;
    template: string;
    channel: string;
    recipient: string;
    state: string;
    attempts: number;
    lastError: string | null;
  }[];
};
type Paging = {
  ackSeconds: number;
  roster: { personId: string; tier: string; you: boolean }[];
  pages: {
    id: string;
    sourceKind: string;
    caseId: string | null;
    severity: string;
    summary: string;
    openedAt: string;
    deliveredAt: string | null;
    escalatedAt: string | null;
    acknowledgedAt: string | null;
    pagedYou: boolean;
    deliverySeconds: number | null;
  }[];
};
const sum = (o: Record<string, number>) =>
  Object.values(o).reduce((a, b) => a + b, 0);
const tst = (minor: string) => {
  const n = BigInt(minor);
  return (
    (n / 100n).toString() +
    "." +
    (n % 100n).toString().padStart(2, "0") +
    " TST"
  );
};
const words = (code: string) => code.replaceAll("_", " ").toLowerCase();

export function OperationsView({ access }: { access: Access }) {
  const { client } = useSession(),
    can = (c: string) => access.capabilities.includes(c),
    [snapshot, setSnapshot] = useState<Snapshot>(),
    [quality, setQuality] = useState<Quality>(),
    [alerts, setAlerts] = useState<MonitoringAlert[]>(),
    [notes, setNotes] = useState<Notifications>(),
    [trace, setTrace] = useState<Record<string, unknown>>(),
    [paging, setPaging] = useState<Paging>(),
    [error, setError] = useState(""),
    [notice, setNotice] = useState(""),
    [busy, setBusy] = useState(false);
  const load = useCallback(async () => {
    // Each section loads independently: a role without one view still sees the others.
    const failures: string[] = [];
    await Promise.all([
      client
        .GET("/api/v1/staff/data-quality")
        .then(value)
        .then((q) => setQuality(q as unknown as Quality))
        .catch((e) => failures.push(message(e))),
      can("case.review") || can("case.approve")
        ? client
            .GET("/api/v1/staff/monitoring/alerts")
            .then(value)
            .then((m) =>
              setAlerts((m as { alerts?: MonitoringAlert[] }).alerts ?? []),
            )
            .catch((e) => failures.push(message(e)))
        : Promise.resolve(),
      client
        .GET("/api/v1/staff/pages")
        .then(value)
        .then((p) => setPaging(p as unknown as Paging))
        .catch((e) => failures.push(message(e))),
      client
        .GET("/api/v1/staff/notifications")
        .then(value)
        .then((n) => setNotes(n as unknown as Notifications))
        .catch((e) => failures.push(message(e))),
    ]);
    setError(failures[0] ?? "");
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [client]);
  useEffect(() => {
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
  const record = () =>
    act(async () => {
      setSnapshot(
        (await client
          .GET("/api/v1/staff/operations-report")
          .then(value)) as unknown as Snapshot,
      );
      return "Operations snapshot recorded.";
    });
  return (
    <>
      <div className="page-title">
        <div>
          <p className="eyebrow">OPERATIONS</p>
          <h1>Monitor backlogs, alerts and data quality.</h1>
          <p>
            Native counts at one snapshot, owned alerts with next actions, and
            safe customer messages.
          </p>
        </div>
        <button className="secondary" onClick={() => void record()}>
          Record operations snapshot
        </button>
      </div>
      {error && <Alert>{error}</Alert>}
      {notice && (
        <p className="notice" role="status">
          {notice}
        </p>
      )}
      {paging && (
        <section className="panel" aria-label="On-call pages">
          <h2>On-call pages</h2>
          <p>
            On call:{" "}
            {paging.roster
              .map(
                (r) =>
                  `${r.tier.toLowerCase()} ${short(r.personId)}${r.you ? " (you)" : ""}`,
              )
              .join(" · ") || "nobody"}
            . Unacknowledged pages escalate to the secondary after{" "}
            {Math.round(paging.ackSeconds / 60)} minutes.
          </p>
          {!paging.pages.length && <p>No pages.</p>}
          {paging.pages.map((p) => (
            <article className="case-row" key={p.id}>
              <div>
                <strong>{p.summary}</strong>
                <Badge
                  tone={
                    p.acknowledgedAt
                      ? "neutral"
                      : p.severity === "CRITICAL"
                        ? "danger"
                        : "attention"
                  }
                >
                  {p.acknowledgedAt
                    ? "Acknowledged"
                    : p.escalatedAt
                      ? "Escalated"
                      : words(p.severity)}
                </Badge>
              </div>
              <p>
                {words(p.sourceKind)} · delivered{" "}
                {p.deliverySeconds === null
                  ? "not yet"
                  : `${p.deliverySeconds}s after detection`}
                {p.caseId ? (
                  <>
                    {" "}
                    · case <a href={"#/cases/" + p.caseId}>{short(p.caseId)}</a>
                  </>
                ) : null}
              </p>
              {!p.acknowledgedAt && p.pagedYou && (
                <button
                  disabled={busy}
                  onClick={() =>
                    void act(async () => {
                      await client
                        .POST("/api/v1/staff/pages/{id}/acknowledgement", {
                          params: { path: { id: p.id } },
                        })
                        .then(value);
                      return "Page acknowledged.";
                    })
                  }
                >
                  Acknowledge page
                </button>
              )}
            </article>
          ))}
        </section>
      )}
      {snapshot && (
        <section className="panel" aria-label="Operations snapshot">
          <h2>Snapshot {snapshot.asOf.slice(0, 19).replace("T", " ")} UTC</h2>
          <dl className="evidence-details">
            <dt>Payments awaiting controls or provider</dt>
            <dd>
              {sum(snapshot.counts.payments.intentsByState)} intents ·{" "}
              {snapshot.counts.payments.submissionsAwaitingOutcome} submissions
            </dd>
            <dt>Reconciliation breaks / recovery requests</dt>
            <dd>
              {snapshot.counts.payments.openReconciliationBreaks} /{" "}
              {snapshot.counts.payments.openRecoveryRequests}
            </dd>
            <dt>Pending / overdue control checks</dt>
            <dd>
              {snapshot.counts.controls.pendingChecks} /{" "}
              {snapshot.counts.controls.overdueChecks}
            </dd>
            <dt>Open cases (overdue · escalated)</dt>
            <dd>
              {sum(snapshot.counts.cases.openByQueue)} (
              {snapshot.counts.cases.overdue} ·{" "}
              {snapshot.counts.cases.escalated})
            </dd>
            <dt>Undelivered events / quarantined events</dt>
            <dd>
              {Object.values(snapshot.counts.integrations.outbox).reduce(
                (a, b) => a + b.undelivered,
                0,
              )}{" "}
              / {sum(snapshot.counts.integrations.deadLetters)}
            </dd>
            <dt>Open monitoring / data-quality alerts</dt>
            <dd>
              {snapshot.counts.monitoring.openAlerts} /{" "}
              {snapshot.counts.dataQuality.openAlerts}
            </dd>
            <dt>Snapshot digest</dt>
            <dd className="mono">{snapshot.digest}</dd>
          </dl>
          {snapshot.overdueCases.length > 0 && (
            <>
              <h3>Overdue cases</h3>
              {snapshot.overdueCases.map((c) => (
                <a className="case-row" key={c.id} href={"#/cases/" + c.id}>
                  <div>
                    <strong>
                      {c.kind} · {c.queue}
                    </strong>
                    <Badge tone="attention">{words(c.nextAction)}</Badge>
                  </div>
                  <small>Due {c.deadline.slice(0, 16)}</small>
                </a>
              ))}
            </>
          )}
        </section>
      )}
      {quality && (
        <section className="panel" aria-label="Data quality">
          <h2>Data quality</h2>
          <p>
            Transaction projection:{" "}
            <Badge
              tone={quality.freshness.status === "STALE" ? "danger" : "good"}
            >
              {words(quality.freshness.status)}
            </Badge>{" "}
            lag {quality.freshness.lagSeconds} s (objective{" "}
            {quality.freshness.objectiveSeconds} s). Spending decisions always
            use Core, never this projection.
          </p>
          {!quality.alerts.length && <p>No data-quality alerts.</p>}
          {quality.alerts.map((a) => (
            <article className="case-row" key={a.id}>
              <div>
                <strong>{words(a.checkCode)}</strong>
                <Badge tone={a.state === "OPEN" ? "attention" : "neutral"}>
                  {a.state === "OPEN"
                    ? "Open"
                    : a.acknowledgedAt
                      ? "Closed"
                      : "Cleared"}
                </Badge>
              </div>
              <p>
                {a.subject.length > 40 ? short(a.subject) : a.subject} · next:{" "}
                {words(a.nextAction)} · case{" "}
                <a href={"#/cases/" + a.caseId}>{short(a.caseId)}</a> (
                {a.caseState.toLowerCase()})
              </p>
              <div className="actions">
                {a.state === "OPEN" &&
                  a.checkCode === "MISSING_FINANCIAL_EVENT" &&
                  can("data.steward") &&
                  a.observation.batchId && (
                    <button
                      className="secondary"
                      disabled={busy}
                      onClick={() =>
                        void act(async () => {
                          const r = (await client
                            .POST("/api/v1/staff/data-quality/repairs", {
                              body: { batchId: a.observation.batchId! },
                            })
                            .then(value)) as unknown as {
                            rowsInserted: number;
                          };
                          return `Projection repaired: ${r.rowsInserted} read-model rows. No balance was written.`;
                        })
                      }
                    >
                      Repair projection
                    </button>
                  )}
                {a.state === "CLEARED" &&
                  !a.acknowledgedAt &&
                  can("data.steward") && (
                    <form
                      className="inline-form"
                      onSubmit={(e) => {
                        e.preventDefault();
                        const note = String(
                          new FormData(e.currentTarget).get("note"),
                        );
                        void act(async () => {
                          await client
                            .POST(
                              "/api/v1/staff/data-quality/alerts/{id}/acknowledgement",
                              {
                                params: { path: { id: a.id } },
                                body: { note },
                              },
                            )
                            .then(value);
                          return "Alert acknowledged and case closed.";
                        });
                      }}
                    >
                      <label>
                        Resolution note
                        <input
                          name="note"
                          required
                          minLength={3}
                          maxLength={500}
                        />
                      </label>
                      <button className="secondary" disabled={busy}>
                        Acknowledge
                      </button>
                    </form>
                  )}
              </div>
            </article>
          ))}
        </section>
      )}
      {alerts && (
        <section className="panel" aria-label="Transaction monitoring">
          <h2>Transaction monitoring</h2>
          {can("case.review") && (
            <form
              className="inline-form"
              onSubmit={(e) => {
                e.preventDefault();
                const f = new FormData(e.currentTarget);
                const from = new Date(String(f.get("from"))).toISOString(),
                  to = new Date(String(f.get("to"))).toISOString();
                void act(async () => {
                  const r = (await client
                    .POST("/api/v1/staff/monitoring/scans", {
                      body: { from, to },
                    })
                    .then(value)) as unknown as {
                    evaluated: number;
                    created: number;
                    existing: number;
                  };
                  return `Retrospective scan evaluated ${r.evaluated} facts: ${r.created} new and ${r.existing} existing alerts.`;
                });
              }}
            >
              <label>
                Scan from
                <input name="from" type="datetime-local" required />
              </label>
              <label>
                Scan to
                <input name="to" type="datetime-local" required />
              </label>
              <button className="secondary" disabled={busy}>
                Run retrospective scan
              </button>
            </form>
          )}
          {!alerts.length && <p>No monitoring alerts.</p>}
          {alerts.map((a) => (
            <article className="case-row" key={a.id}>
              <div>
                <strong>{words(a.ruleCode)}</strong>
                <Badge
                  tone={a.caseState === "CLOSED" ? "neutral" : "attention"}
                >
                  {a.caseState === "CLOSED"
                    ? words(a.disposition?.disposition ?? "closed")
                    : a.disposition
                      ? "Awaiting approval"
                      : "Open"}
                </Badge>
              </div>
              <p>
                {a.description} Account {short(a.accountId)} ·{" "}
                {a.evidence.count} facts · {tst(a.evidence.totalMinor)} ·{" "}
                {a.windowStart.slice(0, 16)} → {a.windowEnd.slice(0, 16)}
              </p>
              {a.disposition && <p>Rationale: {a.disposition.rationale}</p>}
              {!a.disposition && can("case.review") && (
                <form
                  className="inline-form"
                  onSubmit={(e) => {
                    e.preventDefault();
                    const f = new FormData(e.currentTarget);
                    void act(async () => {
                      await client
                        .POST(
                          "/api/v1/staff/monitoring/alerts/{id}/disposition",
                          {
                            params: { path: { id: a.id } },
                            body: {
                              disposition: String(f.get("disposition")) as
                                "NO_FURTHER_ACTION" | "REFER_FINANCIAL_CRIME",
                              rationale: String(f.get("rationale")),
                            },
                          },
                        )
                        .then(value);
                      return "Disposition proposed for independent approval.";
                    });
                  }}
                >
                  <label>
                    Disposition
                    <select name="disposition">
                      <option value="NO_FURTHER_ACTION">
                        No further action
                      </option>
                      <option value="REFER_FINANCIAL_CRIME">
                        Refer to financial crime
                      </option>
                    </select>
                  </label>
                  <label>
                    Rationale
                    <input
                      name="rationale"
                      required
                      minLength={3}
                      maxLength={1000}
                    />
                  </label>
                  <button disabled={busy}>Propose disposition</button>
                </form>
              )}
              {a.disposition &&
                !a.disposition.checkerPerson &&
                can("case.approve") && (
                  <button
                    disabled={busy}
                    onClick={() =>
                      void act(async () => {
                        await client
                          .POST(
                            "/api/v1/staff/monitoring/alerts/{id}/approval",
                            {
                              params: { path: { id: a.id } },
                            },
                          )
                          .then(value);
                        return "Disposition approved; case closed.";
                      })
                    }
                  >
                    Approve disposition
                  </button>
                )}
            </article>
          ))}
        </section>
      )}
      {notes && (
        <section className="panel" aria-label="Customer messages">
          <h2>Customer messages</h2>
          <p>
            Synthetic capture only. Delivered {notes.counts.DELIVERED ?? 0} ·
            pending {notes.counts.PENDING ?? 0} · failed{" "}
            {notes.counts.FAILED ?? 0} · suppressed by preference{" "}
            {notes.counts.SUPPRESSED ?? 0}.
          </p>
          {notes.channels.map((c) => (
            <form
              key={c.channel}
              className="inline-form"
              onSubmit={(e) => {
                e.preventDefault();
                const reason = String(
                  new FormData(e.currentTarget).get("reason"),
                );
                void act(async () => {
                  await client
                    .POST("/api/v1/staff/notifications/channels", {
                      body: {
                        channel: c.channel as "SYNTHETIC_EMAIL" | "IN_APP",
                        available: !c.available,
                        reason,
                      },
                    })
                    .then(value);
                  return `${words(c.channel)} marked ${c.available ? "unavailable" : "available"}.`;
                });
              }}
            >
              <span>
                <strong>{words(c.channel)}</strong>{" "}
                <Badge tone={c.available ? "good" : "danger"}>
                  {c.available ? "Available" : "Unavailable"}
                </Badge>
              </span>
              {can("events.replay") && (
                <>
                  <label>
                    Reason for {words(c.channel)}
                    <input
                      name="reason"
                      required
                      minLength={3}
                      maxLength={500}
                    />
                  </label>
                  <button className="secondary" disabled={busy}>
                    {c.available ? "Mark unavailable" : "Mark available"}
                  </button>
                </>
              )}
            </form>
          ))}
          <div className="table-scroll">
            <table className="data-table">
              <caption className="visually-hidden">
                Recent customer messages
              </caption>
              <thead>
                <tr>
                  <th scope="col">Template</th>
                  <th scope="col">Channel</th>
                  <th scope="col">State</th>
                  <th scope="col">Attempts</th>
                </tr>
              </thead>
              <tbody>
                {notes.recent.map((n) => (
                  <tr key={n.id}>
                    <td>{words(n.template)}</td>
                    <td>
                      {words(n.channel)} <small>{n.recipient}</small>
                    </td>
                    <td>
                      {n.state}
                      {n.lastError ? " · " + words(n.lastError) : ""}
                    </td>
                    <td>{n.attempts}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </section>
      )}
      <section className="panel" aria-label="Operation trace">
        <h2>Trace an operation</h2>
        <form
          className="inline-form"
          onSubmit={(e) => {
            e.preventDefault();
            const id = String(new FormData(e.currentTarget).get("id") || "")
              .trim()
              .toLowerCase();
            if (!/^[a-f0-9-]{36}$/.test(id)) {
              setError("Enter a full identifier.");
              return;
            }
            void act(async () => {
              setTrace(
                (await client
                  .GET("/api/v1/staff/traces/{id}", {
                    params: { path: { id } },
                  })
                  .then(value)) as unknown as Record<string, unknown>,
              );
            });
          }}
        >
          <label>
            Operation, batch, payment, account or case identifier
            <input name="id" required pattern="[a-fA-F0-9-]{36}" />
          </label>
          <button disabled={busy}>Trace</button>
        </form>
        {trace && (
          <ul className="plain-list" aria-label="Trace result">
            {(
              [
                "journal",
                "events",
                "cases",
                "monitoring",
                "notifications",
              ] as const
            ).map((k) => (
              <li key={k}>
                {k}:{" "}
                {Array.isArray(trace[k]) ? (trace[k] as unknown[]).length : 0}{" "}
                linked records
              </li>
            ))}
            <li>
              payment:{" "}
              {trace.payment ? "linked payment instruction found" : "none"}
            </li>
          </ul>
        )}
      </section>
    </>
  );
}
