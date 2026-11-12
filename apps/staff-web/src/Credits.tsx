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
export function CreditInquiry({ originalId }: { originalId?: string }) {
  const { client } = useSession();
  const [items, setItems] = useState<Schema["CreditStatus"][]>([]);
  const [error, setError] = useState("");
  const load = useCallback(async () => {
    try {
      setItems(await client.GET("/api/v1/staff/payment-credits").then(value));
      setError("");
    } catch (e) {
      setError(message(e));
    }
  }, [client]);
  useEffect(() => {
    void load();
  }, [load]);
  const visible = originalId
    ? items.filter((c) => c.details.originalPaymentId === originalId)
    : items;
  return (
    <section className="panel">
      <div className="panel-title">
        <h2>
          {originalId
            ? "Linked returns and credit evidence"
            : "Incoming & returned funds"}
        </h2>
        <button className="secondary" onClick={() => void load()}>
          Refresh credit inquiry
        </button>
      </div>
      {error && <Alert>{error}</Alert>}
      <p>
        Signed receipt, suspense accounting and customer allocation are separate
        records. Restricted destinations remain owned by operations.
      </p>
      {!visible.length ? (
        <p>No matching credit movements.</p>
      ) : (
        visible.map((c) => (
          <article className="case-row" key={c.id}>
            <div>
              <strong>
                {c.details.eventType === "RETURN_SETTLED"
                  ? "Return"
                  : "Inbound"}{" "}
                · {short(c.id)} · {money(c.details.amount)}
              </strong>
              <Badge tone={c.state === "CREDITED" ? "good" : "attention"}>
                {c.state.replaceAll("_", " ")}
              </Badge>
            </div>
            <dl className="facts">
              <div>
                <dt>Provider evidence</dt>
                <dd>{c.details.evidenceId}</dd>
              </div>
              <div>
                <dt>Destination</dt>
                <dd>{c.targetAccountId ?? "Unmatched receiving reference"}</dd>
              </div>
              <div>
                <dt>Original outgoing payment</dt>
                <dd>
                  {c.details.originalPaymentId ? (
                    <a href={"#/payments/" + c.details.originalPaymentId}>
                      {c.details.originalPaymentId}
                    </a>
                  ) : (
                    "Incoming transfer"
                  )}
                </dd>
              </div>
              <div>
                <dt>Suspense receipt</dt>
                <dd>{c.suspenseBatchId ?? "Accounting pending"}</dd>
              </div>
              <div>
                <dt>Customer credit receipt</dt>
                <dd>{c.allocationBatchId ?? "No customer credit"}</dd>
              </div>
              <div>
                <dt>Current reason</dt>
                <dd>{c.reasonCode.replaceAll("_", " ")}</dd>
              </div>
            </dl>
            <small>
              Business date {c.details.businessDate} · Updated{" "}
              {time(c.updatedAt)}
            </small>
          </article>
        ))
      )}
    </section>
  );
}
export function QuarantineInquiry() {
  const { client } = useSession();
  const [items, setItems] = useState<Schema["EvidenceQuarantine"][]>([]);
  const [error, setError] = useState("");
  const load = useCallback(async () => {
    try {
      setItems(
        (
          await client
            .GET("/api/v1/staff/payment-evidence-quarantine")
            .then(value)
        ).items,
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
        <h2>Quarantined provider evidence</h2>
        <button className="secondary" onClick={() => void load()}>
          Refresh evidence quarantine
        </button>
      </div>
      {error && <Alert>{error}</Alert>}
      <p>
        Each investigation reference has an owned payment case. Conflicting
        evidence cannot replace an accepted financial result.
      </p>
      {!items.length ? (
        <p>No quarantined provider evidence.</p>
      ) : (
        items.map((q) => (
          <article className="case-row" key={q.id}>
            <strong>Investigation {short(q.id)}</strong>
            <p>{q.reasonCode.replaceAll("_", " ")}</p>
            <small>
              Related payment {q.paymentId ?? "Unmatched"} ·{" "}
              {time(q.recordedAt)} · Digest {q.digest.slice(0, 16)}
            </small>
          </article>
        ))
      )}
    </section>
  );
}
