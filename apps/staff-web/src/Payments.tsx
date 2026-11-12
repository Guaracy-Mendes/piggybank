import { useCallback, useEffect, useState } from "react";
import {
  Alert,
  Badge,
  Busy,
  message,
  money,
  short,
  time,
  useSession,
  value,
  type Schema,
} from "@piggybank/web-core";
import { CreditInquiry, QuarantineInquiry } from "./Credits";
import { RecoveryPanel, ReconciliationPanel } from "./Recovery";
import type { Access } from "./App";
export function PaymentsView({
  selected,
  access,
}: {
  selected?: string;
  access: Access;
}) {
  const { client } = useSession();
  const [items, setItems] = useState<Schema["PaymentStatus"][]>([]);
  const [detail, setDetail] = useState<Schema["PaymentStatus"]>();
  const [error, setError] = useState("");
  const load = useCallback(async () => {
    try {
      setItems(await client.GET("/api/v1/staff/payments").then(value));
      if (selected)
        setDetail(
          await client
            .GET("/api/v1/staff/payments/{id}", {
              params: { path: { id: selected } },
            })
            .then(value),
        );
      else setDetail(undefined);
      setError("");
    } catch (e) {
      setError(message(e));
    }
  }, [client, selected]);
  useEffect(() => {
    setDetail(undefined);
    void load();
  }, [load]);
  return (
    <>
      <div className="page-title">
        <div>
          <p className="eyebrow">INSTRUCTION & BOOKING</p>
          <h1>Follow the original payment.</h1>
          <p>
            Current payment authority and the payments queue are required for
            every inquiry.
          </p>
        </div>
        <button className="secondary" onClick={() => void load()}>
          Refresh payment inquiry
        </button>
      </div>
      {error && <Alert>{error}</Alert>}
      <div className="queue-layout">
        <section className="panel">
          <h2>Recent payment instructions</h2>
          <p>
            Most recent 100 records. Only Core booking evidence establishes
            settlement.
          </p>
          {items.length ? (
            items.map((p) => (
              <a key={p.id} className="case-row" href={"#/payments/" + p.id}>
                <div>
                  <strong>{short(p.id)}</strong>
                  <Badge tone={p.state === "SETTLED" ? "good" : "attention"}>
                    {p.state.replaceAll("_", " ")}
                  </Badge>
                </div>
                <p>
                  {p.beneficiaryLabel} · {money(p.total)}
                </p>
                <small>{time(p.updatedAt)}</small>
              </a>
            ))
          ) : (
            <p>No payment instructions have been saved.</p>
          )}
        </section>
        <section className="panel">
          {selected && !detail ? (
            <Busy />
          ) : !detail ? (
            <div className="empty">
              <h2>Evidence before disposition.</h2>
              <p>
                Select a payment to inspect its original details and booking
                reference.
              </p>
            </div>
          ) : (
            <>
              <div className="panel-title">
                <h2>Payment {short(detail.id)}</h2>
                <Badge tone={detail.state === "SETTLED" ? "good" : "attention"}>
                  {detail.state.replaceAll("_", " ")}
                </Badge>
              </div>
              <dl className="facts">
                <div>
                  <dt>Recipient label</dt>
                  <dd>{detail.beneficiaryLabel}</dd>
                </div>
                <div>
                  <dt>Source account</dt>
                  <dd>{detail.details.payerAccountId}</dd>
                </div>
                <div>
                  <dt>Principal</dt>
                  <dd>{money(detail.details.amount)}</dd>
                </div>
                <div>
                  <dt>Fee</dt>
                  <dd>{money(detail.details.fee)}</dd>
                </div>
                <div>
                  <dt>Total debit</dt>
                  <dd>{money(detail.total)}</dd>
                </div>
                <div>
                  <dt>Execution date / rail</dt>
                  <dd>
                    {detail.details.requestedDate} · {detail.details.rail}
                  </dd>
                </div>
                <div>
                  <dt>Current reason</dt>
                  <dd>{detail.reasonCode.replaceAll("_", " ")}</dd>
                </div>
                <div>
                  <dt>Workflow operation</dt>
                  <dd>{detail.operationId ?? "Not confirmed"}</dd>
                </div>
                <div>
                  <dt>Core journal receipt</dt>
                  <dd>{detail.batchId ?? "No booked payment evidence"}</dd>
                </div>
                <div>
                  <dt>Return status</dt>
                  <dd>
                    {detail.returns.status.replaceAll("_", " ")} · Provider
                    received {money(detail.returns.received)} · Customer
                    credited {money(detail.returns.credited)}
                  </dd>
                </div>
                <div>
                  <dt>Immutable quote</dt>
                  <dd>{detail.quoteId}</dd>
                </div>
              </dl>
              <p>
                Updated {time(detail.updatedAt)}. Inquiry does not provide
                journal edit or finality override authority.
              </p>
            </>
          )}
        </section>
      </div>
      {selected && (
        <RecoveryPanel
          key={selected}
          paymentId={selected}
          canOperate={access.capabilities.includes("rail.manage")}
          onChanged={load}
        />
      )}
      <ReconciliationPanel />
      <CreditInquiry originalId={selected} />
      <QuarantineInquiry />
    </>
  );
}
