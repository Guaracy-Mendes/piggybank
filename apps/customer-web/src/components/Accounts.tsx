import { useState, useCallback, useEffect } from "react";
import {
  Alert,
  Badge,
  Busy,
  Icon,
  message,
  money,
  short,
  time,
  useSession,
  value,
  type Schema,
} from "@piggybank/web-core";
import type { Account } from "../types";
import { navigate, stableKey, doneKey } from "../lib/navigation";
import { PageTitle } from "../lib/display";

export function BalanceCard({ account }: { account: Account }) {
  const { client } = useSession();
  const [balance, setBalance] = useState<Schema["Balance"] | null>(null);
  const [error, setError] = useState("");
  const load = useCallback(async () => {
    try {
      setBalance(
        await value(
          await client.GET("/api/v1/accounts/{id}/balance", {
            params: { path: { id: account.id } },
          }),
        ),
      );
      setError("");
    } catch (e) {
      setError(message(e));
    }
  }, [client, account]);
  useEffect(() => {
    void load();
  }, [load]);
  return (
    <article className="balance-card">
      <div className="balance-orbit" aria-hidden="true" />
      <div className="spread">
        <span className="balance-label">Everyday account</span>
        <Badge tone={account.debitBlocked ? "attention" : "good"}>
          {account.state === "CLOSED"
            ? "Closed"
            : account.debitBlocked
              ? "Outgoing frozen"
              : "Active"}
        </Badge>
      </div>
      <div className="reference">•••• {short(account.id)}</div>
      <div className="balance-main">
        <span>Available balance</span>
        <strong
          className={`money ${balance && balance.available.minorUnits.length > 15 ? "long-money" : ""}`}
        >
          {balance ? money(balance.available) : "—"}
        </strong>
      </div>
      {error ? (
        <div role="alert">
          {error}{" "}
          <button className="quiet light" onClick={() => void load()}>
            Refresh balance
          </button>
        </div>
      ) : balance ? (
        <>
          <div className="balance-parts">
            <div>
              <span>Posted</span>
              <strong>{money(balance.booked)}</strong>
            </div>
            <div>
              <span>Held</span>
              <strong>{money(balance.held)}</strong>
            </div>
          </div>
          <div className="balance-asof spread">
            <span>From your account · {time(balance.asOf)}</span>
            <button
              className="quiet light balance-refresh"
              onClick={() => void load()}
            >
              Refresh
            </button>
          </div>
        </>
      ) : (
        <Busy label="Updating your balance…" />
      )}
    </article>
  );
}

export function AccountsView({
  accounts,
  selected,
  refresh,
  notify,
}: {
  accounts: Account[];
  selected?: string;
  refresh: () => Promise<void>;
  notify: (text: string) => void;
}) {
  const { client } = useSession();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const account =
    accounts.find((a) => a.id === selected) ??
    accounts.find((a) => a.state === "ACTIVE") ??
    accounts[0];
  async function action(kind: "freeze" | "closure" | "service-requests") {
    if (!account) return;
    if (
      kind === "closure" &&
      !window.confirm(
        "Request closure for this account? Current balances, holds and obligations will be checked.",
      )
    )
      return;
    setBusy(true);
    setError("");
    try {
      if (kind === "service-requests") {
        const key = stableKey("pb:support:" + account.id);
        const result = await value(
          await client.POST("/api/v1/accounts/{id}/service-requests", {
            params: {
              path: { id: account.id },
              header: { "Idempotency-Key": key },
            },
          }),
        );
        doneKey("pb:support:" + account.id);
        notify("Your support request is saved: " + short(result.caseId));
      } else if (kind === "freeze") {
        await value(
          await client.POST("/api/v1/accounts/{id}/freeze", {
            params: { path: { id: account.id } },
          }),
        );
        notify(
          "Outgoing activity is frozen. Contact support when you need it reviewed.",
        );
      } else {
        await value(
          await client.POST("/api/v1/accounts/{id}/closure", {
            params: { path: { id: account.id } },
          }),
        );
        notify("Your account is closed. Its history remains available.");
      }
      await refresh();
    } catch (e) {
      setError(message(e));
    } finally {
      setBusy(false);
    }
  }
  return (
    <>
      <PageTitle
        eyebrow="Your everyday accounts"
        title="A clearer view of your money."
        description="Available, posted and held balances are shown separately, directly from your account."
      />
      {!account ? (
        <div className="card">
          <h2>Your first account starts with an application.</h2>
          <a className="text-link" href="#/applications">
            Explore your account options
          </a>
        </div>
      ) : (
        <>
          <div
            className="account-selector"
            role="group"
            aria-label="Choose account"
          >
            {accounts.map((item) => (
              <button
                key={item.id}
                className={item.id === account.id ? "" : "secondary"}
                onClick={() => navigate("/accounts/" + item.id)}
              >
                •••• {short(item.id)}{" "}
                {item.state === "CLOSED" ? "· Closed" : ""}
              </button>
            ))}
          </div>
          {error ? <Alert>{error}</Alert> : null}
          <div className="account-detail-grid">
            <BalanceCard
              key={account.id + account.revision}
              account={account}
            />
            <div className="card">
              <span className="eyebrow">Account care</span>
              <h2>You're in control.</h2>
              <p className="muted">
                Freeze outgoing activity, ask for help, or request closure once
                balances and obligations are clear.
              </p>
              <div className="account-actions">
                {account.state !== "CLOSED" ? (
                  <>
                    <button
                      className="secondary"
                      disabled={busy || account.debitBlocked}
                      onClick={() => void action("freeze")}
                    >
                      {account.debitBlocked
                        ? "Outgoing frozen"
                        : "Freeze outgoing activity"}
                    </button>
                    <button
                      className="quiet"
                      disabled={busy}
                      onClick={() => void action("closure")}
                    >
                      Request account closure
                    </button>
                  </>
                ) : (
                  <Badge tone="good">Closed · History retained</Badge>
                )}
                <button
                  className="quiet"
                  disabled={busy}
                  onClick={() => void action("service-requests")}
                >
                  <Icon name="support" /> Request support
                </button>
              </div>
            </div>
          </div>
          <History key={account.id} account={account} />
        </>
      )}
    </>
  );
}

