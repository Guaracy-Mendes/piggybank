import { useCallback, useEffect, useRef, useState } from "react";
import {
  Alert,
  Badge,
  message,
  money,
  short,
  useSession,
  value,
  type Schema,
} from "@piggybank/web-core";
import type { Access } from "./App";
type Detail = Schema["PeriodicBasisDetail"];
const amount = (minorUnits: string) =>
  money({ minorUnits, currency: { code: "TST", scale: 2 } });
export function FinanceBases({
  access,
  request,
  selected,
}: {
  access: Access;
  request: Schema["PeriodicPolicyRequest"];
  selected?: string;
}) {
  const { client } = useSession();
  const [records, setRecords] = useState<Detail[]>([]),
    [detail, setDetail] = useState<Detail>();
  const [items, setItems] = useState<Schema["PeriodicBasisItem"][]>([]),
    [next, setNext] = useState<number | null>(null);
  const [error, setError] = useState(""),
    [notice, setNotice] = useState(""),
    [busy, setBusy] = useState(false);
  const [account, setAccount] = useState(""),
    [period, setPeriod] = useState(new Date().toISOString().slice(0, 10));
  const scope = request.policy.id + ":" + (selected ?? "");
  const live = useRef(scope),
    generation = useRef(0);
  live.current = scope;
  const load = useCallback(async () => {
    const revision = ++generation.current;
    try {
      const [list, result, page] = await Promise.all([
        client
          .GET("/api/v1/staff/finance/periodic-bases", {
            params: { query: { policyId: request.policy.id } },
          })
          .then(value),
        selected
          ? client
              .GET("/api/v1/staff/finance/periodic-bases/{id}", {
                params: { path: { id: selected } },
              })
              .then(value)
          : Promise.resolve(undefined),
        selected
          ? client
              .GET("/api/v1/staff/finance/periodic-bases/{id}/items", {
                params: { path: { id: selected }, query: { limit: 5 } },
              })
              .then(value)
          : Promise.resolve(undefined),
      ]);
      if (live.current !== scope || revision !== generation.current) return;
      if (
        result &&
        (result.basis.policy.id !== request.policy.id ||
          page?.digest !== result.digest)
      )
        throw new Error(
          "Saved basis does not match this policy or source page.",
        );
      setRecords(list);
      setDetail(result);
      setItems(page?.items ?? []);
      setNext(page?.nextAfter ?? null);
      setError("");
    } catch (e) {
      if (live.current === scope && revision === generation.current)
        setError(message(e));
    }
  }, [client, request.policy.id, scope, selected]);
  useEffect(() => {
    live.current = scope;
    setDetail(undefined);
    setItems([]);
    setNext(null);
    setNotice("");
    void load();
    return () => {
      generation.current++;
      if (live.current === scope) live.current = "";
    };
  }, [load, scope]);
  async function capture(fresh: boolean) {
    const expected = scope;
    setBusy(true);
    setError("");
    setNotice("");
    try {
      const body = {
        policyId: request.policy.id,
        accountId: account,
        periodStart: period,
      };
      const digest = new Uint8Array(
        await crypto.subtle.digest(
          "SHA-256",
          new TextEncoder().encode(JSON.stringify(body)),
        ),
      );
      const storage =
        "pb:periodic-basis:" +
        access.personId +
        ":" +
        Array.from(digest, (b) => b.toString(16).padStart(2, "0")).join("");
      const key =
        (!fresh && sessionStorage.getItem(storage)) || crypto.randomUUID();
      sessionStorage.setItem(storage, key);
      const result = await client
        .POST("/api/v1/staff/finance/periodic-bases", {
          params: { header: { "Idempotency-Key": key } },
          body,
        })
        .then(value);
      if (live.current !== expected) return;
      location.hash =
        "/finance/policies/" + request.id + "/bases/" + result.basis.id;
      setNotice(
        "Native observation saved. Its original clock and source facts are retained on retry.",
      );
      if (selected === result.basis.id) await load();
    } catch (e) {
      if (live.current === expected) setError(message(e));
    } finally {
      setBusy(false);
    }
  }
  async function more() {
    if (!detail || next === null) return;
    const expected = scope,
      digest = detail.digest,
      after = next;
    setBusy(true);
    try {
      const page = await client
        .GET("/api/v1/staff/finance/periodic-bases/{id}/items", {
          params: { path: { id: detail.basis.id }, query: { after, limit: 5 } },
        })
        .then(value);
      if (live.current !== expected) return;
      if (page.digest !== digest)
        throw new Error("Native basis source digest changed.");
      setItems((old) => [...old, ...page.items]);
      setNext(page.nextAfter);
    } catch (e) {
      if (live.current === expected) setError(message(e));
    } finally {
      setBusy(false);
    }
  }
  const basis = detail?.basis;
  return (
    <section className="panel" aria-label="Native ledger basis">
      <h2>Native ledger basis</h2>
      <p>
        Core captures the account journal prefix by business date and its own
        UTC clock. No operator balance or evaluation clock is accepted. Captures
        do not enroll accounts, post money, or establish an amount owed.
      </p>
      <p>
        At most 1,000 journal facts, including opening history. Larger histories
        require a later accounting capability; a capture never truncates them.
      </p>
      {error && <Alert>{error}</Alert>}
      {notice && (
        <p role="status" className="notice">
          {notice}
        </p>
      )}
      {access.capabilities.includes("finance.recover") && (
        <form
          className="configuration-form"
          onSubmit={(e) => {
            e.preventDefault();
            void capture(false);
          }}
        >
          <label htmlFor="basis-account">Basis account ID</label>
          <input
            id="basis-account"
            value={account}
            onChange={(e) => setAccount(e.target.value)}
            pattern="[a-f0-9-]{36}"
            required
          />
          <label htmlFor="basis-period">Basis period start</label>
          <input
            id="basis-period"
            value={period}
            onChange={(e) => setPeriod(e.target.value)}
            type="date"
            required
          />
          <button disabled={busy}>Capture or retry basis</button>
          <button
            className="secondary"
            type="button"
            disabled={busy || !/^[a-f0-9-]{36}$/.test(account) || !period}
            onClick={() => void capture(true)}
          >
            Capture fresh observation
          </button>
          <p className="wide">
            Retry preserves the saved observation for these inputs. Use a fresh
            observation to include later committed facts, including late
            business-date postings.
          </p>
        </form>
      )}
      <button className="secondary" onClick={() => void load()}>
        Refresh native bases
      </button>
      <div className="queue-layout">
        <div>
          <h3>Saved native bases</h3>
          <p>Latest 100 for this exact published policy.</p>
          {!records.length && <p>No native bases captured.</p>}
          {records.map((r) => (
            <a
              className="case-row"
              href={"#/finance/policies/" + request.id + "/bases/" + r.basis.id}
              key={r.basis.id}
              aria-current={r.basis.id === selected ? "page" : undefined}
            >
              <strong>
                {r.basis.periodStart} · {short(r.basis.accountId)}
              </strong>
              <Badge>{r.basis.periodStatus}</Badge>
              <small>{r.basis.coreAsOf} · UTC</small>
            </a>
          ))}
        </div>
        {detail && basis && basis.id === selected && (
          <div aria-label="Selected native basis">
            <h3>Captured Core observation</h3>
            <Badge>{basis.periodStatus}</Badge>
            <p className="notice">
              {basis.periodStatus === "OPEN_PERIOD_PROVISIONAL"
                ? "Open period — provisional calculation. The period is still accepting facts."
                : "Closed period observation. This is not a global book close or proof that all late facts have arrived."}
            </p>
            <dl className="evidence-details">
              <dt>Account</dt>
              <dd className="mono">{basis.accountId}</dd>
              <dt>Business-date period</dt>
              <dd>
                {basis.periodStart} inclusive to {basis.periodEnd} exclusive
              </dd>
              <dt>Captured at (UTC)</dt>
              <dd>{basis.coreAsOf}</dd>
              <dt>Due at (UTC)</dt>
              <dd>{basis.dueAt}</dd>
              <dt>Account registered at (UTC)</dt>
              <dd>{basis.accountRegisteredAt}</dd>
              <dt>Captured account state</dt>
              <dd>
                {basis.accountState} · debit blocked:{" "}
                {basis.debitBlocked ? "Yes" : "No"} · credit blocked:{" "}
                {basis.creditBlocked ? "Yes" : "No"}
              </dd>
              <dt>Opening booked</dt>
              <dd data-testid="basis-opening">
                {amount(basis.openingBooked)} · {basis.openingBooked} minor
                units
              </dd>
              <dt>Period change</dt>
              <dd>
                {amount(basis.periodChange)} · {basis.periodChange} minor units
              </dd>
              <dt>Observed closing booked</dt>
              <dd data-testid="basis-closing">
                {amount(basis.observedClosingBooked)} ·{" "}
                {basis.observedClosingBooked} minor units
              </dd>
              <dt>Current booked at capture</dt>
              <dd>
                {amount(basis.currentBooked)} · {basis.currentBooked} minor
                units
              </dd>
              <dt>Current held at capture</dt>
              <dd>
                {amount(basis.currentHeld)} · {basis.currentHeld} minor units
              </dd>
              <dt>Current available at capture</dt>
              <dd>
                {amount(basis.currentAvailable)} · {basis.currentAvailable}{" "}
                minor units
              </dd>
              <dt>Basis source</dt>
              <dd>{basis.source}</dd>
              <dt>Source facts</dt>
              <dd>{basis.sourceCount}</dd>
              <dt>Source digest</dt>
              <dd className="mono">{basis.sourceDigest}</dd>
              <dt>Artifact digest</dt>
              <dd className="mono">{detail.digest}</dd>
              <dt>Policy digest</dt>
              <dd className="mono">{basis.policyDigest}</dd>
              <dt>Canonical capturing person</dt>
              <dd className="mono">{basis.capturedPersonId}</dd>
            </dl>
            <h3>Calculation from captured facts</h3>
            {detail.calculation ? (
              <dl className="evidence-details">
                <dt>Preview amount</dt>
                <dd>
                  {money(detail.calculation.amount)} ·{" "}
                  {detail.calculation.amount.minorUnits} minor units
                </dd>
                <dt>Exact fraction</dt>
                <dd>
                  {detail.calculation.exactNumerator} /{" "}
                  {detail.calculation.exactDenominator}
                </dd>
                <dt>Due at captured clock</dt>
                <dd>{detail.calculation.isDue ? "Yes" : "No"}</dd>
                <dt>Provisional</dt>
                <dd>{detail.calculation.provisional ? "Yes" : "No"}</dd>
              </dl>
            ) : (
              <p role="status">
                Calculation unavailable: {detail.calculationReason}. Account
                registration is not evidence of historical enrollment or
                customer consent.
              </p>
            )}
            <h3>Native source facts</h3>
            {items.map((i) => (
              <article
                className="case-row"
                key={i.ordinal}
                data-testid="basis-source"
              >
                <strong>
                  Fact {i.ordinal} · {i.postingKind}
                </strong>
                <p>
                  {i.businessDate} · {i.deltaMinor} minor units
                </p>
                <dl className="evidence-details">
                  <dt>Journal</dt>
                  <dd className="mono">{i.batchId}</dd>
                  <dt>Operation</dt>
                  <dd className="mono">{i.operationId}</dd>
                  <dt>Posted at (UTC)</dt>
                  <dd>{i.postedAt}</dd>
                  <dt>Journal fingerprint</dt>
                  <dd className="mono">{i.journalFingerprint}</dd>
                  {i.originalBatchId && (
                    <>
                      <dt>Original journal</dt>
                      <dd className="mono">{i.originalBatchId}</dd>
                    </>
                  )}
                </dl>
              </article>
            ))}
            {!items.length && (
              <p>No selected native journal facts before period end.</p>
            )}
            {next !== null && (
              <button disabled={busy} onClick={() => void more()}>
                Load more basis facts
              </button>
            )}
          </div>
        )}
      </div>
    </section>
  );
}
