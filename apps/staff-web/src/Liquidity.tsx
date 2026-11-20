import { useEffect, useState } from "react";
import {
  Alert,
  Badge,
  message,
  money,
  short,
  time,
  useSession,
  value,
} from "@piggybank/web-core";

/* WP-40 liquidity forecasting and controlled finance automation. Forecasts use only fresh signed liquidity evidence as
   confirmed funds; a breach restricts new external submissions and opens an owned Finance case. Automated prefunding
   needs a Finance maker and a different approver, and Core books it once within approved limits. */
type Output = {
  status: string;
  outcome: string;
  flags: string[];
  confirmedAvailableMinor: string | null;
  expectedOutflowMinor: string;
  highOutflowMinor: string;
  projectedLowMinor: string | null;
  minBufferMinor: string;
  prefundingMinor: string | null;
  daysWithData: number;
  backtest: {
    days: number;
    observedDays: number;
    modelMaeMinor: string;
    naiveMaeMinor: string;
    bandCoverage: string;
  } | null;
};
type Forecast = {
  id: string;
  policyVersion: number;
  status: string;
  outcome: string;
  output: Output;
  automatic: boolean;
  requestedBy: string;
  createdAt: string;
  escalation: { kind: string; caseId: string; reused: boolean } | null;
  restrictionId: string | null;
  action: { id: string; amountMinor: string; state: string } | null;
  reproductions: { matched: boolean; at: string }[];
};
type PolicyDocument = {
  model: { lookbackDays: number; k: number; minHistoryDays: number };
  limits: {
    minBufferMinor: string;
    maxActionMinor: string;
    maxDailyActionMinor: string;
    maxForecastAgeSeconds: number;
  };
  automation: {
    proposePrefunding: boolean;
    targetBufferMinor: string;
    scheduleMinutes: number;
  };
};
type View = {
  activeVersion: number;
  restricted: boolean;
  policies: {
    version: number;
    document: PolicyDocument;
    proposedBy: string | null;
    outcome: string | null;
    decidedBy: string | null;
  }[];
  forecasts: Forecast[];
  restrictions: {
    id: string;
    actions: string[];
    caseId: string;
    createdAt: string;
    active: boolean;
    lifts: {
      id: string;
      proposedBy: string;
      outcome: string | null;
      decidedBy: string | null;
    }[];
  }[];
  actions: {
    id: string;
    amountMinor: string;
    state: string;
    confirmedBy: string | null;
    decidedBy: string | null;
    batchId: string | null;
    createdAt: string;
  }[];
  outcomes: {
    forecastId: string;
    businessDate: string;
    expectedOutflowMinor: string;
    actualOutflowMinor: string;
    withinBand: boolean;
  }[];
};
const tst = (minor: string | null) =>
  minor === null
    ? "not confirmed"
    : money({ minorUnits: minor, currency: { code: "TST", scale: 2 } });
const words = (code: string) => code.toLowerCase().replaceAll("_", " ");
const tone = (outcome: string) =>
  outcome === "WITHIN_LIMITS" ? "good" : "attention";

