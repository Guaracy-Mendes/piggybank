import { useCallback, useEffect, useState } from "react";
import { Alert, message, short, useSession, value } from "@piggybank/web-core";
import type { Account } from "../types";
import { doneKey, stableKey } from "../lib/navigation";

/* WP-36 next-day transfers on the synthetic batch rail (SIM_BATCH). Approval uses the same identity confirmation as
   every payment; money is held when you confirm and leaves when the batch settles on the settlement date. */
type Recipient = {
  id: string;
  label: string;
  routingNumber: string;
  accountLast4: string;
};
type Entry = {
  id: string;
  recipientLabel: string;
  amountMinor: string;
  state: string;
  approvalUrl: string | null;
  settlementDate: string | null;
  reasonCode: string | null;
};
const tst = (minor: string) => `TST ${(Number(minor) / 100).toFixed(2)}`;
const stateLabels: Record<string, string> = {
  DRAFT: "Needs your approval",
  QUEUED: "Waiting for the next batch",
  SUBMITTED: "Sent to the rail",
  SETTLED: "Settled",
  RETURNED: "Returned to your account",
  REJECTED: "Rejected by the rail; money released",
  CANCELLED: "Cancelled",
};
export function BatchTransfers({ accounts }: { accounts: Account[] }) {
  const { client } = useSession();
  const [recipients, setRecipients] = useState<Recipient[]>([]);
  const [entries, setEntries] = useState<Entry[]>([]);
  const [label, setLabel] = useState("");
  const [routing, setRouting] = useState("");
  const [number, setNumber] = useState("");
  const [account, setAccount] = useState("");
  const [recipient, setRecipient] = useState("");
  const [amount, setAmount] = useState("");
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const load = useCallback(async () => {
    try {
      const [r, e] = await Promise.all([
        client.GET("/api/v1/batch/recipients").then(value),
        client.GET("/api/v1/batch/payments").then(value),
      ]);
      setRecipients(r as unknown as Recipient[]);
      setEntries(e as unknown as Entry[]);
    } catch (e) {
      setError(message(e));
    }
  }, [client]);
  useEffect(() => {
    void load();
  }, [load]);
  const active = accounts.filter((a) => a.state === "ACTIVE");
  async function addRecipient() {
    const scope = "pb:batch-recipient:" + routing + number;
    try {
      await client
        .POST("/api/v1/batch/recipients", {
          params: { header: { "Idempotency-Key": stableKey(scope) } },
          body: { label, routingNumber: routing, accountNumber: number },
        })
        .then(value);
      doneKey(scope);
      setNotice("Recipient saved.");
      setLabel("");
      setRouting("");
      setNumber("");
      void load();
    } catch (e) {
      setError(message(e));
    }
  }
  async function create() {
    const scope = "pb:batch-entry:" + account + recipient + amount;
    try {
      await client
        .POST("/api/v1/batch/payments", {
          params: { header: { "Idempotency-Key": stableKey(scope) } },
          body: {
            accountId: account,
            recipientId: recipient,
            amountMinor: Math.round(Number(amount) * 100),
          },
        })
        .then(value);
      doneKey(scope);
      setNotice(
        "Transfer prepared. Approve it to hold the money for the next batch.",
      );
      setAmount("");
      void load();
    } catch (e) {
      setError(message(e));
    }
  }
  function approve(entry: Entry) {
    if (
      !entry.approvalUrl ||
      !/^\/auth\/customer\/step-up\/[a-f0-9-]{36}$/.test(entry.approvalUrl)
    )
      return;
    sessionStorage.setItem("pb:return", "/payments");
    window.location.assign(entry.approvalUrl);
  }
  async function act(entry: Entry, action: "confirmation" | "cancellation") {
    try {
      await client
        .POST(`/api/v1/batch/payments/{id}/${action}`, {
          params: { path: { id: entry.id } },
        })
        .then(value);
      setNotice(
        action === "confirmation"
          ? "Approved. The money is held for the next batch."
          : "Transfer cancelled.",
      );
      void load();
    } catch (e) {
      setError(message(e));
    }
  }
  return (
    <section className="section-space" aria-labelledby="batch-heading">
      <h2 id="batch-heading">Next-day transfers</h2>
      <p className="muted">
        Send to an account at another bank on the synthetic batch rail. Approved
        before 16:00 UTC on a business day, it settles the next business day.
        You can cancel until the batch is sent.
      </p>
      {error ? <Alert>{error}</Alert> : null}
      {notice ? <p role="status">{notice}</p> : null}
      <form
        className="card"
        onSubmit={(e) => {
          e.preventDefault();
          void addRecipient();
        }}
      >
        <h3>Add a recipient</h3>
        <label>
          Recipient name
          <input
            value={label}
            onChange={(e) => setLabel(e.target.value)}
            maxLength={80}
            required
          />
        </label>
        <label>
          Routing number
          <input
            value={routing}
            onChange={(e) => setRouting(e.target.value)}
            inputMode="numeric"
            pattern="[0-9]{9}"
            required
          />
        </label>
        <label>
          Account number
          <input
            value={number}
            onChange={(e) => setNumber(e.target.value)}
            inputMode="numeric"
            pattern="[0-9]{4,17}"
            required
          />
        </label>
        <button className="secondary">Save recipient</button>
      </form>
      {recipients.length && active.length ? (
        <form
          className="card"
          onSubmit={(e) => {
            e.preventDefault();
            void create();
          }}
        >
          <h3>Prepare a transfer</h3>
          <label>
            From account
            <select
              value={account}
              onChange={(e) => setAccount(e.target.value)}
              required
            >
              <option value="">Choose an account</option>
              {active.map((a) => (
                <option key={a.id} value={a.id}>
                  Account {short(a.id)}
                </option>
              ))}
            </select>
          </label>
          <label>
            To recipient
            <select
              value={recipient}
              onChange={(e) => setRecipient(e.target.value)}
              required
            >
              <option value="">Choose a recipient</option>
              {recipients.map((r) => (
                <option key={r.id} value={r.id}>
                  {r.label} · account ending {r.accountLast4}
                </option>
              ))}
            </select>
          </label>
          <label>
            Amount (TST)
            <input
              value={amount}
              onChange={(e) => setAmount(e.target.value)}
              inputMode="decimal"
              pattern="[0-9]{1,6}(\.[0-9]{1,2})?"
              required
            />
          </label>
          <button>Prepare transfer</button>
        </form>
      ) : null}
      {entries.length ? (
        <ul className="card-list" aria-label="Next-day transfers">
          {entries.map((e) => (
            <li className="card" key={e.id}>
              <p>
                <strong>{e.recipientLabel}</strong> · {tst(e.amountMinor)} ·{" "}
                {stateLabels[e.state] ?? e.state}
                {e.settlementDate && ["QUEUED", "SUBMITTED"].includes(e.state)
                  ? ` · settles ${e.settlementDate}`
                  : ""}
                {e.reasonCode ? ` · reason ${e.reasonCode}` : ""}
              </p>
              <div className="row">
                {e.state === "DRAFT" ? (
                  <>
                    <button onClick={() => approve(e)}>
                      Confirm identity to approve
                    </button>
                    <button
                      className="secondary"
                      onClick={() => void act(e, "confirmation")}
                    >
                      Complete approval
                    </button>
                  </>
                ) : null}
                {["DRAFT", "QUEUED"].includes(e.state) ? (
                  <button
                    className="secondary"
                    onClick={() => void act(e, "cancellation")}
                  >
                    Cancel transfer
                  </button>
                ) : null}
              </div>
            </li>
          ))}
        </ul>
      ) : null}
    </section>
  );
}
