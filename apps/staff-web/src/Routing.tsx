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

/* WP-39 routing across two simulated providers: versioned policies (maker proposes, a different approver decides),
   shadow comparison before activation, measured provider health and recorded route decisions. */
type Health = Record<
  string,
  {
    decisions: number;
    settled: number;
    nonExecuted: number;
    rejected: number;
    unknown: number;
    avgAckMs: number | null;
  }
>;
type Policy = {
  version: number;
  document: {
    strategy: string;
    providers: string[];
    fallbackReasons: string[];
    maxAttempts: number;
    promotionMinDecisions: number;
  };
};
type View = {
  head: {
    activeVersion: number;
    shadowVersion: number | null;
    revision: number;
  };
  policies: Policy[];
  changes: {
    id: string;
    version: number;
    mode: string;
    proposedBy: string;
    outcome: string | null;
    decidedBy: string | null;
    createdAt: string;
  }[];
  health: Health;
  shadow: {
    version: number;
    decisions: number;
    agreementRate: number | null;
    baseline: string;
  } | null;
  decisions: {
    paymentId: string;
    attempt: number;
    provider: string;
    policyVersion: number;
    rule: string;
    shadowProvider: string | null;
    decidedAt: string;
  }[];
};
const words = (code: string) => code.toLowerCase().replaceAll("_", " ");
const healthWeighted = {
  strategy: "HEALTH_WEIGHTED",
  providers: ["SYNTHETIC_RAIL", "SYNTHETIC_RAIL_B"],
  fallbackReasons: ["PROVIDER_CAPACITY_NOT_EXECUTED"],
  maxAttempts: 2,
  health: {
    windowMinutes: 60,
    minSamples: 3,
    maxUnknownRate: 0.2,
    maxNonexecutionRate: 0.5,
  },
  feesMinor: { SYNTHETIC_RAIL: 0, SYNTHETIC_RAIL_B: 0 },
  promotionMinDecisions: 3,
};

