import { useCallback, useEffect, useState } from "react";
import {
  Alert,
  Badge,
  message,
  money,
  short,
  time,
  useSession,
  value,
  type Schema,
} from "@piggybank/web-core";
export function RecoveryPanel({
  paymentId,
  canOperate,
  onChanged,
}: {
  paymentId: string;
  canOperate: boolean;
  onChanged: () => Promise<void>;
}) {
  const { client } = useSession();
  const [items, setItems] = useState<Schema["RecoveryRequest"][]>([]);
  const [reason, setReason] = useState("");
  const [pending, setPending] = useState(false);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const load = useCallback(async () => {
    try {
      setItems(
        await client
          .GET("/api/v1/staff/payments/{id}/recovery-requests", {
            params: { path: { id: paymentId } },
          })
          .then(value),
      );
      setError("");
    } catch (e) {
      setError(message(e));
    }
  }, [client, paymentId]);
  useEffect(() => {
    void load();
    const timer = setInterval(() => void load(), 4000);
    return () => clearInterval(timer);
  }, [load]);
  async function act(action: Schema["RecoveryInput"]["action"]) {
    setPending(true);
    setNotice("");
    try {
      const digest = await crypto.subtle.digest(
        "SHA-256",
        new TextEncoder().encode(JSON.stringify({ paymentId, action, reason })),
      );
      const scope =
        "pb:recovery:" +
        Array.from(new Uint8Array(digest), (b) =>
          b.toString(16).padStart(2, "0"),
        ).join("");
      const key = sessionStorage.getItem(scope) ?? crypto.randomUUID();
      sessionStorage.setItem(scope, key);
      await client
        .POST("/api/v1/staff/payments/{id}/recovery-requests", {
          params: {
            path: { id: paymentId },
            header: { "Idempotency-Key": key },
          },
          body: { action, reason },
        })
        .then(value);
      sessionStorage.removeItem(scope);
      setNotice(
        "Recovery accepted using the original instruction. Financial outcome is still determined by provider evidence and Core receipts.",
      );
      await load();
      await onChanged();
    } catch (e) {
      setError(message(e));
    } finally {
      setPending(false);
    }
  }
  return (
    <section className="panel">
      <div className="panel-title">
        <h2>Controlled payment recovery</h2>
        <button className="secondary" onClick={() => void load()}>
          Refresh recovery history
        </button>
      </div>
      <p>
        Inquiry and cancellation retain the original instruction. An uncertain
        submitted hold stays reserved. Each command has a bounded retry budget;
        inquiry commands have a 30 second cooldown.
      </p>
      {error && <Alert>{error}</Alert>}
      {notice && (
        <p role="status" className="notice">
          {notice}
        </p>
      )}
      {canOperate && (
        <div className="configuration-form">
          <div className="recovery-field">
            <label htmlFor={"recovery-reason-" + paymentId}>
              Recovery reason
            </label>
            <textarea
              id={"recovery-reason-" + paymentId}
              value={reason}
              onChange={(e) => setReason(e.target.value)}
              minLength={4}
              maxLength={240}
              required
            />
          </div>
          <div className="recovery-actions">
            <button
              disabled={pending || reason.trim().length < 4}
              onClick={() => void act("INQUIRE")}
            >
              Inquire using original instruction
            </button>
            <button
              className="secondary"
              disabled={pending || reason.trim().length < 4}
              onClick={() => void act("CANCEL")}
            >
              Request provider cancellation
            </button>
          </div>
        </div>
      )}
      {!items.length ? (
        <p>No recovery commands recorded.</p>
      ) : (
        items.map((r) => (
          <article className="case-row" key={r.id}>
            <div>
              <strong>
                {r.action === "CANCEL"
                  ? "Cancellation request"
                  : "Original instruction inquiry"}{" "}
                · {short(r.id)}
              </strong>
              <Badge tone={r.state === "COMPLETED" ? "good" : "attention"}>
                {r.state.replaceAll("_", " ")}
              </Badge>
            </div>
            <p>{r.reasonCode.replaceAll("_", " ")}</p>
            <small>
              Requested {time(r.createdAt)} · Operation{" "}
              {r.operationId ?? "Scheduling pending"}
            </small>
          </article>
        ))
      )}
    </section>
  );
}
export function ReconciliationPanel() {
  const { client } = useSession();
  const [items, setItems] = useState<Schema["ReconciliationBreak"][]>([]);
  const [error, setError] = useState("");
  const load = useCallback(async () => {
    try {
      setItems(
        await client.GET("/api/v1/staff/payment-reconciliation").then(value),
      );
      setError("");
    } catch (e) {
      setError(message(e));
    }
  }, [client]);
  useEffect(() => {
    void load();
  }, [load]);
  return (
    <section className="panel">
      <div className="panel-title">
        <h2>Payment reconciliation</h2>
        <button className="secondary" onClick={() => void load()}>
          Refresh reconciliation
        </button>
      </div>
      <p>
        Submitted holds older than five minutes and inconsistent financial
        records create owned cases. A matched observation preserves its
        investigation and history; it does not close the case or authorize a
        journal change.
      </p>
      {error && <Alert>{error}</Alert>}
      {!items.length ? (
        <p>No observed reconciliation breaks.</p>
      ) : (
        items.map((b) => (
          <article className="case-row" key={b.id}>
            <div>
              <strong>{b.code.replaceAll("_", " ")}</strong>
              <Badge tone={b.state === "MATCHED" ? "good" : "attention"}>
                {b.state}
              </Badge>
            </div>
            <p>
              <a href={"#/payments/" + b.paymentId}>
                Payment {short(b.paymentId)}
              </a>
              {b.caseId && (
                <>
                  {" "}
                  ·{" "}
                  <a href={"#/cases/" + b.caseId}>
                    Owned case {short(b.caseId)}
                  </a>
                </>
              )}
            </p>
            <dl className="facts">
              <div>
                <dt>Core commitment / hold</dt>
                <dd>
                  {money(b.observation.core.total)} ·{" "}
                  {b.observation.core.holdState}
                </dd>
              </div>
              <div>
                <dt>Financial result / workflow</dt>
                <dd>
                  {b.observation.core.finalOutcome ?? "Outcome unknown"} ·{" "}
                  {b.observation.workflow?.state ?? "Missing workflow"}
                </dd>
              </div>
              <div>
                <dt>Core booking</dt>
                <dd>{b.observation.core.batchId ?? "No booking receipt"}</dd>
              </div>
              <div>
                <dt>Daily principal usage</dt>
                <dd>{b.observation.core.dailyUsageState}</dd>
              </div>
            </dl>
            <small>
              First observed {time(b.firstSeenAt)} · Last observed{" "}
              {time(b.lastSeenAt)} · Version {b.version}
            </small>
          </article>
        ))
      )}
    </section>
  );
}
