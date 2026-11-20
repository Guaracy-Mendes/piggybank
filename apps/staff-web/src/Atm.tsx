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

/* WP-35 ATM channel: the confirmed-operations supplement, recent authorizations and settlement reconciliation. */
type View = {
  operations: {
    operation: string;
    enabled: boolean;
    declineReason: string | null;
  }[];
  limits: {
    perWithdrawalMinor: string;
    dailyMinor: string;
    authorizationSeconds: number;
  };
  authorizations: {
    id: string;
    terminalId: string;
    amountMinor: string;
    state: string;
    declineReason: string | null;
    lateReversal: boolean;
    createdAt: string;
  }[];
  settlements: {
    fileId: string;
    businessDate: string;
    matched: number;
    differences: number;
    items: { authorizationId: string; outcome: string }[];
  }[];
};
const tst = (minor: string) => `TST ${(Number(minor) / 100).toFixed(2)}`;
export function AtmView() {
  const { client } = useSession();
  const [view, setView] = useState<View | null>(null);
  const [error, setError] = useState("");
  useEffect(() => {
    client
      .GET("/api/v1/staff/atm")
      .then(value)
      .then((v) => setView(v as unknown as View))
      .catch((e) => setError(message(e)));
  }, [client]);
  return (
    <section aria-labelledby="atm-heading">
      <h1 id="atm-heading">ATM channel</h1>
      <p className="muted">
        Synthetic network simulator. Only confirmed operations run; actual
        network certification is required before enablement.
      </p>
      {error && <Alert>{error}</Alert>}
      {view && (
        <>
          <h2>Channel supplement</h2>
          <p>
            {tst(view.limits.perWithdrawalMinor)} per withdrawal ·{" "}
            {tst(view.limits.dailyMinor)} per card per day · authorizations
            expire after {view.limits.authorizationSeconds} seconds
          </p>
          <ul>
            {view.operations.map((o) => (
              <li key={o.operation}>
                {o.operation.toLowerCase().replaceAll("_", " ")}:{" "}
                <Badge tone={o.enabled ? "good" : "neutral"}>
                  {o.enabled ? "enabled" : "declined"}
                </Badge>
              </li>
            ))}
          </ul>
          <h2>Recent authorizations</h2>
          {view.authorizations.length === 0 ? (
            <p className="muted">No ATM activity yet.</p>
          ) : (
            <table>
              <thead>
                <tr>
                  <th>Authorization</th>
                  <th>Terminal</th>
                  <th>Amount</th>
                  <th>State</th>
                  <th>When</th>
                </tr>
              </thead>
              <tbody>
                {view.authorizations.map((a) => (
                  <tr key={a.id}>
                    <td>{short(a.id)}</td>
                    <td>{a.terminalId}</td>
                    <td>{tst(a.amountMinor)}</td>
                    <td>
                      {a.state.toLowerCase()}
                      {a.declineReason
                        ? ` · ${a.declineReason.toLowerCase().replaceAll("_", " ")}`
                        : ""}
                      {a.lateReversal ? " · late reversal under review" : ""}
                    </td>
                    <td>{time(a.createdAt)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
          <h2>Settlement reconciliation</h2>
          {view.settlements.length === 0 ? (
            <p className="muted">No settlement files yet.</p>
          ) : (
            <ul className="card-list">
              {view.settlements.map((f) => (
                <li className="card" key={f.fileId}>
                  <strong>{f.fileId}</strong> · {f.businessDate} · {f.matched}{" "}
                  matched ·{" "}
                  <Badge tone={f.differences ? "attention" : "good"}>
                    {f.differences} differences
                  </Badge>
                  <ul>
                    {f.items
                      .filter((i) => i.outcome !== "MATCHED")
                      .map((i) => (
                        <li key={i.authorizationId}>
                          {short(i.authorizationId)} ·{" "}
                          {i.outcome.toLowerCase().replaceAll("_", " ")}
                        </li>
                      ))}
                  </ul>
                </li>
              ))}
            </ul>
          )}
        </>
      )}
    </section>
  );
}
