import { useCallback, useEffect, useRef, useState } from "react";
import {
  Alert,
  Badge,
  message,
  short,
  useSession,
  value,
  type Schema,
} from "@piggybank/web-core";
type Item = Schema["FinanceAuditDelivery"];
const reason: Record<Schema["FinanceAuditAttempt"]["outcome"], string> = {
  NATIVE_RECEIPT_CONFIRMED: "Original native receipt confirmed",
  INVALID_RECORD: "Original record needs investigation",
  NON_CANONICAL_RECORD: "Original wire representation needs investigation",
  AUDIT_IDENTITY_CONFLICT: "Original identity conflicts with native evidence",
  CONFIRMATION_UNAVAILABLE:
    "Native confirmation unavailable; original identity retained",
  APPEND_UNCONFIRMED: "Append outcome unconfirmed; original identity retained",
};
export function FinanceAudit({ selected }: { selected?: string }) {
  const { client } = useSession();
  const [status, setStatus] = useState<Schema["FinanceAuditStatus"]>(),
    [detail, setDetail] = useState<Item>();
  const [error, setError] = useState("");
  const generation = useRef(0),
    selection = useRef(selected);
  selection.current = selected;
  const load = useCallback(async () => {
    const current = ++generation.current;
    try {
      const [summary, item] = await Promise.all([
        client.GET("/api/v1/staff/finance/audit-delivery").then(value),
        selected
          ? client
              .GET("/api/v1/staff/finance/audit-delivery/{id}", {
                params: { path: { id: selected } },
              })
              .then(value)
          : Promise.resolve(undefined),
      ]);
      if (current !== generation.current || selection.current !== selected)
        return;
      setStatus(summary);
      setDetail(item);
      setError("");
    } catch (e) {
      if (current === generation.current && selection.current === selected) {
        setStatus(undefined);
        setDetail(undefined);
        setError(message(e));
      }
    }
  }, [client, selected]);
  useEffect(() => {
    setDetail(undefined);
    void load();
    const timer = setInterval(() => void load(), 4000);
    return () => {
      clearInterval(timer);
      generation.current++;
    };
  }, [load]);
  return (
    <>
      <div className="page-title">
        <div>
          <p className="eyebrow">FINANCE AUDIT</p>
          <h1>Track every audit handoff.</h1>
          <p>
            Original event identities, durable retries and native Controls
            receipts.
          </p>
        </div>
        <button className="secondary" onClick={() => void load()}>
          Refresh audit delivery
        </button>
      </div>
      <p className="notice">
        Audit delivery records a handoff to Controls. Financial completion still
        requires the original Core receipt. Failed audit records remain pending;
        they are never discarded or marked delivered by a status label.
      </p>
      {error && <Alert>{error}</Alert>}
      {status && (
        <section className="panel" aria-label="Audit delivery summary">
          <h2>Delivery observations</h2>
          <dl className="evidence-details">
            <dt>Pending events</dt>
            <dd data-testid="audit-pending-count">{status.pendingCount}</dd>
            <dt>Native confirmations</dt>
            <dd>{status.confirmedCount}</dd>
            <dt>Counts observed at (UTC)</dt>
            <dd>{status.asOf}</dd>
            <dt>Last relay scan (UTC)</dt>
            <dd>{status.lastScannedAt ?? "Not yet scanned"}</dd>
            <dt>Durable cursor</dt>
            <dd className="mono">{status.cursor}</dd>
          </dl>
          <p>
            Live counts and item reads can observe different commits. Each
            delivered item independently verifies its native receipt.
          </p>
        </section>
      )}
      <div className="queue-layout">
        <section className="panel">
          <h2>Delivery queue</h2>
          <p>
            Up to 100 original events, pending first in UUID order. Saved event
            URLs reopen records outside this window. Recovery visits at most 50
            events per pass and wraps to retry earlier identities.
          </p>
          {status && !status.items.length && (
            <p>No Finance audit events recorded.</p>
          )}
          {status?.items.map((item) => (
            <a
              key={item.id}
              className="case-row"
              href={"#/finance/audit/" + item.id}
              aria-current={item.id === selected ? "page" : undefined}
            >
              <div>
                <strong>{item.action ?? "Original record unavailable"}</strong>
                <Badge tone={item.state === "DELIVERED" ? "good" : "attention"}>
                  {item.state}
                </Badge>
              </div>
              <p>
                {item.receipt
                  ? "Original native receipt confirmed"
                  : item.lastAttempt
                    ? reason[item.lastAttempt.outcome]
                    : "Awaiting original native confirmation"}
              </p>
              <small>{short(item.id)}</small>
            </a>
          ))}
        </section>
        {detail && detail.id === selected && (
          <section className="panel" aria-label="Selected audit event">
            <h2>Selected audit event</h2>
            <Badge tone={detail.state === "DELIVERED" ? "good" : "attention"}>
              {detail.state}
            </Badge>
            <dl className="evidence-details">
              <dt>Original audit event</dt>
              <dd className="mono">{detail.id}</dd>
              <dt>Original resource</dt>
              <dd className="mono">{detail.resourceId ?? "Unavailable"}</dd>
              <dt>Action</dt>
              <dd>{detail.action ?? "Unavailable"}</dd>
              <dt>Original record digest</dt>
              <dd className="mono">{detail.sourceDigest}</dd>
              <dt>Delivery acknowledged at (UTC)</dt>
              <dd>{detail.deliveredAt ?? "Not acknowledged"}</dd>
              <dt>Latest attempt</dt>
              <dd>
                {detail.lastAttempt
                  ? reason[detail.lastAttempt.outcome]
                  : "No relay attempt retained"}
              </dd>
              <dt>Attempted at (UTC)</dt>
              <dd>
                {detail.lastAttempt?.attemptedAt ?? "No relay attempt retained"}
              </dd>
            </dl>
            {detail.receipt ? (
              <div>
                <h3>Native audit receipt</h3>
                <dl className="evidence-details">
                  <dt>Original native event</dt>
                  <dd className="mono">{detail.receipt.auditId}</dd>
                  <dt>Controls chain digest</dt>
                  <dd className="mono">{detail.receipt.controlsDigest}</dd>
                  <dt>Controls sequence</dt>
                  <dd>{detail.receipt.controlsSequence}</dd>
                  <dt>Receipt confirmed at (UTC)</dt>
                  <dd>{detail.receipt.confirmedAt}</dd>
                  <dt>Receipt origin</dt>
                  <dd>
                    {detail.receipt.origin === "LEGACY_VERIFIED"
                      ? "Legacy delivery verified against original native evidence"
                      : "Native confirmation"}
                  </dd>
                </dl>
              </div>
            ) : (
              <p className="notice">
                Original native receipt not confirmed. Keep the original
                identity and worker recovery running. Investigate the retained
                source rather than replacing or deleting audit evidence.
              </p>
            )}
          </section>
        )}
      </div>
    </>
  );
}
