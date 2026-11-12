import { useCallback, useEffect, useState } from "react";
import {
  Alert,
  Badge,
  message,
  short,
  time,
  useSession,
  value,
  type Schema,
} from "@piggybank/web-core";
import type { Access } from "./App";
function tomorrowLocal() {
  const date = new Date(Date.now() + 86400000);
  return new Date(date.getTime() - date.getTimezoneOffset() * 60000)
    .toISOString()
    .slice(0, 16);
}
export function RolesView({ access }: { access: Access }) {
  const { client } = useSession();
  const [register, setRegister] = useState<Schema["RoleRegister"]>();
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [pending, setPending] = useState(false);
  const maker = access.capabilities.includes("roles.manage");
  const checker = access.capabilities.includes("roles.approve");
  const load = useCallback(async () => {
    try {
      setRegister(await client.GET("/api/v1/staff/roles").then(value));
      setError("");
    } catch (e) {
      setError(message(e));
    }
  }, [client]);
  useEffect(() => {
    void load();
  }, [load]);
  async function act(work: () => Promise<unknown>, note: string) {
    setPending(true);
    try {
      await work();
      setNotice(note);
      await load();
    } catch (e) {
      setError(message(e));
    } finally {
      setPending(false);
    }
  }
  return (
    <>
      <div className="page-title">
        <div>
          <p className="eyebrow">EXPLICIT AUTHORITY</p>
          <h1>Access with a clear purpose.</h1>
          <p>
            Review scoped grants, record changes and retain independent
            approval.
          </p>
        </div>
        <button className="secondary" onClick={() => void load()}>
          Refresh access register
        </button>
      </div>
      {error && <Alert>{error}</Alert>}
      {notice && (
        <p role="status" className="notice">
          {notice}
        </p>
      )}
      {maker && (
        <section className="panel">
          <h2>Propose an entitlement</h2>
          <form
            className="configuration-form"
            onSubmit={(e) => {
              e.preventDefault();
              const f = new FormData(e.currentTarget);
              void act(
                () =>
                  client
                    .POST("/api/v1/staff/roles", {
                      body: {
                        targetPerson: String(f.get("target")),
                        role: String(f.get("role")),
                        scope: String(f.get("scope")) || null,
                        validUntil: new Date(
                          String(f.get("until")),
                        ).toISOString(),
                      },
                    })
                    .then(value),
                "Role proposal saved for an independent person.",
              );
            }}
          >
            <label>
              Canonical person ID
              <input name="target" required pattern="[a-fA-F0-9-]{36}" />
            </label>
            <label>
              Role
              <select name="role">
                {[
                  "account_support",
                  "payment_operator",
                  "payment_inquiry",
                  "payment_approver",
                  "case_reviewer",
                  "case_approver",
                  "security_admin",
                  "security_approver",
                  "operations_replay",
                  "product_admin",
                  "product_approver",
                  "finance_adjuster",
                  "finance_approver",
                  "data_steward",
                  "data_approver",
                ].map((r) => (
                  <option key={r}>{r}</option>
                ))}
              </select>
            </label>
            <label>
              Resource scope · optional
              <input name="scope" pattern="[a-fA-F0-9-]{36}" />
            </label>
            <label>
              Valid until
              <input
                name="until"
                type="datetime-local"
                required
                defaultValue={tomorrowLocal()}
              />
            </label>
            <div className="wide">
              <p>
                Use an existing canonical identity. Relationships and queue
                membership remain separate requirements.
              </p>
              <button disabled={pending}>Propose role grant</button>
            </div>
          </form>
        </section>
      )}
      <section className="panel">
        <h2>Pending and historical proposals</h2>
        {register?.proposals.length === 0 && (
          <p>No entitlement proposals have been recorded.</p>
        )}
        {register?.proposals.map((p) => (
          <article className="register-row" key={p.id}>
            <div>
              <strong>{p.role}</strong>
              <p>
                Target {short(p.targetPerson)} · Scope{" "}
                {p.scope ? short(p.scope) : "Global"} · Maker{" "}
                {short(p.makerPerson)}
              </p>
              <small>
                {p.state} · Until {time(p.validUntil)}
              </small>
            </div>
            {checker &&
              p.state === "PENDING" &&
              p.makerPerson !== access.personId && (
                <button
                  disabled={pending}
                  onClick={() =>
                    void act(
                      () =>
                        client
                          .POST("/api/v1/staff/roles/{id}/approval", {
                            params: { path: { id: p.id } },
                          })
                          .then(value),
                      "Independent role approval recorded.",
                    )
                  }
                >
                  Approve role grant
                </button>
              )}
          </article>
        ))}
      </section>
      <section className="panel">
        <h2>Grant register</h2>
        <p>
          Showing the most recent 100 grants. Current validity and revocation
          apply on every server check.
        </p>
        {register?.grants.map((g) => (
          <article className="register-row" key={g.id}>
            <div>
              <strong>{g.role}</strong>
              <p>
                Person {short(g.personId)} ·{" "}
                {g.scope ? "Scope " + short(g.scope) : "Global scope"}
              </p>
              <small>Until {time(g.validUntil)}</small>
            </div>
            {g.revokedAt ? (
              <Badge>Revoked</Badge>
            ) : (
              <>
                <Badge tone="good">
                  {new Date(g.validUntil) > new Date() ? "Current" : "Expired"}
                </Badge>
                {maker && (
                  <button
                    className="secondary"
                    disabled={pending}
                    onClick={() =>
                      void act(
                        () =>
                          client
                            .POST("/api/v1/staff/roles/{id}/revocation", {
                              params: { path: { id: g.id } },
                            })
                            .then(value),
                        "Grant revoked. Current server checks apply immediately.",
                      )
                    }
                  >
                    Revoke grant
                  </button>
                )}
              </>
            )}
          </article>
        ))}
      </section>
    </>
  );
}
