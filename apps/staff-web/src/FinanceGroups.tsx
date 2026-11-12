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
const amount = (minor: string | null | undefined) =>
  minor == null
    ? "Unavailable"
    : money({ minorUnits: minor, currency: { code: "TST", scale: 2 } });
export function FinanceGroups({
  access,
  reportId,
  selected,
}: {
  access: Access;
  reportId: string;
  selected?: string;
}) {
  const { client } = useSession();
  const [runs, setRuns] = useState<Schema["FinanceGroupReconciliation"][]>([]);
  const [detail, setDetail] =
    useState<Schema["FinanceGroupReconciliationDetail"]>();
  const [page, setPage] = useState<Schema["FinanceGroupReconciliationPage"]>();
  const [setId, setSetId] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const canCreate = access.capabilities.includes("finance.recover");
  const load = useCallback(async () => {
    try {
      setRuns(
        await client
          .GET("/api/v1/staff/finance/reconciliation-groups", {
            params: { query: { reportId } },
          })
          .then(value),
      );
      if (selected) {
        const d = await client
          .GET("/api/v1/staff/finance/reconciliation-groups/{id}", {
            params: { path: { id: selected } },
          })
          .then(value);
        if (d.reconciliation.reportId !== reportId)
          throw new Error("This group belongs to another report.");
        setDetail(d);
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
            .GET("/api/v1/staff/finance/reconciliation-groups/{id}/items", {
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
    const scope = `pb:finance-group:${reportId}:${setId}`;
    const key = sessionStorage.getItem(scope) ?? crypto.randomUUID();
    sessionStorage.setItem(scope, key);
    try {
      const d = await client
        .POST("/api/v1/staff/finance/reconciliation-groups", {
          params: { header: { "Idempotency-Key": key } },
          body: { reportId, setId },
        })
        .then(value);
      sessionStorage.removeItem(scope);
      location.hash = `/finance/reports/${reportId}/groups/${d.reconciliation.id}`;
    } catch (e) {
      setError(message(e));
    } finally {
      setBusy(false);
    }
  }
  return (
    <section className="panel" aria-label="Grouped clearing reconciliation">
      <div className="panel-title">
        <h2>Reconcile a clearing file group</h2>
        <button className="secondary" onClick={() => void load()}>
          Refresh clearing groups
        </button>
      </div>
      <p>
        A signed provider manifest declares every member file, its digest and
        row count, with one UTC period and cutoff. Match up to ten files and
        1,000 provider rows in one frozen observation. Cross-file duplicates
        remain visible. An unavailable or changed member prevents capture.
      </p>
      <p>
        The existing bank-source and journal snapshot bounds still apply. New
        facts need a new report or observation; saved groups retain their
        original evidence.
      </p>
      {error && <Alert>{error}</Alert>}
      {canCreate && (
        <form onSubmit={(e) => void create(e)}>
          <label htmlFor="settlement-set">Published settlement set ID</label>
          <input
            id="settlement-set"
            value={setId}
            onChange={(e) => setSetId(e.target.value.trim())}
            required
            pattern="[a-fA-F0-9-]{36}"
          />
          <button disabled={busy || !/^[a-f0-9-]{36}$/i.test(setId)}>
            {busy ? "Capturing clearing group…" : "Capture clearing group"}
          </button>
        </form>
      )}
      {!runs.length ? (
        <p>No clearing group has been captured for this snapshot.</p>
      ) : (
        <nav aria-label="Saved clearing groups">
          {runs.map((r) => (
            <a
              className="case-row"
              key={r.id}
              href={`#/finance/reports/${reportId}/groups/${r.id}`}
              aria-current={selected === r.id ? "page" : undefined}
            >
              <strong>{short(r.id)}</strong>{" "}
              <Badge
                tone={r.completeness === "COMPLETE" ? "good" : "attention"}
              >
                {r.completeness}
              </Badge>
              <p>
                {r.kind === "OUTGOING"
                  ? "Outgoing payments"
                  : "Incoming and returned payments"}{" "}
                · {r.members.length} files · {r.providerRowCount} provider rows
              </p>
              <p>
                {r.matchedCount} matched · {r.exceptionCount} exceptions ·{" "}
                {r.timingCount} timing · {r.pendingCount} pending ·{" "}
                {r.notPostedCount} without a financial effect
              </p>
              {r.coreSuspensePrincipalTotal != null && (
                <p>
                  Principal still in suspense:{" "}
                  {amount(r.coreSuspensePrincipalTotal)}
                </p>
              )}
              <small>{time(r.createdAt)}</small>
            </a>
          ))}
        </nav>
      )}
      {detail && (
        <>
          <h3>Saved clearing group {short(detail.reconciliation.id)}</h3>
          <dl className="evidence-details">
            <dt>Group artifact digest</dt>
            <dd>{detail.digest}</dd>
            <dt>Group scope</dt>
            <dd>{detail.reconciliation.scope}</dd>
            <dt>Provider set identity</dt>
            <dd>{detail.reconciliation.setId}</dd>
            <dt>Provider set digest</dt>
            <dd>{detail.reconciliation.setDigest}</dd>
            <dt>Period</dt>
            <dd>
              {detail.reconciliation.range.fromDate} through{" "}
              {detail.reconciliation.range.toDate} (exclusive), UTC
            </dd>
            <dt>Common provider cutoff</dt>
            <dd>{time(detail.reconciliation.providerCutoff)}</dd>
            <dt>Parent report as of</dt>
            <dd>{time(detail.reconciliation.reportAsOf)}</dd>
            <dt>Native accounting as of</dt>
            <dd>{time(detail.reconciliation.coreAsOf)}</dd>
            <dt>Workflow as of</dt>
            <dd>{time(detail.reconciliation.paymentsAsOf)}</dd>
            <dt>Native principal</dt>
            <dd>{amount(detail.reconciliation.corePrincipalTotal)}</dd>
            <dt>Matched principal</dt>
            <dd>{amount(detail.reconciliation.matchedPrincipalTotal)}</dd>
            {detail.reconciliation.kind === "OUTGOING" ? (
              <>
                <dt>Original bank fees</dt>
                <dd>{amount(detail.reconciliation.coreBankFeeTotal)}</dd>
              </>
            ) : (
              <>
                <dt>Allocated principal</dt>
                <dd>
                  {amount(detail.reconciliation.coreAllocatedPrincipalTotal)}
                </dd>
                <dt>Principal in suspense</dt>
                <dd>
                  {amount(detail.reconciliation.coreSuspensePrincipalTotal)}
                </dd>
              </>
            )}
          </dl>
          <h3>Closed provider file membership</h3>
          {detail.reconciliation.members.map((m) => (
            <article className="lineage-item" key={m.fileId}>
              <p>
                Member file {m.fileId}: {m.rowCount} rows
              </p>
              <p>Sealed file digest: {m.digest}</p>
            </article>
          ))}
          <h3>Grouped provider controls by currency</h3>
          {!detail.reconciliation.providerTotals.length ? (
            <p>The declared files contain no provider rows.</p>
          ) : (
            detail.reconciliation.providerTotals.map((t) => (
              <p key={`${t.currencyCode}:${t.currencyScale}`}>
                {t.currencyCode} (scale {t.currencyScale}): principal{" "}
                {t.principal} minor units; provider fee {t.providerFee} minor
                units. Duplicate rows remain in these controls.
              </p>
            ))
          )}
          <h3>Owned group differences</h3>
          {!detail.issues.length ? (
            <p>
              No owned group differences were recorded. Short timing grace may
              still apply.
            </p>
          ) : (
            detail.issues.map((i) => (
              <p key={i.id}>
                Row {i.ordinal}: {i.code.replaceAll("_", " ")} ·{" "}
                {i.caseId ? (
                  <a href={`#/cases/${i.caseId}`}>
                    Open group reconciliation case {short(i.caseId)}
                  </a>
                ) : (
                  "Case assignment pending"
                )}
              </p>
            ))
          )}
          <h3>Frozen group lineage</h3>
          {page?.items.map((item) => (
            <article className="lineage-item" key={item.operationId}>
              <h4>
                {item.outgoing ? "Payment" : "Credit movement"}{" "}
                {item.outgoing ? (
                  <a href={`#/payments/${item.operationId}`}>
                    {short(item.operationId)}
                  </a>
                ) : (
                  short(item.operationId)
                )}
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
              {item.outgoing && (
                <>
                  <p>
                    Principal:{" "}
                    {item.outgoing.corePrincipal
                      ? money(item.outgoing.corePrincipal)
                      : "Unavailable"}{" "}
                    · Bank fee:{" "}
                    {item.outgoing.coreBankFee
                      ? money(item.outgoing.coreBankFee)
                      : "Unavailable"}
                  </p>
                  <p>
                    Workflow: {item.outgoing.paymentState ?? "Not captured"} ·
                    Core outcome: {item.outgoing.coreOutcome ?? "Not captured"}
                  </p>
                  <p>Journal: {item.outgoing.coreBatchId ?? "Not captured"}</p>
                  {item.outgoing.glExportIds.map((id) => (
                    <p key={id}>
                      <a href={`#/finance/${id}`}>
                        Grouped GL export {short(id)}
                      </a>
                    </p>
                  ))}
                </>
              )}
              {item.credit && (
                <>
                  <p>
                    {item.credit.kind ?? "Unverified credit kind"} · Principal:{" "}
                    {item.credit.principal
                      ? money(item.credit.principal)
                      : "Unavailable"}{" "}
                    · Workflow: {item.credit.paymentState ?? "Not captured"}
                  </p>
                  <p>
                    Suspense journal:{" "}
                    {item.credit.suspenseBatchId ?? "Not captured"}
                  </p>
                  <p>
                    Allocation journal:{" "}
                    {item.credit.allocationBatchId ?? "Not captured"}
                  </p>
                  {item.credit.originalPaymentId && (
                    <p>
                      Original payment{" "}
                      <a href={`#/payments/${item.credit.originalPaymentId}`}>
                        {short(item.credit.originalPaymentId)}
                      </a>{" "}
                      · Lifetime returned principal:{" "}
                      {amount(item.credit.returnedPrincipalTotal)}
                    </p>
                  )}
                  {item.credit.glExportIds.map((id) => (
                    <p key={id}>
                      <a href={`#/finance/${id}`}>
                        Grouped credit GL export {short(id)}
                      </a>
                    </p>
                  ))}
                </>
              )}
              {!item.sourceRows.length ? (
                <p>No provider row was captured for this operation.</p>
              ) : (
                item.sourceRows.map((r, index) => {
                  const claim =
                    item.outgoing?.providerRows[index] ??
                    item.credit?.providerRows[index];
                  return (
                    <div key={`${r.fileId}:${r.rowOrdinal}`}>
                      <p>
                        Provider file {r.fileId}, row {r.rowOrdinal}, line{" "}
                        {r.lineId}. Row digest {r.rowDigest}.
                      </p>
                      {claim && (
                        <p>
                          Provider claim: {claim.principalMinor}{" "}
                          {claim.currencyCode} minor units (scale{" "}
                          {claim.currencyScale}); provider fee{" "}
                          {claim.providerFeeMinor}; {claim.businessDate}.
                          Evidence {claim.evidenceId} version{" "}
                          {claim.providerVersion}.
                        </p>
                      )}
                    </div>
                  );
                })
              )}
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
              First group lineage page
            </button>
            <button
              className="secondary"
              disabled={page?.nextAfter == null}
              onClick={() => void loadPage(page?.nextAfter ?? 0)}
            >
              Next group lineage page
            </button>
          </div>
        </>
      )}
    </section>
  );
}
