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
import type { Access } from "./App";
const amount = (minor: string) =>
  money({ minorUnits: minor, currency: { code: "TST", scale: 2 } });
export function FinanceCreditReconciliation({
  access,
  reportId,
  selected,
}: {
  access: Access;
  reportId: string;
  selected?: string;
}) {
  const { client } = useSession();
  const [runs, setRuns] = useState<Schema["FinanceCreditReconciliation"][]>([]);
  const [detail, setDetail] =
    useState<Schema["FinanceCreditReconciliationDetail"]>();
  const [page, setPage] = useState<Schema["FinanceCreditReconciliationPage"]>();
  const [fileId, setFileId] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const canCreate = access.capabilities.includes("finance.recover");
  const load = useCallback(async () => {
    try {
      setRuns(
        await client
          .GET("/api/v1/staff/finance/credit-reconciliations", {
            params: { query: { reportId } },
          })
          .then(value),
      );
      if (selected) {
        const next = await client
          .GET("/api/v1/staff/finance/credit-reconciliations/{id}", {
            params: { path: { id: selected } },
          })
          .then(value);
        if (next.reconciliation.reportId !== reportId)
          throw new Error(
            "The credit reconciliation belongs to another snapshot.",
          );
        setDetail(next);
      } else setDetail(undefined);
      setError("");
    } catch (e) {
      setError(message(e));
    }
  }, [client, reportId, selected]);
  const loadPage = useCallback(
    async (after: number) => {
      if (!selected) return;
      try {
        setPage(
          await client
            .GET("/api/v1/staff/finance/credit-reconciliations/{id}/items", {
              params: { path: { id: selected }, query: { after, limit: 5 } },
            })
            .then(value),
        );
        setError("");
      } catch (e) {
        setError(message(e));
      }
    },
    [client, selected],
  );
  useEffect(() => {
    setDetail(undefined);
    setPage(undefined);
    void load();
    void loadPage(0);
  }, [load, loadPage]);
  async function create(event: React.FormEvent) {
    event.preventDefault();
    setBusy(true);
    setError("");
    const scope = `pb:finance-credit-reconciliation:${reportId}:${fileId}`;
    const key = sessionStorage.getItem(scope) ?? crypto.randomUUID();
    sessionStorage.setItem(scope, key);
    try {
      const saved = await client
        .POST("/api/v1/staff/finance/credit-reconciliations", {
          params: { header: { "Idempotency-Key": key } },
          body: { reportId, fileId },
        })
        .then(value);
      sessionStorage.removeItem(scope);
      location.hash = `/finance/reports/${reportId}/credit-reconciliations/${saved.reconciliation.id}`;
    } catch (e) {
      setError(message(e));
    } finally {
      setBusy(false);
    }
  }
  return (
    <section className="panel" aria-label="Incoming and return reconciliation">
      <div className="panel-title">
        <h2>Reconcile incoming and returned payments</h2>
        <button className="secondary" onClick={() => void load()}>
          Refresh credit reconciliations
        </button>
      </div>
      <p>
        Match the independent credit clearing file, authenticated movement, Core
        suspense and customer allocation, original returned payment and captured
        GL. Funds remaining in suspense keep the result partial. Each return
        checks the lifetime principal cap; the original payment and its bank fee
        remain separate.
      </p>
      {error && <Alert>{error}</Alert>}
      {canCreate && (
        <form onSubmit={(e) => void create(e)}>
          <label htmlFor="credit-reconciliation-file">
            Published credit settlement file ID
          </label>
          <input
            id="credit-reconciliation-file"
            value={fileId}
            onChange={(e) => setFileId(e.target.value.trim())}
            required
            pattern="[a-fA-F0-9-]{36}"
          />
          <button disabled={busy || !/^[a-f0-9-]{36}$/i.test(fileId)}>
            {busy
              ? "Capturing credit reconciliation…"
              : "Capture credit reconciliation"}
          </button>
        </form>
      )}
      {!runs.length ? (
        <p>No credit reconciliation has been captured for this snapshot.</p>
      ) : (
        <nav aria-label="Saved credit reconciliations">
          {runs.map((run) => (
            <a
              className="case-row"
              key={run.id}
              href={`#/finance/reports/${reportId}/credit-reconciliations/${run.id}`}
              aria-current={selected === run.id ? "page" : undefined}
            >
              <strong>{short(run.id)}</strong>{" "}
              <Badge
                tone={run.completeness === "COMPLETE" ? "good" : "attention"}
              >
                {run.completeness}
              </Badge>
              <p>
                {run.matchedCount} matched · {run.exceptionCount} exceptions ·{" "}
                {run.timingCount} timing · {run.pendingCount} pending
              </p>
              <p>
                Principal still in suspense:{" "}
                {amount(run.coreSuspensePrincipalTotal)}
              </p>
              <small>
                Credit file {short(run.fileId)} · {time(run.createdAt)}
              </small>
            </a>
          ))}
        </nav>
      )}
      {detail && (
        <>
          <h3>Saved credit reconciliation {short(detail.reconciliation.id)}</h3>
          <dl className="evidence-details">
            <dt>Credit artifact digest</dt>
            <dd>{detail.digest}</dd>
            <dt>Credit scope</dt>
            <dd>{detail.reconciliation.scope}</dd>
            <dt>Parent report as of</dt>
            <dd>{time(detail.reconciliation.reportAsOf)}</dd>
            <dt>Core credit as of</dt>
            <dd>{time(detail.reconciliation.coreAsOf)}</dd>
            <dt>Credit workflow as of</dt>
            <dd>{time(detail.reconciliation.paymentsAsOf)}</dd>
            <dt>Credit file cutoff</dt>
            <dd>{time(detail.reconciliation.fileGeneratedAt)}</dd>
            <dt>Core received principal</dt>
            <dd>{amount(detail.reconciliation.corePrincipalTotal)}</dd>
            <dt>Core allocated principal</dt>
            <dd>{amount(detail.reconciliation.coreAllocatedPrincipalTotal)}</dd>
            <dt>Core principal still in suspense</dt>
            <dd>{amount(detail.reconciliation.coreSuspensePrincipalTotal)}</dd>
            <dt>Matched credit principal</dt>
            <dd>{amount(detail.reconciliation.matchedPrincipalTotal)}</dd>
          </dl>
          <h3>Credit file controls by currency</h3>
          {!detail.reconciliation.providerTotals.length ? (
            <p>The credit file contains no rows.</p>
          ) : (
            detail.reconciliation.providerTotals.map((t) => (
              <p key={`${t.currencyCode}:${t.currencyScale}`}>
                {t.currencyCode} (scale {t.currencyScale}): principal{" "}
                {t.principal} minor units; provider fees {t.providerFee} minor
                units. Duplicates remain in these controls.
              </p>
            ))
          )}
          <h3>Owned credit differences</h3>
          {!detail.issues.length ? (
            <p>
              No owned credit differences were recorded. Timing work can remain
              inside its acknowledgment window.
            </p>
          ) : (
            detail.issues.map((i) => (
              <p key={i.id}>
                Row {i.ordinal}: {i.code.replaceAll("_", " ")} ·{" "}
                {i.caseId ? (
                  <a href={`#/cases/${i.caseId}`}>
                    Open credit reconciliation case {short(i.caseId)}
                  </a>
                ) : (
                  "Case assignment pending"
                )}
              </p>
            ))
          )}
          <h3>Frozen credit lineage</h3>
          {page?.items.map((item) => (
            <article className="lineage-item" key={item.movementId}>
              <h4>Movement {short(item.movementId)}</h4>
              <Badge
                tone={item.classification === "MATCHED" ? "good" : "attention"}
              >
                {item.classification}
              </Badge>
              <p>
                {item.kind ?? "Unknown movement kind"} · Workflow:{" "}
                {item.paymentState ?? "Missing"} · Principal:{" "}
                {item.principal ? money(item.principal) : "Unavailable"}
              </p>
              {item.originalPaymentId && (
                <p>
                  Original payment{" "}
                  <a href={`#/payments/${item.originalPaymentId}`}>
                    {short(item.originalPaymentId)}
                  </a>{" "}
                  · Total returned principal across all periods:{" "}
                  {item.returnedPrincipalTotal == null
                    ? "Unavailable"
                    : amount(item.returnedPrincipalTotal)}
                </p>
              )}
              <p>
                Customer destination:{" "}
                {item.targetAccountId ?? "Unmatched receiver"} · Reference:{" "}
                {item.receivingReference ??
                  (item.originalPaymentId
                    ? "Return to original payer"
                    : "Unavailable")}
              </p>
              <p>
                Signed evidence: {item.evidenceId ?? "Unavailable"} · Digest:{" "}
                {item.evidenceDigest ?? "Unavailable"}
              </p>
              <p>
                Receipt into suspense: {item.suspenseBatchId ?? "Missing"} ·
                Booked date: {item.suspenseBusinessDate ?? "Unavailable"}
              </p>
              <p>
                Customer allocation:{" "}
                {item.allocationBatchId ?? "Funds remain in suspense"} · Booked
                date: {item.allocationBusinessDate ?? "Unavailable"}
              </p>
              {item.originalBatchId && (
                <p>Original settled journal: {item.originalBatchId}</p>
              )}
              <p>
                Captured journal batches:{" "}
                {item.journalBatchIds.join(", ") || "None in this snapshot"}
              </p>
              {item.glExportIds.map((id) => (
                <p key={id}>
                  <a href={`#/finance/${id}`}>Credit GL export {short(id)}</a>
                </p>
              ))}
              {item.providerRows.map((r, n) => (
                <p key={`${r.lineId}:${n}`}>
                  Credit clearing line {short(r.lineId)}: {r.principalMinor}{" "}
                  {r.currencyCode} minor units (scale {r.currencyScale});
                  provider fee {r.providerFeeMinor}; {r.businessDate}. Evidence{" "}
                  {short(r.evidenceId)} version {r.providerVersion}.
                </p>
              ))}
              {!!item.issueCodes.length && (
                <p>
                  {item.issueCodes
                    .map((c) => c.replaceAll("_", " "))
                    .join(" · ")}
                </p>
              )}
            </article>
          ))}
          <div className="recovery-actions">
            <button
              className="secondary"
              disabled={!page?.items.length || page.items[0]?.ordinal === 1}
              onClick={() => void loadPage(0)}
            >
              First credit lineage page
            </button>
            <button
              className="secondary"
              disabled={page?.nextAfter == null}
              onClick={() => void loadPage(page?.nextAfter ?? 0)}
            >
              Next credit lineage page
            </button>
          </div>
        </>
      )}
    </section>
  );
}
