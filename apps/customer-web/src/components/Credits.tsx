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
export function Credits({ originalId }: { originalId?: string }) {
  const { client } = useSession();
  const [items, setItems] = useState<Schema["CreditStatus"][]>([]);
  const [error, setError] = useState("");
  const load = useCallback(async () => {
    try {
      setItems(await client.GET("/api/v1/payment-credits").then(value));
      setError("");
    } catch (e) {
      setError(message(e));
    }
  }, [client]);
  useEffect(() => {
    void load();
    const timer = setInterval(() => void load(), 4000);
    return () => clearInterval(timer);
  }, [load]);
  const visible = originalId
    ? items.filter((c) => c.details.originalPaymentId === originalId)
    : items;
  return (
    <section className="card payment-card">
      <div className="spread">
        <h2>
          {originalId ? "Linked returned funds" : "Incoming & returned funds"}
        </h2>
        <button className="secondary" onClick={() => void load()}>
          Refresh incoming funds
        </button>
      </div>
      <p className="muted">
        Funds become available after destination and screening checks. A return
        preserves the original payment and its fee.
      </p>
      {error && <Alert>{error}</Alert>}
      {!visible.length ? (
        <p>No matching incoming or returned funds.</p>
      ) : (
        visible.map((c) => (
          <article className="recipient-row" key={c.id}>
            <div>
              <strong>
                {c.details.eventType === "RETURN_SETTLED"
                  ? "Returned payment"
                  : "Incoming transfer"}{" "}
                · {money(c.details.amount)}
              </strong>
              <p>
                Reference {short(c.id)} · {c.details.businessDate} · Updated{" "}
                {time(c.updatedAt)}
              </p>
              {c.details.originalPaymentId && (
                <a href={"#/payments/" + c.details.originalPaymentId}>
                  Original payment {short(c.details.originalPaymentId)}
                </a>
              )}
              <p>
                {c.state === "CREDITED"
                  ? "Credited to your account."
                  : "Funds are under review and are not available yet."}
              </p>
              {c.allocationBatchId && (
                <small>Booking reference {short(c.allocationBatchId)}</small>
              )}
            </div>
            <Badge tone={c.state === "CREDITED" ? "good" : "attention"}>
              {c.state === "CREDITED"
                ? "Available"
                : ["RECEIVED", "CORE_SUSPENSE", "CONTROLS_PENDING"].includes(
                      c.state,
                    )
                  ? "Checks in progress"
                  : "With our team"}
            </Badge>
          </article>
        ))
      )}
    </section>
  );
}
