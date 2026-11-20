import { useCallback, useEffect, useState } from "react";
import { Alert, message, short, useSession, value } from "@piggybank/web-core";
import { doneKey, stableKey } from "../lib/navigation";

/* WP-35 synthetic ATM access: a card token linked to one of your accounts. Cash withdrawal and balance inquiry only,
   within daily limits; block it at any time. (No physical card is issued in this demonstration.) */
type Card = { id: string; token: string; accountId: string; state: string };
type Account = { id: string; state: string };
export function AtmAccess() {
  const { client } = useSession();
  const [cards, setCards] = useState<Card[]>([]);
  const [accounts, setAccounts] = useState<Account[]>([]);
  const [account, setAccount] = useState("");
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const load = useCallback(async () => {
    try {
      const [c, a] = await Promise.all([
        client.GET("/api/v1/atm/cards").then(value),
        client.GET("/api/v1/accounts").then(value),
      ]);
      setCards(c as unknown as Card[]);
      setAccounts(
        (a as unknown as Account[]).filter((x) => x.state === "ACTIVE"),
      );
    } catch (e) {
      setError(message(e));
    }
  }, [client]);
  useEffect(() => {
    void load();
  }, [load]);
  async function issue() {
    const scope = "pb:atm-card:" + account;
    try {
      await client
        .POST("/api/v1/atm/cards", {
          params: { header: { "Idempotency-Key": stableKey(scope) } },
          body: { accountId: account },
        })
        .then(value);
      doneKey(scope);
      setNotice("ATM access added.");
      void load();
    } catch (e) {
      setError(message(e));
    }
  }
  async function block(card: Card) {
    try {
      await client
        .POST("/api/v1/atm/cards/{id}/block", {
          params: { path: { id: card.id } },
        })
        .then(value);
      setNotice("ATM access blocked. It cannot be used again.");
      void load();
    } catch (e) {
      setError(message(e));
    }
  }
  return (
    <div>
      <h3>ATM access</h3>
      <p className="muted">
        Withdraw cash and check your balance at an ATM, up to TST 500.00 per
        withdrawal and TST 1,000.00 a day.
      </p>
      {error ? <Alert>{error}</Alert> : null}
      {notice ? <p role="status">{notice}</p> : null}
      {cards.map((c) => (
        <div className="row" key={c.id}>
          <span>
            Card {c.token.slice(-4)} · account {short(c.accountId)} ·{" "}
            {c.state.toLowerCase()}
          </span>
          {c.state === "ACTIVE" ? (
            <button className="secondary" onClick={() => void block(c)}>
              Block card ending {c.token.slice(-4)}
            </button>
          ) : null}
        </div>
      ))}
      {accounts.length ? (
        <div className="row">
          <label htmlFor="atm-account">Account for ATM access</label>
          <select
            id="atm-account"
            value={account}
            onChange={(e) => setAccount(e.target.value)}
          >
            <option value="">Choose an account</option>
            {accounts.map((a) => (
              <option key={a.id} value={a.id}>
                Account {short(a.id)}
              </option>
            ))}
          </select>
          <button
            className="secondary"
            disabled={!account}
            onClick={() => void issue()}
          >
            Add ATM access
          </button>
        </div>
      ) : null}
    </div>
  );
}
