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
import { FinanceCreditReconciliation } from "./FinanceCreditReconciliation";
import { FinanceReconciliation } from "./FinanceReconciliation";
import { FinanceGroups } from "./FinanceGroups";
import { FinanceRevisions } from "./FinanceRevisions";
const amount = (minor: string | null | undefined) =>
  minor == null
    ? "Unavailable"
    : money({ minorUnits: minor, currency: { code: "TST", scale: 2 } });
export function FinanceReports({
  access,
  selected,
  reconciliationId,
  creditReconciliationId,
  groupId,
}: {
  access: Access;
  selected?: string;
  reconciliationId?: string;
  creditReconciliationId?: string;
  groupId?: string;
}) {
  const { client } = useSession();
  const [reports, setReports] = useState<Schema["FinanceReport"][]>([]),
    [liquidity, setLiquidity] = useState<Schema["LiquiditySnapshot"]>();
  const [detail, setDetail] = useState<Schema["FinanceReportDetail"]>(),
    [page, setPage] = useState<Schema["FinanceReportPage"]>();
  const [error, setError] = useState(""),
    [busy, setBusy] = useState(false);
  const [fromDate, setFromDate] = useState(
    new Date().toISOString().slice(0, 10),
  );
  const [toDate, setToDate] = useState(
    new Date(Date.now() + 86400000).toISOString().slice(0, 10),
  );
  const [now, setNow] = useState(Date.now());
  const load = useCallback(async () => {
    try {
      const [list, cash] = await Promise.all([
        client.GET("/api/v1/staff/finance/reports").then(value),
        client.GET("/api/v1/staff/finance/liquidity").then(value),
      ]);
      setReports(list);
      setLiquidity(cash);
      if (selected) {
        setDetail(
          await client
            .GET("/api/v1/staff/finance/reports/{id}", {
              params: { path: { id: selected } },
            })
            .then(value),
        );
      } else {
        setDetail(undefined);
        setPage(undefined);
      }
      setError("");
    } catch (e) {
      setError(message(e));
      setLiquidity(undefined);
    }
  }, [client, selected]);
  const loadPage = useCallback(
    async (after: number) => {
      if (!selected) return;
      try {
        setPage(
          await client
            .GET("/api/v1/staff/finance/reports/{id}/items", {
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
    const timer = setInterval(() => void load(), 4000);
    return () => clearInterval(timer);
  }, [load, loadPage]);
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, []);
  async function create() {
    setBusy(true);
    setError("");
    try {
      const body = { fromDate, toDate };
      const digest = await crypto.subtle.digest(
        "SHA-256",
        new TextEncoder().encode(JSON.stringify(body)),
      );
      const scope =
        "pb:finance-report:" +
        Array.from(new Uint8Array(digest), (b) =>
          b.toString(16).padStart(2, "0"),
        ).join("");
      const key = sessionStorage.getItem(scope) ?? crypto.randomUUID();
      sessionStorage.setItem(scope, key);
      const saved = await client
        .POST("/api/v1/staff/finance/reports", {
          params: { header: { "Idempotency-Key": key } },
          body,
        })
        .then(value);
      sessionStorage.removeItem(scope);
      location.hash = "/finance/reports/" + saved.report.id;
    } catch (e) {
      setError(message(e));
    } finally {
      setBusy(false);
    }
  }
  const fresh =
    liquidity?.status === "FRESH" &&
    liquidity.validUntil != null &&
    Date.parse(liquidity.validUntil) > now &&
    now - Date.parse(liquidity.coreAsOf) <= 5000;
  const state =
    liquidity?.status === "FRESH" && !fresh
      ? "STALE"
      : (liquidity?.status ?? "UNAVAILABLE");
  return (
    <>
      {error && <Alert>{error}</Alert>}
      <section className="panel">
        <div className="panel-title">
          <h2>Liquidity evidence</h2>
          <Badge tone={fresh ? "good" : "attention"}>{state}</Badge>
        </div>
        <p>
          Independent synthetic clearing evidence. Available liquidity
          conservatively subtracts all Core outgoing commitments from provider
          available cash; expected funds remain separate.
        </p>
        <dl className="evidence-details">
          <dt>Available liquidity</dt>
          <dd data-testid="liquidity-available">
            {amount(fresh ? liquidity?.available : null)}
          </dd>
          <dt>Core reserved principal</dt>
          <dd>{amount(liquidity?.coreReservedPrincipal)}</dd>
          <dt>Core submitted principal</dt>
          <dd>{amount(liquidity?.coreSubmittedPrincipal)}</dd>
          <dt>Provider reserved</dt>
          <dd>{amount(fresh ? liquidity?.providerReserved : null)}</dd>
          <dt>Externally settled balance</dt>
          <dd>{amount(fresh ? liquidity?.externallySettled : null)}</dd>
          <dt>Expected incoming</dt>
          <dd>{amount(fresh ? liquidity?.expectedIncoming : null)}</dd>
          <dt>Expected outgoing</dt>
          <dd>{amount(fresh ? liquidity?.expectedOutgoing : null)}</dd>
          <dt>Core observation</dt>
          <dd>{liquidity ? time(liquidity.coreAsOf) : "Unavailable"}</dd>
          <dt>Provider observation</dt>
          <dd>
            {liquidity?.providerAsOf ? time(liquidity.providerAsOf) : "Missing"}
          </dd>
          <dt>Provider valid until</dt>
          <dd>
            {liquidity?.validUntil ? time(liquidity.validUntil) : "Missing"}
          </dd>
        </dl>
        {!fresh && (
          <p role="status">
            Current provider cash values are unavailable. Core balances are not
            a substitute for independent liquidity evidence.
          </p>
        )}
      </section>
      <section className="panel">
        <div className="panel-title">
          <h2>Finance report snapshots</h2>
          <button className="secondary" onClick={() => void load()}>
            Refresh reports and liquidity
          </button>
        </div>
        <p>
          UTC half-open periods, at most 1,000 Core batches. Each saved snapshot
          keeps its original source membership and evidence; create a new
          snapshot to observe later bookings.
        </p>
        {access.capabilities.includes("finance.recover") && (
          <form
            className="configuration-form"
            onSubmit={(e) => {
              e.preventDefault();
              void create();
            }}
          >
            <label>
              Report from date · inclusive
              <input
                type="date"
                value={fromDate}
                onChange={(e) => setFromDate(e.target.value)}
                required
              />
            </label>
            <label>
              Report to date · exclusive
              <input
                type="date"
                value={toDate}
                onChange={(e) => setToDate(e.target.value)}
                required
              />
            </label>
            <button disabled={busy || fromDate >= toDate}>
              {busy ? "Saving snapshot…" : "Save Finance snapshot"}
            </button>
          </form>
        )}
        {!reports.length ? (
          <p>No report snapshots saved.</p>
        ) : (
          reports.map((r) => (
            <a
              className="case-row"
              href={"#/finance/reports/" + r.id}
              key={r.id}
            >
              <div>
                <strong>Snapshot {short(r.id)}</strong>
                <Badge
                  tone={r.completeness === "COMPLETE" ? "good" : "attention"}
                >
                  {r.completeness}
                </Badge>
              </div>
              <p>
                {r.range.fromDate} to {r.range.toDate} · {r.matchedCount} of{" "}
                {r.sourceCount} GL sources matched
              </p>
              <small>
                Saved {time(r.createdAt)} · Debit {amount(r.coreDebitTotal)}
              </small>
            </a>
          ))
        )}
      </section>
      {detail && (
        <section className="panel">
          <div className="panel-title">
            <h2>Saved snapshot {short(detail.report.id)}</h2>
            <Badge
              tone={
                detail.report.completeness === "COMPLETE" ? "good" : "attention"
              }
            >
              {detail.report.completeness}
            </Badge>
          </div>
          <p>
            This immutable artifact describes the observations captured at the
            times below. Current liquidity appears separately above.
          </p>
          <dl className="evidence-details">
            <dt>Snapshot digest</dt>
            <dd>{detail.digest}</dd>
            <dt>Core as of</dt>
            <dd>{time(detail.report.coreAsOf)}</dd>
            <dt>Finance as of</dt>
            <dd>{time(detail.report.financeAsOf)}</dd>
            <dt>Core debit control</dt>
            <dd>{amount(detail.report.coreDebitTotal)}</dd>
            <dt>Core credit control</dt>
            <dd>{amount(detail.report.coreCreditTotal)}</dd>
            <dt>Matched GL debit</dt>
            <dd>{amount(detail.report.confirmedGlDebitTotal)}</dd>
            <dt>Matched GL credit</dt>
            <dd>{amount(detail.report.confirmedGlCreditTotal)}</dd>
            <dt>GL completeness</dt>
            <dd>{detail.report.glCompleteness}</dd>
            <dt>Liquidity at capture</dt>
            <dd>{detail.report.liquidity.status}</dd>
            <dt>Captured evidence now</dt>
            <dd>{detail.currentLiquidityStatus}</dd>
            <dt>Source controls</dt>
            <dd>
              {detail.report.matchedCount} matched ·{" "}
              {detail.report.missingCount} missing ·{" "}
              {detail.report.pendingCount} pending ·{" "}
              {detail.report.conflictingCount} conflicting
            </dd>
          </dl>
          {!!detail.report.reasonCodes.length && (
            <p>
              {detail.report.reasonCodes
                .map((c) => c.replaceAll("_", " "))
                .join(" · ")}
            </p>
          )}
          {!!detail.issues.length && (
            <>
              <h3>Owned snapshot exceptions</h3>
              {detail.issues.map((i) => (
                <article className="case-row" key={i.id}>
                  <strong>{i.code.replaceAll("_", " ")}</strong>
                  <p>
                    Observed in{" "}
                    {i.ordinal == null
                      ? "the whole snapshot"
                      : "source " + i.ordinal}
                  </p>
                  {i.caseId ? (
                    <a href={"#/cases/" + i.caseId}>
                      Review Finance case {short(i.caseId)}
                    </a>
                  ) : (
                    <span>Case assignment pending</span>
                  )}
                </article>
              ))}
            </>
          )}
          <h3>Frozen source lineage</h3>
          {!!detail.revisions.length && (
            <>
              <h3>Published report lineage</h3>
              {detail.revisions.map((r) => (
                <article className="case-row" key={r.proposalId}>
                  <strong>Approved revision {short(r.proposalId)}</strong>
                  <p>{r.reason}</p>
                  <p>
                    <a href={"#/finance/reports/" + r.originalReportId}>
                      Original snapshot {short(r.originalReportId)}
                    </a>
                    {" → "}
                    <a href={"#/finance/reports/" + r.replacementReportId}>
                      Replacement snapshot {short(r.replacementReportId)}
                    </a>
                  </p>
                  <small>
                    Published {time(r.publishedAt)} · Independent checker{" "}
                    {short(r.checkerPerson)}
                  </small>
                </article>
              ))}
            </>
          )}
          {page?.items.map((item) => (
            <article className="case-row" key={item.ordinal}>
              <div>
                <strong>
                  Source {item.ordinal} · {short(item.source.journal.id)}
                </strong>
                <Badge
                  tone={
                    item.classification === "MATCHED" ? "good" : "attention"
                  }
                >
                  {item.classification.replaceAll("_", " ")}
                </Badge>
              </div>
              <p>
                Core batch {item.source.journal.id} ·{" "}
                {item.source.journal.businessDate}
              </p>
              <small>Source digest {item.sourceDigest}</small>
              {item.exportId && (
                <p>
                  <a href={"#/finance/" + item.exportId}>
                    Inspect current export {short(item.exportId)}
                  </a>
                </p>
              )}
              {item.source.journal.entries.map((e) => (
                <p key={e.accountId}>
                  Account {short(e.accountId)} · {money(e.amount)}
                </p>
              ))}
              {item.receipt && (
                <p>
                  Captured GL receipt {item.receipt.providerJournalReference}
                </p>
              )}
            </article>
          ))}
          {page && (
            <div className="recovery-actions">
              <button
                className="secondary"
                disabled={page.items[0]?.ordinal === 1 || !page.items.length}
                onClick={() => void loadPage(0)}
              >
                First lineage page
              </button>
              <button
                className="secondary"
                disabled={page.nextAfter == null}
                onClick={() => void loadPage(page.nextAfter ?? 0)}
              >
                Next lineage page
              </button>
            </div>
          )}
        </section>
      )}
      {selected && (
        <>
          <FinanceReconciliation
            access={access}
            reportId={selected}
            selected={reconciliationId}
          />
          <FinanceCreditReconciliation
            access={access}
            reportId={selected}
            selected={creditReconciliationId}
          />
          <FinanceGroups
            access={access}
            reportId={selected}
            selected={groupId}
          />
        </>
      )}
      <FinanceRevisions access={access} reports={reports} onPublished={load} />
    </>
  );
}
