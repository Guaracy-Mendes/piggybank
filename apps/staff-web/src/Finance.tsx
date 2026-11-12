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
import type { Access } from "./App";
import { FinanceReports } from "./FinanceReports";
const total = (minorUnits: string) =>
  money({ minorUnits, currency: { code: "TST", scale: 2 } });
export function FinanceView({
  selected,
  access,
  reportId,
  reconciliationId,
  creditReconciliationId,
  groupId,
}: {
  selected?: string;
  access: Access;
  reportId?: string;
  reconciliationId?: string;
  creditReconciliationId?: string;
  groupId?: string;
}) {
  const { client } = useSession();
  const [items, setItems] = useState<Schema["GlExport"][]>([]);
  const [breaks, setBreaks] = useState<Schema["FinanceBreak"][]>([]);
  const [detail, setDetail] = useState<Schema["GlExportDetail"]>();
  const [error, setError] = useState("");
  const load = useCallback(async () => {
    try {
      const [exports, observations] = await Promise.all([
        client.GET("/api/v1/staff/finance/exports").then(value),
        client.GET("/api/v1/staff/finance/breaks").then(value),
      ]);
      setItems(exports);
      setBreaks(observations);
      if (selected)
        setDetail(
          await client
            .GET("/api/v1/staff/finance/exports/{id}", {
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
    const timer = setInterval(() => void load(), 4000);
    return () => clearInterval(timer);
  }, [load]);
  return (
    <>
      <div className="page-title">
        <div>
          <p className="eyebrow">CORE SOURCE & GENERAL LEDGER</p>
          <h1>Account for every batch.</h1>
          <p>
            Exact TST control totals, approved mapping versions and
            authenticated booking receipts.
          </p>
        </div>
        <button className="secondary" onClick={() => void load()}>
          Refresh Finance
        </button>
      </div>
      {error && <Alert>{error}</Alert>}
      <FinanceReports
        access={access}
        selected={reportId}
        reconciliationId={reconciliationId}
        creditReconciliationId={creditReconciliationId}
        groupId={groupId}
      />
      <div className="queue-layout">
        <section className="panel">
          <h2>GL exports</h2>
          <p>
            Most recent 100 exports. Booking requires an authenticated GL
            receipt.
          </p>
          {!items.length ? (
            <p>No Core journal batches have been exported yet.</p>
          ) : (
            items.map((p) => (
              <a
                className="case-row"
                href={"#/finance/" + p.id}
                key={p.id}
                aria-current={selected === p.id ? "page" : undefined}
              >
                <div>
                  <strong>{short(p.sourceBatchId)}</strong>
                  <Badge tone={p.state === "BOOKED" ? "good" : "attention"}>
                    {p.state.replaceAll("_", " ")}
                  </Badge>
                </div>
                <p>
                  {p.batch.postingKind.replaceAll("_", " ")} ·{" "}
                  {total(p.batch.debitTotal)}
                </p>
                <small>
                  {p.batch.businessDate} · Chart version {p.batch.chartVersion}
                </small>
              </a>
            ))
          )}
        </section>
        <section className="panel">
          {selected && !detail ? (
            <Busy />
          ) : !detail ? (
            <div className="empty">
              <h2>Trace the booking.</h2>
              <p>
                Select an export to inspect its source, sealed mapping and
                acknowledgment.
              </p>
            </div>
          ) : (
            <>
              <div className="panel-title">
                <h2>GL export {short(detail.export.id)}</h2>
                <Badge
                  tone={detail.export.state === "BOOKED" ? "good" : "attention"}
                >
                  {detail.export.state.replaceAll("_", " ")}
                </Badge>
              </div>
              <dl className="evidence-details">
                <dt>Core source batch</dt>
                <dd>{detail.export.sourceBatchId}</dd>
                <dt>Core operation</dt>
                <dd>{detail.export.batch.coreOperationId}</dd>
                <dt>Linked original batch</dt>
                <dd>
                  {detail.export.batch.originalSourceBatchId ??
                    "Original posting"}
                </dd>
                <dt>Business date</dt>
                <dd>{detail.export.batch.businessDate} · UTC</dd>
                <dt>Approved chart</dt>
                <dd>
                  {detail.export.batch.chartId} · Version{" "}
                  {detail.export.batch.chartVersion}
                </dd>
                <dt>Debit control total</dt>
                <dd>{total(detail.export.batch.debitTotal)}</dd>
                <dt>Credit control total</dt>
                <dd>{total(detail.export.batch.creditTotal)}</dd>
                <dt>Provider journal reference</dt>
                <dd>
                  {detail.export.receipt?.providerJournalReference ??
                    "No authenticated booking receipt"}
                </dd>
                <dt>Booking evidence</dt>
                <dd>{detail.export.receipt?.evidenceId ?? "Pending"}</dd>
                <dt>Transport attempts</dt>
                <dd>
                  {detail.export.attempts} of {detail.export.attemptLimit} ·
                  Recovery generation {detail.export.retryGeneration}
                </dd>
                <dt>Updated</dt>
                <dd>{time(detail.export.updatedAt)}</dd>
              </dl>
              <h3>Mapped journal legs</h3>
              {detail.export.batch.entries.map((e) => (
                <article className="case-row" key={e.coreAccountId}>
                  <div>
                    <strong>{e.glAccount}</strong>
                    <span>{money(e.delta)}</span>
                  </div>
                  <small>Core account {e.coreAccountId}</small>
                </article>
              ))}
              <h3>Transport history</h3>
              {detail.history.map((h, i) => (
                <article className="case-row" key={i}>
                  <div>
                    <strong>{h.reasonCode.replaceAll("_", " ")}</strong>
                    <Badge>{h.state.replaceAll("_", " ")}</Badge>
                  </div>
                  <small>
                    Attempt {h.attempts} · {time(h.recordedAt)}
                  </small>
                </article>
              ))}
              {detail.export.state === "REVIEW_REQUIRED" &&
                access.capabilities.includes("finance.recover") && (
                  <Recovery
                    key={detail.export.id}
                    id={detail.export.id}
                    onChanged={load}
                  />
                )}
              {!!detail.recoveryRequests.length && (
                <>
                  <h3>Audited recovery requests</h3>
                  {detail.recoveryRequests.map((r) => (
                    <article className="case-row" key={r.id}>
                      <strong>
                        Generation {r.generation} · {short(r.id)}
                      </strong>
                      <p>{r.reason}</p>
                      <small>
                        Operation {r.operationId ?? "Scheduling pending"} ·{" "}
                        {time(r.createdAt)}
                      </small>
                    </article>
                  ))}
                </>
              )}
            </>
          )}
        </section>
      </div>
      <section className="panel">
        <h2>Finance exceptions</h2>
        <p>Matching evidence retains the case and audit history for review.</p>
        {!breaks.length ? (
          <p>No Finance breaks recorded.</p>
        ) : (
          breaks.map((b) => (
            <article className="case-row" key={b.id}>
              <div>
                <strong>{b.code.replaceAll("_", " ")}</strong>
                <Badge tone={b.state === "MATCHED" ? "good" : "attention"}>
                  {b.state}
                </Badge>
              </div>
              <p>Core source {b.sourceBatchId}</p>
              {b.caseId ? (
                <a href={"#/cases/" + b.caseId}>
                  Review Finance case {short(b.caseId)}
                </a>
              ) : (
                <span>Case assignment pending</span>
              )}
              <small> · Observed {time(b.lastSeenAt)}</small>
            </article>
          ))
        )}
      </section>
    </>
  );
}
function Recovery({
  id,
  onChanged,
}: {
  id: string;
  onChanged: () => Promise<void>;
}) {
  const { client } = useSession();
  const [reason, setReason] = useState("");
  const [pending, setPending] = useState(false);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  async function act() {
    setPending(true);
    setError("");
    try {
      const digest = await crypto.subtle.digest(
        "SHA-256",
        new TextEncoder().encode(JSON.stringify({ id, reason })),
      );
      const scope =
        "pb:gl-recovery:" +
        Array.from(new Uint8Array(digest), (b) =>
          b.toString(16).padStart(2, "0"),
        ).join("");
      const key = sessionStorage.getItem(scope) ?? crypto.randomUUID();
      sessionStorage.setItem(scope, key);
      await client
        .POST("/api/v1/staff/finance/exports/{id}/recovery-requests", {
          params: { path: { id }, header: { "Idempotency-Key": key } },
          body: { reason },
        })
        .then(value);
      sessionStorage.removeItem(scope);
      setNotice(
        "Original batch recovery accepted. Await the authenticated booking receipt.",
      );
      await onChanged();
    } catch (e) {
      setError(message(e));
    } finally {
      setPending(false);
    }
  }
  return (
    <section className="configuration-form">
      <h3>Recover original GL batch</h3>
      <p>
        Inquire first, then replay the same sealed batch only if the provider
        authenticates that it is missing. Each request permits eight transport
        attempts.
      </p>
      {error && <Alert>{error}</Alert>}
      {notice && <p role="status">{notice}</p>}
      <div className="recovery-field">
        <label htmlFor={"gl-recovery-" + id}>Finance recovery reason</label>
        <textarea
          id={"gl-recovery-" + id}
          value={reason}
          onChange={(e) => setReason(e.target.value)}
          minLength={4}
          maxLength={240}
        />
      </div>
      <button
        disabled={pending || reason.trim().length < 4}
        onClick={() => void act()}
      >
        Recover original GL batch
      </button>
    </section>
  );
}
