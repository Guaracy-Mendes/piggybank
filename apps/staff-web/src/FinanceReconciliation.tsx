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
export function FinanceReconciliation({
  access,
  reportId,
  selected,
}: {
  access: Access;
  reportId: string;
  selected?: string;
}) {
  const { client } = useSession();
  const [runs, setRuns] = useState<Schema["FinanceReconciliation"][]>([]);
  const [detail, setDetail] = useState<Schema["FinanceReconciliationDetail"]>();
  const [page, setPage] = useState<Schema["FinanceReconciliationPage"]>();
  const [fileId, setFileId] = useState("");
  const [busy, setBusy] = useState(false),
    [error, setError] = useState("");
  const canCreate = access.capabilities.includes("finance.recover");
  const load = useCallback(async () => {
    try {
      setRuns(
        await client
          .GET("/api/v1/staff/finance/reconciliations", {
            params: { query: { reportId } },
          })
          .then(value),
      );
      if (selected) {
        const next = await client
          .GET("/api/v1/staff/finance/reconciliations/{id}", {
            params: { path: { id: selected } },
          })
          .then(value);
        if (next.reconciliation.reportId !== reportId)
          throw new Error("The reconciliation belongs to another snapshot.");
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
            .GET("/api/v1/staff/finance/reconciliations/{id}/items", {
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
    const scope = `pb:finance-reconciliation:${reportId}:${fileId}`;
    const key = sessionStorage.getItem(scope) ?? crypto.randomUUID();
    sessionStorage.setItem(scope, key);
    try {
      const result = await client
        .POST("/api/v1/staff/finance/reconciliations", {
          params: { header: { "Idempotency-Key": key } },
          body: { reportId, fileId },
        })
        .then(value);
      sessionStorage.removeItem(scope);
      location.hash = `/finance/reports/${reportId}/reconciliations/${result.reconciliation.id}`;
    } catch (e) {
      setError(message(e));
    } finally {
      setBusy(false);
    }
  }
  return (
    <section className="panel" aria-label="Payment reconciliation">
      <div className="panel-title">
        <h2>Reconcile outgoing payments</h2>
        <button className="secondary" onClick={() => void load()}>
          Refresh reconciliations
        </button>
      </div>
      <p>
        Match this frozen journal and GL snapshot to original payment
        instructions, Core accounting receipts and an independently
        authenticated clearing file for the same UTC period. Customer bank fees
        and provider fees appear separately. A new run preserves its
        observations and creates owned cases for differences.
      </p>
      <p>
        The synthetic provider uses the explicit zero clearing fee policy.
        Missing, duplicate, currency, value and date differences require review.
        New financial facts require a new snapshot; saved results retain their
        original meaning.
      </p>
      {error && <Alert>{error}</Alert>}
      {canCreate && (
        <form onSubmit={(e) => void create(e)}>
          <label htmlFor="reconciliation-file">
            Published settlement file ID
          </label>
          <input
            id="reconciliation-file"
            value={fileId}
            onChange={(e) => setFileId(e.target.value.trim())}
            required
            pattern="[a-fA-F0-9-]{36}"
            placeholder="UUID of the authenticated clearing file"
          />
          <button disabled={busy || !/^[a-f0-9-]{36}$/i.test(fileId)}>
            {busy
              ? "Capturing reconciliation…"
              : "Capture payment reconciliation"}
          </button>
        </form>
      )}
      {!runs.length ? (
        <p>No payment reconciliation has been captured for this snapshot.</p>
      ) : (
        <nav aria-label="Saved payment reconciliations">
          {runs.map((run) => (
            <a
              className="case-row"
              key={run.id}
              href={`#/finance/reports/${reportId}/reconciliations/${run.id}`}
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
                {run.timingCount} timing · {run.pendingCount} pending ·{" "}
                {run.notPostedCount} without a financial effect
              </p>
              <small>
                Clearing file {short(run.fileId)} · {time(run.createdAt)}
              </small>
            </a>
          ))}
        </nav>
      )}
      {detail && (
        <>
          <h3>
            Saved payment reconciliation {short(detail.reconciliation.id)}
          </h3>
          <dl className="evidence-details">
            <dt>Artifact digest</dt>
            <dd>{detail.digest}</dd>
            <dt>Scope</dt>
            <dd>{detail.reconciliation.scope}</dd>
            <dt>Report as of</dt>
            <dd>{time(detail.reconciliation.reportAsOf)}</dd>
            <dt>Core accounting as of</dt>
            <dd>{time(detail.reconciliation.coreAsOf)}</dd>
            <dt>Payment workflow as of</dt>
            <dd>{time(detail.reconciliation.paymentsAsOf)}</dd>
            <dt>Clearing file cutoff</dt>
            <dd>{time(detail.reconciliation.fileGeneratedAt)}</dd>
            <dt>Core posted principal</dt>
            <dd>
              {money({
                minorUnits: detail.reconciliation.corePrincipalTotal,
                currency: { code: "TST", scale: 2 },
              })}
            </dd>
            <dt>Core customer bank fees</dt>
            <dd>
              {money({
                minorUnits: detail.reconciliation.coreBankFeeTotal,
                currency: { code: "TST", scale: 2 },
              })}
            </dd>
            <dt>Matched principal</dt>
            <dd>
              {money({
                minorUnits: detail.reconciliation.matchedPrincipalTotal,
                currency: { code: "TST", scale: 2 },
              })}
            </dd>
          </dl>
          <h3>Provider file controls by currency</h3>
          {!detail.reconciliation.providerTotals.length ? (
            <p>The clearing file contains no rows.</p>
          ) : (
            detail.reconciliation.providerTotals.map((total) => (
              <p key={`${total.currencyCode}:${total.currencyScale}`}>
                {total.currencyCode} (scale {total.currencyScale}): principal{" "}
                {total.principal} minor units; provider fees {total.providerFee}{" "}
                minor units. Duplicate rows remain in these file controls.
              </p>
            ))
          )}
          <h3>Owned reconciliation differences</h3>
          {!detail.issues.length ? (
            <p>
              No owned differences were recorded. Pending work can remain inside
              its timing window.
            </p>
          ) : (
            detail.issues.map((issue) => (
              <p key={issue.id}>
                Row {issue.ordinal}: {issue.code.replaceAll("_", " ")} ·{" "}
                {issue.caseId ? (
                  <a href={`#/cases/${issue.caseId}`}>
                    Open reconciliation case {short(issue.caseId)}
                  </a>
                ) : (
                  "Case assignment pending"
                )}
              </p>
            ))
          )}
          <h3>Frozen payment lineage</h3>
          {page?.items.map((item) => (
            <article className="lineage-item" key={item.paymentId}>
              <h4>
                Payment{" "}
                <a href={`#/payments/${item.paymentId}`}>
                  {short(item.paymentId)}
                </a>
              </h4>
              <Badge
                tone={
                  item.classification === "MATCHED" ||
                  item.classification === "NOT_POSTED"
                    ? "good"
                    : "attention"
                }
              >
                {item.classification}
              </Badge>
              <p>
                Workflow: {item.paymentState ?? "Missing"} · Core:{" "}
                {item.coreOutcome ?? "Missing"}
              </p>
              <p>
                Principal:{" "}
                {item.corePrincipal ? money(item.corePrincipal) : "Unavailable"}{" "}
                · Bank fee:{" "}
                {item.coreBankFee ? money(item.coreBankFee) : "Unavailable"}
              </p>
              <p>
                Instruction digest: {item.instructionDigest ?? "Unavailable"}
              </p>
              <p>
                Journal batches:{" "}
                {item.journalBatchIds.length
                  ? item.journalBatchIds.join(", ")
                  : "None in this snapshot"}
              </p>
              {item.glExportIds.map((id) => (
                <p key={id}>
                  <a href={`#/finance/${id}`}>GL export {short(id)}</a>
                </p>
              ))}
              {item.providerRows.map((row, index) => (
                <p key={`${row.lineId}:${index}`}>
                  Clearing line {short(row.lineId)}: {row.principalMinor}{" "}
                  {row.currencyCode} minor units (scale {row.currencyScale});
                  provider fee {row.providerFeeMinor}; {row.businessDate}.
                  Evidence {short(row.evidenceId)} version {row.providerVersion}
                  .
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
              First payment lineage page
            </button>
            <button
              className="secondary"
              disabled={page?.nextAfter == null}
              onClick={() => void loadPage(page?.nextAfter ?? 0)}
            >
              Next payment lineage page
            </button>
          </div>
        </>
      )}
    </section>
  );
}
