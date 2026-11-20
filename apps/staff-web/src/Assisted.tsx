import { useCallback, useEffect, useState } from "react";
import {
  Alert,
  Badge,
  message,
  short,
  time,
  useSession,
  value,
} from "@piggybank/web-core";

/* WP-34 assisted channels: staff act for a customer only inside a session the customer authorized in a bank
   channel. Each channel enables only its confirmed operations (no payments until DEC-09). */
type Policy = {
  channel: "BRANCH" | "PRIVATE_BANKER" | "CALL_CENTER";
  verificationMethods: string[];
  operations: string[];
  sessionMinutes: number;
  note: string;
};
type Session = {
  id: string;
  channel: string;
  verification: string;
  purpose: string;
  state: string;
  customerPerson: string;
  expiresAt: string | null;
  createdAt: string;
  actions: {
    operation: string;
    outcome: string;
    reason: string;
    recordedAt: string;
  }[];
};
type Held = {
  account: { id: string; state: string; debitBlocked: boolean };
  balance: { available: { minorUnits: string } };
};
const labels: Record<string, string> = {
  BRANCH: "Branch",
  PRIVATE_BANKER: "Private banker",
  CALL_CENTER: "Call center",
  ID_DOCUMENT_SIGHTED: "Identity document sighted",
  KNOWN_RELATIONSHIP: "Known relationship",
  VERIFIED_CONTACT_CALLBACK: "Call back on verified contact",
};

