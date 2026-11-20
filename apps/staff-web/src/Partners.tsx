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

/* WP-33 partner registry (maker/checker), customer consents and authenticated callback deliveries.
   PostgreSQL rechecks roles and independence; a proposer can never approve its own registration. */
type Client = {
  id: string;
  clientId: string;
  displayName: string;
  allowedScopes: string[];
  quotaPerMinute: number;
  callbackUrl: string | null;
  state: string;
  proposedBy: string;
  decidedBy: string | null;
  decisionReason: string | null;
  activeConsents: number;
  createdAt: string;
};
type Consent = {
  id: string;
  clientId: string;
  partnerName: string;
  scopes: string[];
  state: string;
  accountIds: string[];
  personId: string | null;
  expiresAt: string | null;
  revokedBy: string | null;
  createdAt: string;
};
type Callback = {
  id: string;
  clientId: string;
  eventType: string;
  state: string;
  attempts: number;
  lastStatus: number | null;
  createdAt: string;
};
type View = { clients: Client[]; consents: Consent[]; callbacks: Callback[] };
const tone = (state: string) =>
  state === "ACTIVE" || state === "DELIVERED"
    ? "good"
    : state === "PENDING_APPROVAL" ||
        state === "PENDING" ||
        state === "AWAITING_AUTHORIZATION"
      ? "attention"
      : state === "SUSPENDED" || state === "FAILED" || state === "REJECTED"
        ? "danger"
        : "neutral";