export function RoutingView() {
  const { client } = useSession();
  const [view, setView] = useState<View | null>(null);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const load = () =>
    client
      .GET("/api/v1/staff/routing")
      .then(value)
      .then((v) => setView(v as unknown as View))
      .catch((e) => setError(message(e)));
  useEffect(() => {
    void load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [client]);
  async function propose(
    body: { document?: object; version?: number; mode: string },
    label: string,
  ) {
    setError("");
    setNotice("");
    try {
      await client
        .POST("/api/v1/staff/routing/changes", {
          body: body as { mode: "ACTIVE" },
        })
        .then(value);
      setNotice(`${label} proposed. A different approver must decide.`);
      void load();
    } catch (e) {
      setError(message(e));
    }
  }
  async function decide(id: string, approve: boolean) {
    setError("");
    setNotice("");
    try {
      await client
        .POST("/api/v1/staff/routing/changes/{id}/decision", {
          params: { path: { id } },
          body: { approve },
        })
        .then(value);
      setNotice(approve ? "Change applied." : "Change rejected.");
      void load();
    } catch (e) {
      setError(message(e));
    }
  }
  return (
    <section aria-labelledby="routing-heading">
      <h1 id="routing-heading">Payment routing</h1>
      <p className="muted">
        Two simulated providers for the same rail. A payment moves to another
        provider only after signed proof that the first did not execute it.
        Benefit against a real baseline needs production evidence
        (DEC-03/04/07/11).
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
            Active policy v{view.head.activeVersion}
            {view.head.shadowVersion
              ? ` · shadow v${view.head.shadowVersion}`
              : " · no shadow policy"}{" "}
            · revision {view.head.revision}
          </p>
          <h2>Measured provider health (last hour)</h2>
          <div className="table-scroll">
            <table>
              <thead>
                <tr>
                  <th>Provider</th>
                  <th>Decisions</th>
                  <th>Settled</th>
                  <th>Not executed</th>
                  <th>Unknown</th>
                  <th>Average acknowledgement</th>
                </tr>
              </thead>
              <tbody>
                {Object.entries(view.health).map(([p, h]) => (
                  <tr key={p}>
                    <td>{p}</td>
                    <td>{h.decisions}</td>
                    <td>{h.settled}</td>
                    <td>{h.nonExecuted}</td>
                    <td>{h.unknown}</td>
                    <td>{h.avgAckMs === null ? "—" : `${h.avgAckMs} ms`}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          {view.shadow && (
            <p>
              Shadow v{view.shadow.version}: {view.shadow.decisions} decisions
              compared
              {view.shadow.agreementRate !== null
                ? `, ${Math.round(view.shadow.agreementRate * 100)}% agree with the active choice`
                : ""}
              . Baseline: local simulator only.
            </p>
          )}
          <h2>Policy versions</h2>
          <ul className="card-list">
            {view.policies.map((p) => (
              <li className="card" key={p.version}>
                <strong>v{p.version}</strong> · {words(p.document.strategy)} ·{" "}
                {p.document.providers.join(" then ")} · fallback on{" "}
                {p.document.fallbackReasons.length
                  ? p.document.fallbackReasons.map(words).join(", ")
                  : "nothing"}{" "}
                · up to {p.document.maxAttempts} attempts{" "}
                {p.version === view.head.activeVersion && (
                  <Badge tone="good">active</Badge>
                )}
                {p.version === view.head.shadowVersion && (
                  <Badge tone="attention">shadow</Badge>
                )}
                <div className="actions">
                  {p.version !== view.head.activeVersion && (
                    <>
                      <button
                        className="secondary"
                        onClick={() =>
                          void propose(
                            { version: p.version, mode: "SHADOW" },
                            `Shadow evaluation of v${p.version}`,
                          )
                        }
                      >
                        Shadow v{p.version}
                      </button>
                      <button
                        className="secondary"
                        onClick={() =>
                          void propose(
                            { version: p.version, mode: "ACTIVE" },
                            `Activation of v${p.version}`,
                          )
                        }
                      >
                        Activate v{p.version}
                      </button>
                    </>
                  )}
                </div>
              </li>
            ))}
          </ul>
          <button
            onClick={() =>
              void propose(
                { document: healthWeighted, mode: "SHADOW" },
                "A health-weighted policy in shadow mode",
              )
            }
          >
            Propose health-weighted policy (shadow)
          </button>
          <h2>Changes</h2>
          {view.changes.length === 0 ? (
            <p className="muted">No changes yet.</p>
          ) : (
            <ul>
              {view.changes.map((c) => (
                <li key={c.id}>
                  v{c.version} → {words(c.mode)} · proposed by {c.proposedBy} ·{" "}
                  {time(c.createdAt)} ·{" "}
                  {c.outcome ? (
                    `${words(c.outcome)} by ${c.decidedBy ?? ""}`
                  ) : (
                    <>
                      <button onClick={() => void decide(c.id, true)}>
                        Approve change {short(c.id)}
                      </button>{" "}
                      <button
                        className="secondary"
                        onClick={() => void decide(c.id, false)}
                      >
                        Reject change {short(c.id)}
                      </button>
                    </>
                  )}
                </li>
              ))}
            </ul>
          )}
          <h2>Recent route decisions</h2>
          {view.decisions.length === 0 ? (
            <p className="muted">No payments routed yet.</p>
          ) : (
            <ul>
              {view.decisions.map((d) => (
                <li key={`${d.paymentId}-${d.attempt}`}>
                  {short(d.paymentId)} attempt {d.attempt} → {d.provider} (v
                  {d.policyVersion}, {words(d.rule)})
                  {d.shadowProvider
                    ? ` · shadow chose ${d.shadowProvider}`
                    : ""}{" "}
                  · {time(d.decidedAt)}
                </li>
              ))}
            </ul>
          )}
        </>
      )}
    </section>
  );
}