export function AssistedView() {
  const { client } = useSession();
  const [policies, setPolicies] = useState<Policy[]>([]);
  const [sessions, setSessions] = useState<Session[]>([]);
  const [channel, setChannel] = useState<Policy["channel"]>("CALL_CENTER");
  const [verification, setVerification] = useState("");
  const [customer, setCustomer] = useState("");
  const [purpose, setPurpose] = useState("");
  const [accounts, setAccounts] = useState<Record<string, Held[]>>({});
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const load = useCallback(async () => {
    try {
      const v = (await client
        .GET("/api/v1/staff/assisted")
        .then(value)) as unknown as {
        policies: Policy[];
        sessions: Session[];
      };
      setPolicies(v.policies);
      setSessions(v.sessions);
    } catch (e) {
      setError(message(e));
    }
  }, [client]);
  useEffect(() => {
    void load();
  }, [load]);
  const policy = policies.find((p) => p.channel === channel);
  async function open() {
    setError("");
    try {
      await client
        .POST("/api/v1/staff/assisted", {
          params: { header: { "Idempotency-Key": crypto.randomUUID() } },
          body: {
            channel,
            customerPerson: customer.trim(),
            verification: verification as "ID_DOCUMENT_SIGHTED",
            purpose,
          },
        })
        .then(value);
      setNotice(
        "Session opened. Ask the customer to authorize it in their app.",
      );
      setPurpose("");
      void load();
    } catch (e) {
      setError(message(e));
    }
  }
  async function read(id: string) {
    setError("");
    try {
      const v = (await client
        .GET("/api/v1/staff/assisted/{id}/accounts", {
          params: { path: { id } },
        })
        .then(value)) as unknown as { accounts: Held[] };
      setAccounts({ ...accounts, [id]: v.accounts });
      void load();
    } catch (e) {
      setError(message(e));
      void load();
    }
  }
  async function freeze(id: string, accountId: string) {
    try {
      await client
        .POST("/api/v1/staff/assisted/{id}/freeze", {
          params: { path: { id } },
          body: { accountId },
        })
        .then(value);
      setNotice("Protective freeze applied. Outgoing payments are blocked.");
      void read(id);
    } catch (e) {
      setError(message(e));
    }
  }
  async function close(id: string) {
    try {
      await client
        .POST("/api/v1/staff/assisted/{id}/closure", {
          params: { path: { id } },
        })
        .then(value);
      setNotice("Session closed.");
      void load();
    } catch (e) {
      setError(message(e));
    }
  }
  return (
    <section aria-labelledby="assisted-heading">
      <h1 id="assisted-heading">Assisted service</h1>
      <p className="muted">
        Act for a customer only after verifying them and receiving their
        authorization in the app. Each channel allows only its confirmed
        operations; payments are not enabled in any channel.
      </p>
      {error && <Alert>{error}</Alert>}
      {notice && (
        <p className="notice" role="status">
          {notice}
        </p>
      )}
      <form
        className="card"
        onSubmit={(e) => {
          e.preventDefault();
          void open();
        }}
      >
        <h2>Start an assisted session</h2>
        <label>
          Channel
          <select
            value={channel}
            onChange={(e) => {
              setChannel(e.target.value as Policy["channel"]);
              setVerification("");
            }}
          >
            {policies.map((p) => (
              <option key={p.channel} value={p.channel}>
                {labels[p.channel]}
              </option>
            ))}
          </select>
        </label>
        <label>
          How you verified the customer
          <select
            value={verification}
            onChange={(e) => setVerification(e.target.value)}
            required
          >
            <option value="">Choose a method</option>
            {policy?.verificationMethods.map((m) => (
              <option key={m} value={m}>
                {labels[m] ?? m}
              </option>
            ))}
          </select>
        </label>
        <label>
          Customer person identifier
          <input
            value={customer}
            onChange={(e) => setCustomer(e.target.value)}
            pattern="[a-f0-9-]{36}"
            required
          />
        </label>
        <label>
          Purpose
          <input
            value={purpose}
            onChange={(e) => setPurpose(e.target.value)}
            minLength={3}
            maxLength={300}
            required
          />
        </label>
        {policy && (
          <p className="muted">
            Allowed:{" "}
            {policy.operations.join(", ").toLowerCase().replaceAll("_", " ")} ·{" "}
            {policy.sessionMinutes} minutes · {policy.note}
          </p>
        )}
        <button>Open session</button>
      </form>
      <h2>Your sessions</h2>
      {sessions.length === 0 ? (
        <p className="muted">No sessions yet.</p>
      ) : (
        <ul className="card-list">
          {sessions.map((s) => (
            <li className="card" key={s.id}>
              <div className="row">
                <strong>{labels[s.channel] ?? s.channel}</strong>
                <Badge
                  tone={
                    s.state === "AUTHORIZED"
                      ? "good"
                      : s.state === "AWAITING_CUSTOMER"
                        ? "attention"
                        : "neutral"
                  }
                >
                  {s.state.replaceAll("_", " ").toLowerCase()}
                </Badge>
                <span className="muted">
                  Customer {short(s.customerPerson)} · {s.purpose}
                  {s.expiresAt && s.state === "AUTHORIZED"
                    ? ` · until ${time(s.expiresAt)}`
                    : ""}
                </span>
              </div>
              {s.state === "AUTHORIZED" && (
                <div className="row">
                  <button className="secondary" onClick={() => void read(s.id)}>
                    View accounts
                  </button>
                  <button
                    className="secondary"
                    onClick={() => void close(s.id)}
                  >
                    Close session
                  </button>
                </div>
              )}
              {(accounts[s.id] ?? []).map((h) => (
                <div className="row" key={h.account.id}>
                  <span>
                    Account {short(h.account.id)} · available TST{" "}
                    {(Number(h.balance.available.minorUnits) / 100).toFixed(2)}
                    {h.account.debitBlocked
                      ? " · outgoing payments frozen"
                      : ""}
                  </span>
                  {!h.account.debitBlocked && s.state === "AUTHORIZED" && (
                    <button
                      className="secondary"
                      onClick={() => void freeze(s.id, h.account.id)}
                    >
                      Freeze account {short(h.account.id)}
                    </button>
                  )}
                </div>
              ))}
              {s.actions.length > 0 && (
                <details>
                  <summary>Decision record ({s.actions.length})</summary>
                  <ul>
                    {s.actions.map((a, i) => (
                      <li key={i}>
                        {time(a.recordedAt)} · {a.operation.toLowerCase()} ·{" "}
                        {a.outcome.toLowerCase()} ·{" "}
                        {a.reason.toLowerCase().replaceAll("_", " ")}
                      </li>
                    ))}
                  </ul>
                </details>
              )}
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