export function LiquidityView() {
  const { client } = useSession();
  const [view, setView] = useState<View | null>(null);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [buffer, setBuffer] = useState("");
  const load = () =>
    client
      .GET("/api/v1/staff/liquidity/forecasts")
      .then(value)
      .then((v) => setView(v as unknown as View))
      .catch((e) => setError(message(e)));
  useEffect(() => {
    void load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [client]);
  async function act(run: () => Promise<unknown>, done: string) {
    setError("");
    setNotice("");
    try {
      await run();
      setNotice(done);
      void load();
    } catch (e) {
      setError(message(e));
    }
  }
  const id = (x: string) => ({ params: { path: { id: x } } });
  const latest = view?.forecasts[0];
  const active = view?.policies.find((p) => p.version === view.activeVersion);
  return (
    <section aria-labelledby="liquidity-heading">
      <h1 id="liquidity-heading">Liquidity forecast</h1>
      <p className="muted">
        Confirmed funds come only from fresh signed liquidity evidence. History
        is the governed warehouse at a recorded cut, so every forecast can be
        reproduced. Real treasury feeds and limits need DEC-03/06/07/12.
      </p>
      {error && <Alert>{error}</Alert>}
      {notice && (
        <p className="notice" role="status">
          {notice}
        </p>
      )}
      <button
        onClick={() =>
          void act(
            () => client.POST("/api/v1/staff/liquidity/forecasts").then(value),
            "Forecast recorded.",
          )
        }
      >
        Run forecast now
      </button>
      {view && (
        <>
          {view.restricted ? (
            <p className="notice" role="status" data-testid="restriction">
              <Badge tone="attention">Restricted</Badge> New external payment
              submissions are paused by a liquidity breach.
            </p>
          ) : (
            <p data-testid="restriction">
              <Badge tone="good">Open</Badge> External payment submissions are
              not restricted.
            </p>
          )}
          {latest && (
            <article className="card" data-testid="latest-forecast">
              <h2>Latest forecast</h2>
              <p>
                <Badge tone={tone(latest.outcome)}>
                  {words(latest.outcome)}
                </Badge>{" "}
                <Badge>{words(latest.status)}</Badge> · policy v
                {latest.policyVersion} ·{" "}
                {latest.automatic ? "scheduled" : `by ${latest.requestedBy}`} ·{" "}
                {time(latest.createdAt)}
              </p>
              <dl className="facts">
                <dt>Confirmed available</dt>
                <dd data-testid="confirmed-available">
                  {tst(latest.output.confirmedAvailableMinor)}
                </dd>
                <dt>Expected outflow (high band)</dt>
                <dd>
                  {tst(latest.output.expectedOutflowMinor)} (
                  {tst(latest.output.highOutflowMinor)})
                </dd>
                <dt>Projected low against buffer</dt>
                <dd data-testid="projected-low">
                  {tst(latest.output.projectedLowMinor)} against{" "}
                  {tst(latest.output.minBufferMinor)}
                </dd>
                <dt>History</dt>
                <dd>{latest.output.daysWithData} days with data</dd>
                {latest.output.backtest && (
                  <>
                    <dt>Backtest against actual outflow</dt>
                    <dd>
                      {latest.output.backtest.days} days (
                      {latest.output.backtest.observedDays} observed): model
                      error {latest.output.backtest.modelMaeMinor}, previous-day
                      baseline {latest.output.backtest.naiveMaeMinor} minor
                      units; band coverage{" "}
                      {Math.round(
                        Number(latest.output.backtest.bandCoverage) * 100,
                      )}
                      %. Local synthetic history only.
                    </dd>
                  </>
                )}
              </dl>
              {latest.output.flags.length > 0 && (
                <p data-testid="forecast-flags">
                  Flags: {latest.output.flags.map(words).join(", ")}
                </p>
              )}
              {latest.escalation && (
                <p>
                  Escalated as {words(latest.escalation.kind)} to Finance case{" "}
                  {short(latest.escalation.caseId)}
                  {latest.escalation.reused ? " (open case reused)" : ""}.
                </p>
              )}
            </article>
          )}
          <h2>Restrictions</h2>
          {view.restrictions.length === 0 ? (
            <p className="muted">No restrictions.</p>
          ) : (
            <ul>
              {view.restrictions.map((r) => (
                <li key={r.id}>
                  {r.actions.map(words).join(", ")} · case {short(r.caseId)} ·{" "}
                  {time(r.createdAt)} ·{" "}
                  {r.active ? (
                    <Badge tone="attention">active</Badge>
                  ) : (
                    <Badge tone="good">lifted</Badge>
                  )}
                  {r.lifts.map((l) => (
                    <span key={l.id}>
                      {" "}
                      · lift proposed by {l.proposedBy}
                      {l.outcome ? (
                        `, ${words(l.outcome)} by ${l.decidedBy ?? ""}`
                      ) : (
                        <>
                          {" "}
                          <button
                            onClick={() =>
                              void act(
                                () =>
                                  client
                                    .POST(
                                      "/api/v1/staff/liquidity/lifts/{id}/decision",
                                      { ...id(l.id), body: { approve: true } },
                                    )
                                    .then(value),
                                "Restriction lifted.",
                              )
                            }
                          >
                            Approve lift {short(l.id)}
                          </button>{" "}
                          <button
                            className="secondary"
                            onClick={() =>
                              void act(
                                () =>
                                  client
                                    .POST(
                                      "/api/v1/staff/liquidity/lifts/{id}/decision",
                                      { ...id(l.id), body: { approve: false } },
                                    )
                                    .then(value),
                                "Lift rejected.",
                              )
                            }
                          >
                            Reject lift {short(l.id)}
                          </button>
                        </>
                      )}
                    </span>
                  ))}
                  {r.active && !r.lifts.some((l) => !l.outcome) && (
                    <>
                      {" "}
                      <button
                        className="secondary"
                        onClick={() =>
                          void act(
                            () =>
                              client
                                .POST(
                                  "/api/v1/staff/liquidity/restrictions/{id}/lifts",
                                  id(r.id),
                                )
                                .then(value),
                            "Lift proposed. A different approver must decide on a current within-limits forecast.",
                          )
                        }
                      >
                        Propose lift
                      </button>
                    </>
                  )}
                </li>
              ))}
            </ul>
          )}
          <h2>Automated actions</h2>
          {view.actions.length === 0 ? (
            <p className="muted">No automated actions proposed.</p>
          ) : (
            <ul>
              {view.actions.map((a) => (
                <li key={a.id} data-testid={`action-${a.state.toLowerCase()}`}>
                  Settlement prefunding {tst(a.amountMinor)} · {words(a.state)}
                  {a.confirmedBy ? ` · confirmed by ${a.confirmedBy}` : ""}
                  {a.decidedBy ? ` · decided by ${a.decidedBy}` : ""}
                  {a.batchId ? ` · Core batch ${short(a.batchId)}` : ""}{" "}
                  {a.state === "PROPOSED" && (
                    <button
                      onClick={() =>
                        void act(
                          () =>
                            client
                              .POST(
                                "/api/v1/staff/liquidity/actions/{id}/confirmation",
                                id(a.id),
                              )
                              .then(value),
                          "Action confirmed. A different approver must decide.",
                        )
                      }
                    >
                      Confirm action {short(a.id)}
                    </button>
                  )}
                  {a.state === "CONFIRMED" && (
                    <>
                      <button
                        onClick={() =>
                          void act(
                            () =>
                              client
                                .POST(
                                  "/api/v1/staff/liquidity/actions/{id}/decision",
                                  { ...id(a.id), body: { approve: true } },
                                )
                                .then(value),
                            "Action approved and booked by Core.",
                          )
                        }
                      >
                        Approve action {short(a.id)}
                      </button>{" "}
                      <button
                        className="secondary"
                        onClick={() =>
                          void act(
                            () =>
                              client
                                .POST(
                                  "/api/v1/staff/liquidity/actions/{id}/decision",
                                  { ...id(a.id), body: { approve: false } },
                                )
                                .then(value),
                            "Action rejected.",
                          )
                        }
                      >
                        Reject action {short(a.id)}
                      </button>
                    </>
                  )}
                  {a.state === "APPROVED" && (
                    <button
                      onClick={() =>
                        void act(
                          () =>
                            client
                              .POST(
                                "/api/v1/staff/liquidity/actions/{id}/execution",
                                id(a.id),
                              )
                              .then(value),
                          "Booking completed.",
                        )
                      }
                    >
                      Retry booking {short(a.id)}
                    </button>
                  )}
                </li>
              ))}
            </ul>
          )}
          <h2>Policy versions</h2>
          <ul className="card-list">
            {view.policies.map((p) => (
              <li className="card" key={p.version}>
                <strong>v{p.version}</strong> · buffer{" "}
                {tst(p.document.limits.minBufferMinor)} · per action up to{" "}
                {tst(p.document.limits.maxActionMinor)} · daily up to{" "}
                {tst(p.document.limits.maxDailyActionMinor)} ·{" "}
                {p.document.automation.scheduleMinutes
                  ? `scheduled every ${p.document.automation.scheduleMinutes} min`
                  : "on request only"}{" "}
                {p.version === view.activeVersion && (
                  <Badge tone="good">active</Badge>
                )}
                {p.proposedBy && (
                  <span>
                    {" "}
                    · proposed by {p.proposedBy}
                    {p.outcome ? (
                      `, ${words(p.outcome)} by ${p.decidedBy ?? ""}`
                    ) : (
                      <>
                        {" "}
                        <button
                          onClick={() =>
                            void act(
                              () =>
                                client
                                  .POST(
                                    "/api/v1/staff/liquidity/policies/{version}/decision",
                                    {
                                      params: { path: { version: p.version } },
                                      body: { approve: true },
                                    },
                                  )
                                  .then(value),
                              `Policy v${p.version} applied.`,
                            )
                          }
                        >
                          Approve policy v{p.version}
                        </button>{" "}
                        <button
                          className="secondary"
                          onClick={() =>
                            void act(
                              () =>
                                client
                                  .POST(
                                    "/api/v1/staff/liquidity/policies/{version}/decision",
                                    {
                                      params: { path: { version: p.version } },
                                      body: { approve: false },
                                    },
                                  )
                                  .then(value),
                              `Policy v${p.version} rejected.`,
                            )
                          }
                        >
                          Reject policy v{p.version}
                        </button>
                      </>
                    )}
                  </span>
                )}
              </li>
            ))}
          </ul>
          {active && (
            <form
              onSubmit={(e) => {
                e.preventDefault();
                const minor = Math.round(Number(buffer) * 100);
                if (!Number.isFinite(minor) || minor < 0) {
                  setError("Enter a buffer in TST.");
                  return;
                }
                const document = structuredClone(active.document);
                document.limits.minBufferMinor = String(minor);
                if (Number(document.automation.targetBufferMinor) < minor)
                  document.automation.targetBufferMinor = String(minor);
                void act(
                  () =>
                    client
                      .POST("/api/v1/staff/liquidity/policies", {
                        body: { document },
                      })
                      .then(value),
                  "Policy version proposed. A different approver must apply it.",
                );
              }}
            >
              <label>
                New minimum buffer (TST)
                <input
                  inputMode="decimal"
                  value={buffer}
                  onChange={(e) => setBuffer(e.target.value)}
                />
              </label>
              <button className="secondary">Propose policy version</button>
            </form>
          )}
          <h2>Recent forecasts</h2>
          <div className="table-scroll">
            <table>
              <thead>
                <tr>
                  <th>Recorded</th>
                  <th>Outcome</th>
                  <th>Status</th>
                  <th>Projected low</th>
                  <th>Reproduction</th>
                </tr>
              </thead>
              <tbody>
                {view.forecasts.map((f) => (
                  <tr key={f.id}>
                    <td>{time(f.createdAt)}</td>
                    <td>{words(f.outcome)}</td>
                    <td>{words(f.status)}</td>
                    <td>{tst(f.output.projectedLowMinor)}</td>
                    <td>
                      {f.reproductions.length > 0 &&
                        (f.reproductions[f.reproductions.length - 1].matched
                          ? "matched · "
                          : "mismatch · ")}
                      <button
                        className="secondary"
                        onClick={() =>
                          void act(
                            () =>
                              client
                                .POST(
                                  "/api/v1/staff/liquidity/forecasts/{id}/reproduction",
                                  id(f.id),
                                )
                                .then(value),
                            "Reproduction recorded.",
                          )
                        }
                      >
                        Reproduce {short(f.id)}
                      </button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          <h2>Forecasts against actual outflow</h2>
          {view.outcomes.length === 0 ? (
            <p className="muted">
              No completed business days since the first forecast yet.
            </p>
          ) : (
            <ul>
              {view.outcomes.map((o) => (
                <li key={o.forecastId}>
                  {o.businessDate}: expected {tst(o.expectedOutflowMinor)},
                  actual {tst(o.actualOutflowMinor)} ·{" "}
                  {o.withinBand ? "within band" : "outside band"}
                </li>
              ))}
            </ul>
          )}
        </>
      )}
    </section>
  );
}
