import { useCallback, useEffect, useState } from "react";
import { api, tst, uuid, value, type Schema } from "../api";
import { reauthenticate } from "../auth";
import { clearPending, loadPending, savePending, type Pending } from "../pending";
import { Banner, Busy, Button, Card, Choice, Field, Line, Title } from "../ui";

type Payment = Schema["PaymentStatus"];
const final = ["SETTLED", "DECLINED", "EXPIRED", "BLOCKED", "CANCELLED", "RETURNED"];

/* Payments: persisted intent, native action-bound approval, truthful status. Offline disables commands; nothing is queued. */
export function Payments({ online, refreshKey }: { online: boolean; refreshKey: number }) {
  const [payments, setPayments] = useState<Payment[]>();
  const [accounts, setAccounts] = useState<Schema["AccountSummary"][]>([]);
  const [recipients, setRecipients] = useState<Schema["Beneficiary"][]>([]);
  const [pending, setPending] = useState<Pending>();
  const [current, setCurrent] = useState<Payment>();
  const [from, setFrom] = useState<string>();
  const [to, setTo] = useState<string>();
  const [amount, setAmount] = useState("");
  const [reference, setReference] = useState("");
  const [label, setLabel] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const load = useCallback(async () => {
    try {
      const [p, a, b, saved] = await Promise.all([
        api.GET("/api/v1/payments").then(value),
        api.GET("/api/v1/accounts").then(value),
        api.GET("/api/v1/beneficiaries").then(value),
        loadPending(),
      ]);
      setPayments(p);
      setAccounts(a.filter((x) => x.state === "ACTIVE" && !x.debitBlocked));
      setRecipients(b.filter((x) => x.state === "VERIFIED"));
      setPending(saved);
      // Recover an interrupted request from the persisted operation, never by creating another payment.
      if (saved && !saved.paymentId && online) {
        const r = await api.POST("/api/v1/payments", { params: { header: { "Idempotency-Key": saved.key } }, body: { quoteId: saved.quoteId } }).then(value);
        saved.paymentId = r.id;
        await savePending(saved);
      }
      if (saved?.paymentId) {
        const r = await api.GET("/api/v1/payments/{id}", { params: { path: { id: saved.paymentId } } }).then(value);
        setCurrent(r);
        if (final.includes(r.state)) await clearPending();
      }
      setError("");
    } catch (e) {
      setError((e as Error).message);
    }
  }, [online]);
  useEffect(() => {
    void load();
  }, [load, refreshKey]);
  async function run(fn: () => Promise<string | void>) {
    setBusy(true);
    setError("");
    setNotice("");
    try {
      const done = await fn();
      if (done) setNotice(done);
      await load();
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  }
  const create = () =>
    run(async () => {
      const minor = Math.round(Number(amount) * 100);
      if (!from || !to || !Number.isFinite(minor) || minor <= 0) throw new Error("Choose an account, a recipient and an amount.");
      const quote = await api
        .POST("/api/v1/payment-quotes", {
          body: { payerAccountId: from, beneficiaryId: to, amount: { minorUnits: String(minor), currency: { code: "TST", scale: 2 } }, requestedDate: new Date().toISOString().slice(0, 10) },
        })
        .then(value);
      const p: Pending = { key: uuid(), quoteId: quote.id, createdAt: new Date().toISOString() };
      await savePending(p); // persisted before the request is sent
      const r = await api.POST("/api/v1/payments", { params: { header: { "Idempotency-Key": p.key } }, body: { quoteId: quote.id } }).then(value);
      await savePending({ ...p, paymentId: r.id });
      setAmount("");
      return "Payment saved. Approve it with your password to send.";
    });
  const approve = (p: Payment) =>
    run(async () => {
      const started = await api.POST("/api/v1/payments/{id}/approval", { params: { path: { id: p.id } } }).then(value);
      if (!started.challengeId) throw new Error("Approval is not available for this payment.");
      const begin = await api.POST("/api/v1/approvals/{challengeId}/native-step-up", { params: { path: { challengeId: started.challengeId } } }).then(value);
      const idToken = await reauthenticate(begin.nonce);
      await api.POST("/api/v1/approvals/{challengeId}/native-proof", { params: { path: { challengeId: started.challengeId } }, body: { idToken } }).then(value);
      const confirmed = await api.POST("/api/v1/payments/{id}/confirmation", { params: { path: { id: p.id } }, body: { quoteId: p.quoteId } }).then(value);
      return "Payment submitted. Status: " + confirmed.state.replaceAll("_", " ").toLowerCase() + ".";
    });
  const addRecipient = () =>
    run(async () => {
      await api.POST("/api/v1/beneficiaries", { params: { header: { "Idempotency-Key": uuid() } }, body: { label, receivingReference: reference.trim().toUpperCase() } }).then(value);
      setLabel("");
      setReference("");
      return "Recipient saved and verified.";
    });
  if (!payments) return error ? <Banner tone="error" text={error} /> : <Busy label="Loading payments" />;
  return (
    <>
      <Title sub="Every payment needs your password again before it is sent.">Payments</Title>
      {!online ? <Banner tone="error" text="You're offline. Payments are paused and nothing will be sent later automatically." /> : null}
      {error ? <Banner tone="error" text={error} /> : null}
      {notice ? <Banner text={notice} /> : null}
      {current && !final.includes(current.state) ? (
        <Card label="Payment in progress">
          <Line strong>{current.beneficiaryLabel}</Line>
          <Line>{tst(current.total.minorUnits)}</Line>
          <Line>Status: {current.state.replaceAll("_", " ").toLowerCase()}</Line>
          <Line muted>Reference {current.id.slice(0, 8)}</Line>
          {current.state === "DRAFT" ? <Button label="Approve and send" disabled={!online || busy} onPress={() => void approve(current)} /> : null}
          <Button quiet label="Refresh status" disabled={busy} onPress={() => void load()} />
        </Card>
      ) : null}
      {pending && !pending.paymentId ? <Banner text="A payment request was interrupted. Reconnect to check its status." /> : null}
      <Card label="New payment">
        <Line strong>New payment</Line>
        {accounts.length && recipients.length ? (
          <>
            <Choice label="From account" options={accounts.map((a) => ({ id: a.id, text: "Account •••• " + a.id.slice(-4) }))} value={from} onChange={setFrom} />
            <Choice label="To recipient" options={recipients.map((b) => ({ id: b.id, text: b.label }))} value={to} onChange={setTo} />
            <Field label="Amount (TST)" value={amount} onChangeText={setAmount} keyboardType="decimal-pad" />
            <Button label="Review payment" disabled={!online || busy || !!(current && !final.includes(current.state))} onPress={() => void create()} />
          </>
        ) : (
          <Line muted>You need an active account and a verified recipient.</Line>
        )}
      </Card>
      <Card label="Add a recipient">
        <Line strong>Add a recipient</Line>
        <Field label="Recipient name" value={label} onChangeText={setLabel} />
        <Field label="Receiving reference" value={reference} onChangeText={setReference} autoCapitalize="characters" />
        <Button quiet label="Save recipient" disabled={!online || busy || !label || reference.length < 10} onPress={() => void addRecipient()} />
      </Card>
      <Card label="Recent payments">
        <Line strong>Recent payments</Line>
        {payments.slice(0, 10).map((p) => (
          <Line key={p.id}>
            {p.beneficiaryLabel + " · " + tst(p.total.minorUnits) + " · " + p.state.replaceAll("_", " ").toLowerCase()}
          </Line>
        ))}
        {!payments.length ? <Line muted>No payments yet.</Line> : null}
      </Card>
    </>
  );
}