export function History({ account }: { account: Account }) {
  const { client } = useSession();
  const day = new Date();
  const initialTo = new Date(day.getTime() + 86400000)
    .toISOString()
    .slice(0, 10);
  const initialFrom = new Date(day.getTime() - 90 * 86400000)
    .toISOString()
    .slice(0, 10);
  const [from, setFrom] = useState(initialFrom);
  const [to, setTo] = useState(initialTo);
  const [page, setPage] = useState<Schema["HistoryPage"] | null>(null);
  const [items, setItems] = useState<Schema["Movement"][]>([]);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [range, setRange] = useState({ from: initialFrom, to: initialTo });
  const load = useCallback(
    async (next?: Schema["HistoryCursor"], period = range) => {
      setBusy(true);
      try {
        const result = await value(
          await client.GET("/api/v1/accounts/{id}/transactions", {
            params: {
              path: { id: account.id },
              query: {
                from: period.from,
                to: period.to,
                limit: 20,
                ...(next
                  ? { snapshotId: next.snapshotId, after: next.after }
                  : {}),
              },
            },
          }),
        );
        setPage(result);
        setItems((old) => (next ? [...old, ...result.items] : result.items));
        setError("");
      } catch (e) {
        setError(message(e));
      } finally {
        setBusy(false);
      }
    },
    [client, account.id, range],
  );
  useEffect(() => {
    void load();
  }, [load]);
  return (
    <section className="card section-space">
      <div className="section-title">
        <div>
          <h2>Transaction history</h2>
          <p className="hint">
            Posted movements and linked corrections, kept in a consistent
            snapshot.
          </p>
        </div>
        <button
          className="secondary"
          disabled={busy}
          onClick={() => void load()}
        >
          Refresh history
        </button>
      </div>
      <form
        className="history-filter"
        onSubmit={(e) => {
          e.preventDefault();
          setRange({ from, to });
        }}
      >
        <label>
          From
          <input
            type="date"
            required
            value={from}
            onChange={(e) => setFrom(e.target.value)}
          />
        </label>
        <label>
          To, exclusive
          <input
            type="date"
            required
            value={to}
            onChange={(e) => setTo(e.target.value)}
          />
        </label>
        <button className="secondary" disabled={busy}>
          Apply dates
        </button>
      </form>
      {error ? <Alert>{error}</Alert> : null}
      {busy && !page ? (
        <Busy />
      ) : items.length ? (
        <div className="table-wrap">
          <table>
            <caption className="visually-hidden">
              Posted account movements
            </caption>
            <thead>
              <tr>
                <th>Date</th>
                <th>Movement</th>
                <th>Reference</th>
                <th>Amount</th>
              </tr>
            </thead>
            <tbody>
              {items.map((item) => (
                <tr key={item.id}>
                  <td>{item.businessDate}</td>
                  <td>
                    <strong>
                      {item.postingKind.toLowerCase().replaceAll("_", " ")}
                    </strong>
                    <div className="hint">
                      {item.originalBatchId
                        ? "Linked to " + short(item.originalBatchId)
                        : "Posted"}
                    </div>
                  </td>
                  <td className="reference">{short(item.batchId)}</td>
                  <td className="money">{money(item.amount)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      ) : (
        <div className="empty">No posted movements in this period.</div>
      )}
      {page ? (
        <div className="spread history-footer">
          <small className="muted">
            {items.length} of {page.total} movements · Snapshot{" "}
            {time(page.asOf)}
          </small>
          {page.next ? (
            <button
              className="secondary"
              disabled={busy}
              onClick={() => void load(page.next!)}
            >
              Load more
            </button>
          ) : null}
        </div>
      ) : null}
    </section>
  );
}