export function PartnersView() {
  const { client } = useSession();
  const [view, setView] = useState<View | null>(null);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [secret, setSecret] = useState<{
    clientId: string;
    value: string;
  } | null>(null);
  const load = useCallback(async () => {
    try {
      setView(
        (await client
          .GET("/api/v1/staff/partners")
          .then(value)) as unknown as View,
      );
    } catch (e) {
      setError(message(e));
    }
  }, [client]);
  useEffect(() => {
    void load();
  }, [load]);
  const done = (text: string) => {
    setNotice(text);
    setError("");
    void load();
  };
  return (
    <section aria-labelledby="partners-heading">
      <h1 id="partners-heading">Partners & consent</h1>
      <p className="muted">
        Registered partner applications act only for customers who consented,
        within the scopes and accounts they chose. Partners can draft payments
        but never approve them.
      </p>
      {error && <Alert>{error}</Alert>}
      {notice && (
        <p className="notice" role="status">
          {notice}
        </p>
      )}
      {secret && (
        <div className="card" role="status">
          <strong>Callback signing secret for {secret.clientId}</strong>
          <p>
            Shown once. Hand it to the partner out of band; it is stored sealed
            and cannot be displayed again.
          </p>
          <code data-testid="callback-secret">{secret.value}</code>
          <button className="secondary" onClick={() => setSecret(null)}>
            I have recorded it
          </button>
        </div>
      )}
      <Propose
        proposed={(clientId, value) => {
          if (value) setSecret({ clientId, value });
          done(`${clientId} proposed. A second person must approve it.`);
        }}
        failed={setError}
      />
      <h2>Registered clients</h2>
      {!view ? null : view.clients.length === 0 ? (
        <p className="muted">No partner clients yet.</p>
      ) : (
        <table>
          <thead>
            <tr>
              <th>Client</th>
              <th>Scopes</th>
              <th>Quota</th>
              <th>State</th>
              <th>Consents</th>
              <th>Decision</th>
            </tr>
          </thead>
          <tbody>
            {view.clients.map((c) => (
              <tr key={c.id}>
                <td>
                  <strong>{c.displayName}</strong>
                  <br />
                  <code>{c.clientId}</code>
                </td>
                <td>{c.allowedScopes.join(", ")}</td>
                <td>{c.quotaPerMinute}/min</td>
                <td>
                  <Badge tone={tone(c.state)}>
                    {c.state.replaceAll("_", " ").toLowerCase()}
                  </Badge>
                </td>
                <td>{c.activeConsents}</td>
                <td>
                  <Decide client={c} done={done} failed={setError} />
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      <h2>Recent consents</h2>
      {!view ? null : view.consents.length === 0 ? (
        <p className="muted">No consents yet.</p>
      ) : (
        <table>
          <thead>
            <tr>
              <th>Partner</th>
              <th>Customer</th>
              <th>Scopes</th>
              <th>Accounts</th>
              <th>State</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {view.consents.map((k) => (
              <tr key={k.id}>
                <td>{k.partnerName}</td>
                <td>{k.personId ? short(k.personId) : "Not yet authorized"}</td>
                <td>{k.scopes.join(", ")}</td>
                <td>{k.accountIds.map(short).join(", ")}</td>
                <td>
                  <Badge tone={tone(k.state)}>
                    {k.state.replaceAll("_", " ").toLowerCase()}
                  </Badge>
                  {k.revokedBy ? ` by ${k.revokedBy.toLowerCase()}` : ""}
                </td>
                <td>
                  {k.state === "ACTIVE" && (
                    <button
                      className="secondary"
                      onClick={() =>
                        void client
                          .POST("/api/v1/staff/consents/{id}/revocation", {
                            params: { path: { id: k.id } },
                            body: {
                              reason: "Revoked by security administration",
                            },
                          })
                          .then(value)
                          .then(() =>
                            done(
                              "Consent revoked. The partner's next call fails.",
                            ),
                          )
                          .catch((e) => setError(message(e)))
                      }
                    >
                      Revoke
                    </button>
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      <h2>Callback deliveries</h2>
      {!view ? null : view.callbacks.length === 0 ? (
        <p className="muted">No callbacks yet.</p>
      ) : (
        <table>
          <thead>
            <tr>
              <th>Client</th>
              <th>Event</th>
              <th>State</th>
              <th>Attempts</th>
              <th>Last status</th>
              <th>Created</th>
            </tr>
          </thead>
          <tbody>
            {view.callbacks.map((b) => (
              <tr key={b.id}>
                <td>
                  <code>{b.clientId}</code>
                </td>
                <td>{b.eventType}</td>
                <td>
                  <Badge tone={tone(b.state)}>{b.state.toLowerCase()}</Badge>
                </td>
                <td>{b.attempts}</td>
                <td>{b.lastStatus ?? "—"}</td>
                <td>{time(b.createdAt)}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </section>
  );
}

function Propose({
  proposed,
  failed,
}: {
  proposed: (clientId: string, secret: string | null) => void;
  failed: (text: string) => void;
}) {
  const { client } = useSession();
  const [clientId, setClientId] = useState("partner-");
  const [name, setName] = useState("");
  const [read, setRead] = useState(true);
  const [pay, setPay] = useState(false);
  const [quota, setQuota] = useState(60);
  const [callback, setCallback] = useState("");
  const [busy, setBusy] = useState(false);
  async function submit() {
    setBusy(true);
    try {
      const result = (await client
        .POST("/api/v1/staff/partners", {
          params: { header: { "Idempotency-Key": crypto.randomUUID() } },
          body: {
            clientId,
            displayName: name,
            allowedScopes: [
              ...(read ? ["accounts:read"] : []),
              ...(pay ? ["payments:initiate"] : []),
            ] as ("accounts:read" | "payments:initiate")[],
            quotaPerMinute: quota,
            ...(callback ? { callbackUrl: callback } : {}),
          },
        })
        .then(value)) as unknown as { callbackSecret: string | null };
      proposed(clientId, result.callbackSecret);
      setClientId("partner-");
      setName("");
      setCallback("");
    } catch (e) {
      failed(message(e));
    } finally {
      setBusy(false);
    }
  }
  return (
    <form
      className="card"
      onSubmit={(e) => {
        e.preventDefault();
        void submit();
      }}
    >
      <h2>Propose a partner client</h2>
      <p className="muted">
        Security administrators propose; a different security administrator or
        approver decides. The client must already exist at the partner issuer.
      </p>
      <label>
        Client ID
        <input
          value={clientId}
          onChange={(e) => setClientId(e.target.value)}
          pattern="partner-[a-z0-9-]{2,40}"
          required
        />
      </label>
      <label>
        Display name
        <input
          value={name}
          onChange={(e) => setName(e.target.value)}
          minLength={2}
          maxLength={80}
          required
        />
      </label>
      <fieldset>
        <legend>Allowed scopes</legend>
        <label className="check">
          <input
            type="checkbox"
            checked={read}
            onChange={(e) => setRead(e.target.checked)}
          />{" "}
          accounts:read
        </label>
        <label className="check">
          <input
            type="checkbox"
            checked={pay}
            onChange={(e) => setPay(e.target.checked)}
          />{" "}
          payments:initiate
        </label>
      </fieldset>
      <label>
        Quota per minute
        <input
          type="number"
          min={1}
          max={100000}
          value={quota}
          onChange={(e) => setQuota(Number(e.target.value))}
          required
        />
      </label>
      <label>
        Callback URL (loopback only, optional)
        <input
          value={callback}
          onChange={(e) => setCallback(e.target.value)}
          placeholder="http://127.0.0.1:4390/hooks"
        />
      </label>
      <button disabled={busy || (!read && !pay)}>Propose client</button>
    </form>
  );
}

function Decide({
  client: partner,
  done,
  failed,
}: {
  client: Client;
  done: (text: string) => void;
  failed: (text: string) => void;
}) {
  const { client } = useSession();
  const [reason, setReason] = useState("");
  const options =
    partner.state === "PENDING_APPROVAL"
      ? (["APPROVE", "REJECT"] as const)
      : partner.state === "ACTIVE"
        ? (["SUSPEND"] as const)
        : partner.state === "SUSPENDED"
          ? (["REINSTATE"] as const)
          : ([] as const);
  if (!options.length)
    return <span className="muted">{partner.decisionReason ?? "—"}</span>;
  async function decide(
    decision: "APPROVE" | "REJECT" | "SUSPEND" | "REINSTATE",
  ) {
    try {
      await client
        .POST("/api/v1/staff/partners/{id}/decision", {
          params: { path: { id: partner.id } },
          body: { decision, reason },
        })
        .then(value);
      setReason("");
      done(`${partner.clientId}: ${decision.toLowerCase()} recorded.`);
    } catch (e) {
      failed(message(e));
    }
  }
  return (
    <div className="row">
      <label className="visually-hidden" htmlFor={"reason-" + partner.id}>
        Reason for {partner.clientId}
      </label>
      <input
        id={"reason-" + partner.id}
        value={reason}
        placeholder="Reason"
        onChange={(e) => setReason(e.target.value)}
      />
      {options.map((o) => (
        <button
          key={o}
          className={o === "APPROVE" ? undefined : "secondary"}
          disabled={reason.trim().length < 3}
          onClick={() => void decide(o)}
        >
          {o.charAt(0) + o.slice(1).toLowerCase()}
        </button>
      ))}
    </div>
  );
}
