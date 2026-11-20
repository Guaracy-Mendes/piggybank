import { useCallback, useEffect, useState } from "react";
import { api, tst, uuid, value, type Schema } from "../api";
import { Banner, Busy, Button, Card, Field, Line, Title } from "../ui";

type Balance = Schema["Balance"];
type Txn = { batchId: string; postingKind?: string; kind?: string; businessDate?: string; amount?: { minorUnits: string }; deltaMinor?: string };

/* Accounts, balances (authoritative Core, with freshness) and onboarding parity with the web portal. */
export function Accounts({ refreshKey }: { refreshKey: number }) {
  const [accounts, setAccounts] = useState<Schema["AccountSummary"][]>();
  const [balances, setBalances] = useState<Record<string, Balance>>({});
  const [apps, setApps] = useState<Schema["Application"][]>([]);
  const [open, setOpen] = useState<string>();
  const [txns, setTxns] = useState<Txn[]>();
  const [error, setError] = useState("");
  const load = useCallback(async () => {
    try {
      const [a, p] = await Promise.all([api.GET("/api/v1/accounts").then(value), api.GET("/api/v1/applications").then(value)]);
      setAccounts(a);
      setApps(p as Schema["Application"][]);
      const entries = await Promise.all(a.map(async (x) => [x.id, await api.GET("/api/v1/accounts/{id}/balance", { params: { path: { id: x.id } } }).then(value)] as const));
      setBalances(Object.fromEntries(entries));
      setError("");
    } catch (e) {
      setError((e as Error).message);
    }
  }, []);
  useEffect(() => {
    void load();
  }, [load, refreshKey]);
  async function show(id: string) {
    setOpen(id);
    setTxns(undefined);
    try {
      const day = (offset: number) => new Date(Date.now() + offset * 86400000).toISOString().slice(0, 10);
      const r = await api
        .GET("/api/v1/accounts/{id}/transactions", { params: { path: { id }, query: { from: day(-90), to: day(1), limit: 20 } } })
        .then(value);
      setTxns(r.items as unknown as Txn[]);
    } catch (e) {
      setError((e as Error).message);
    }
  }
  if (!accounts) return error ? <Banner tone="error" text={error} /> : <Busy label="Loading accounts" />;
  const pending = apps.filter((a) => !["ACTIVE", "DECLINED", "EXPIRED"].includes(a.state));
  return (
    <>
      <Title sub="Balances come straight from the bank ledger.">Accounts</Title>
      {error ? <Banner tone="error" text={error} /> : null}
      {accounts.map((a) => {
        const b = balances[a.id];
        return (
          <Card key={a.id} label={"Account ending " + a.id.slice(-4)}>
            <Line strong>Account •••• {a.id.slice(-4)}</Line>
            <Line>Available {tst(b?.available.minorUnits)}</Line>
            <Line muted>
              Booked {tst(b?.booked.minorUnits)} · held {tst(b?.held.minorUnits)}
            </Line>
            <Line muted>{b ? "As of " + new Date(b.asOf).toLocaleTimeString() : "Balance loading"}</Line>
            <Button quiet label={open === a.id ? "Refresh activity" : "Show activity"} onPress={() => void show(a.id)} />
            {open === a.id &&
              (txns === undefined ? (
                <Busy label="Loading activity" />
              ) : txns.length ? (
                txns.slice(0, 20).map((t) => (
                  <Line key={t.batchId}>
                    {(t.businessDate ?? "") + " · " + (t.postingKind ?? t.kind ?? "") + " · " + tst(t.amount?.minorUnits ?? t.deltaMinor)}
                  </Line>
                ))
              ) : (
                <Line muted>No activity yet.</Line>
              ))}
          </Card>
        );
      })}
      {pending.map((a) => (
        <Card key={a.id} label="Application in progress">
          <Line strong>Application in progress</Line>
          <Line>Status: {a.state.replaceAll("_", " ").toLowerCase()}</Line>
          <Line muted>Next: {a.nextAction.replaceAll("_", " ").toLowerCase()}</Line>
        </Card>
      ))}
      {!accounts.length && !pending.length ? <Apply done={load} /> : null}
    </>
  );
}

/* Onboarding: saved, resumable application against the same versioned product and exact terms as web. */
function Apply({ done }: { done: () => Promise<void> }) {
  const [name, setName] = useState("");
  const [email, setEmail] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  async function submit() {
    setBusy(true);
    setError("");
    try {
      const products = await api.GET("/api/v1/products").then(value);
      const product = products[0];
      if (!product) throw new Error("No synthetic product is published.");
      const app = await api.POST("/api/v1/applications", { params: { header: { "Idempotency-Key": uuid() } }, body: { productVersionId: product.id, marketing: false } }).then(value);
      const path = { params: { path: { id: app.id } } };
      await api.PUT("/api/v1/applications/{id}/profile", { ...path, body: { version: app.version, profile: { fullName: name, dateOfBirth: "1990-01-01", email, residency: "SYNTHETIC", consentVersion: "synthetic-consent-v1", marketing: false } } }).then(value);
      await api.POST("/api/v1/applications/{id}/evidence", { ...path, body: { content: "Synthetic identity evidence submitted from the native app." } }).then(value);
      const terms = await api.GET("/api/v1/products/{id}/terms", { params: { path: { id: product.id } } }).then(value);
      await api.POST("/api/v1/applications/{id}/terms", { ...path, body: { termsId: terms.id } }).then(value);
      await api.POST("/api/v1/applications/{id}/submit", { params: { path: { id: app.id }, header: { "Idempotency-Key": uuid() } } }).then(value);
      await done();
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  }
  return (
    <Card label="Open an account">
      <Line strong>Open a synthetic account</Line>
      <Line muted>Use synthetic details only. Submitting accepts the current synthetic terms.</Line>
      {error ? <Banner tone="error" text={error} /> : null}
      <Field label="Full name" value={name} onChangeText={setName} autoComplete="off" />
      <Field label="Email (example.invalid)" value={email} onChangeText={setEmail} autoCapitalize="none" keyboardType="email-address" />
      <Button label="Submit application" disabled={busy || name.length < 2 || !email.endsWith("@example.invalid")} onPress={() => void submit()} />
    </Card>
  );
}
