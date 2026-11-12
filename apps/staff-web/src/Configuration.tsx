import { useCallback, useEffect, useRef, useState } from "react";
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
const amount = (n: string) => ({
  minorUnits: n,
  currency: { code: "TST" as const, scale: 2 as const },
});
export function ConfigurationView({ access }: { access: Access }) {
  const { client } = useSession();
  const [items, setItems] = useState<Schema["ConfigurationRecord"][]>([]);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const id = useRef(crypto.randomUUID());
  const [kind, setKind] = useState<"PRODUCT" | "TERMS">("PRODUCT");
  const maker = access.capabilities.includes("product.propose");
  const checker = access.capabilities.includes("product.approve");
  const load = useCallback(async () => {
    try {
      setItems(
        (await client.GET("/api/v1/staff/configuration").then(value)).items,
      );
      setError("");
    } catch (e) {
      setError(message(e));
    }
  }, [client]);
  useEffect(() => {
    void load();
  }, [load]);
  async function act(work: () => Promise<unknown>, note: string) {
    setPending(true);
    try {
      await work();
      setNotice(note);
      await load();
    } catch (e) {
      setError(message(e));
    } finally {
      setPending(false);
    }
  }
  return (
    <>
      <div className="page-title">
        <div>
          <p className="eyebrow">VERSIONED & APPROVED</p>
          <h1>Terms that stay traceable.</h1>
          <p>
            Publish exact product, pricing and terms versions after independent
            review.
          </p>
        </div>
        <button className="secondary" onClick={() => void load()}>
          Refresh proposals
        </button>
      </div>
      {error && <Alert>{error}</Alert>}
      {notice && (
        <p role="status" className="notice">
          {notice}
        </p>
      )}
      {maker && (
        <section className="panel">
          <h2>Propose a version</h2>
          <form
            className="configuration-form"
            onSubmit={(e) => {
              e.preventDefault();
              const f = new FormData(e.currentTarget);
              const read = (key: string) => String(f.get(key) ?? "");
              const common = {
                id: id.current,
                version: Number(read("version")),
                label: "SYNTHETIC" as const,
                effectiveFrom: read("from"),
                effectiveUntil: read("until"),
              };
              const draft: Schema["ConfigurationDraft"] =
                kind === "PRODUCT"
                  ? {
                      kind,
                      details: {
                        ...common,
                        code: read("code"),
                        name: read("name"),
                        minimumAge: 18,
                        paymentLimit: amount(read("payment")),
                        dailyLimit: amount(read("daily")),
                        flatFee: amount(read("fee")),
                        feeBasisPoints: Number(read("bps")),
                        chartVersion: "synthetic-chart-v1",
                      },
                    }
                  : {
                      kind,
                      details: {
                        ...common,
                        productVersionId: read("product"),
                        title: read("name"),
                        text: read("text"),
                      },
                    };
              void act(async () => {
                await client
                  .POST("/api/v1/staff/configuration", { body: draft })
                  .then(value);
                id.current = crypto.randomUUID();
              }, "Immutable draft saved for independent review.");
            }}
          >
            <label>
              Version kind
              <select
                value={kind}
                onChange={(e) => setKind(e.target.value as typeof kind)}
              >
                <option value="PRODUCT">Product & pricing</option>
                <option value="TERMS">Exact terms</option>
              </select>
            </label>
            <label>
              Name or title
              <input name="name" required maxLength={100} />
            </label>
            <label>
              Version
              <input
                name="version"
                type="number"
                min="1"
                max="2147483647"
                defaultValue="1"
                required
              />
            </label>
            <label>
              Effective from
              <input
                name="from"
                type="date"
                defaultValue="2026-01-01"
                required
              />
            </label>
            <label>
              Effective until
              <input
                name="until"
                type="date"
                defaultValue="2030-01-01"
                required
              />
            </label>
            {kind === "PRODUCT" ? (
              <>
                <label>
                  Product code
                  <input
                    name="code"
                    required
                    pattern="[A-Z][A-Z0-9_]{0,31}"
                    maxLength={32}
                  />
                </label>
                <label>
                  Payment limit · TST minor units
                  <input
                    name="payment"
                    required
                    inputMode="numeric"
                    pattern="[1-9][0-9]{0,18}"
                    defaultValue="1000000"
                  />
                </label>
                <label>
                  Daily limit · TST minor units
                  <input
                    name="daily"
                    required
                    inputMode="numeric"
                    pattern="[1-9][0-9]{0,18}"
                    defaultValue="5000000"
                  />
                </label>
                <label>
                  Flat fee · TST minor units
                  <input
                    name="fee"
                    required
                    inputMode="numeric"
                    pattern="0|[1-9][0-9]{0,18}"
                    defaultValue="0"
                  />
                </label>
                <label>
                  Variable fee · basis points
                  <input
                    name="bps"
                    type="number"
                    min="0"
                    max="1000"
                    defaultValue="0"
                    required
                  />
                </label>
              </>
            ) : (
              <>
                <label>
                  Published product version ID
                  <input name="product" required pattern="[a-fA-F0-9-]{36}" />
                </label>
                <label className="wide">
                  Exact terms text
                  <textarea name="text" required maxLength={32768} rows={5} />
                </label>
              </>
            )}
            <div className="wide">
              <p>
                Currency TST · Minimum age 18 · Synthetic chart v1. These values
                are part of the immutable proposal.
              </p>
              <button disabled={pending}>Save configuration proposal</button>
            </div>
          </form>
        </section>
      )}
      <section className="proposal-grid" aria-label="Configuration register">
        {items.length === 0 && (
          <div className="panel">
            <h2>No configuration drafts yet.</h2>
            <p>
              Published fixture versions remain available to customer journeys.
            </p>
          </div>
        )}
        {items.map((p) => (
          <article className="panel" key={p.proposalId}>
            <div className="panel-title">
              <h2>{"name" in p.details ? p.details.name : p.details.title}</h2>
              <Badge tone={p.publishedAt ? "good" : "attention"}>
                {p.publishedAt ? "PUBLISHED" : p.state}
              </Badge>
            </div>
            <p>
              {p.kind} · Version {p.details.version} · Maker{" "}
              {short(p.makerPerson)}
            </p>
            <p>
              {p.details.effectiveFrom} → {p.details.effectiveUntil}
            </p>
            {"flatFee" in p.details ? (
              <dl className="facts">
                <div>
                  <dt>Per payment</dt>
                  <dd>{money(p.details.paymentLimit)}</dd>
                </div>
                <div>
                  <dt>Daily limit</dt>
                  <dd>{money(p.details.dailyLimit)}</dd>
                </div>
                <div>
                  <dt>Fee</dt>
                  <dd>
                    {money(p.details.flatFee)} + {p.details.feeBasisPoints} bps
                  </dd>
                </div>
              </dl>
            ) : (
              <details>
                <summary>Read exact proposed terms</summary>
                <pre>{p.details.text}</pre>
              </details>
            )}
            <p>
              <small>
                Proposal {p.proposalId}
                <br />
                Expires {time(p.expiresAt)}
              </small>
            </p>
            {checker &&
              p.makerPerson !== access.personId &&
              p.state === "PENDING" && (
                <button
                  disabled={pending}
                  onClick={() =>
                    void act(
                      () =>
                        client
                          .POST("/api/v1/staff/configuration/{id}/approval", {
                            params: { path: { id: p.proposalId } },
                          })
                          .then(value),
                      "Independent configuration approval recorded.",
                    )
                  }
                >
                  Approve configuration
                </button>
              )}
            {maker && p.state === "APPROVED" && !p.publishedAt && (
              <button
                disabled={pending}
                onClick={() =>
                  void act(
                    () =>
                      client
                        .POST("/api/v1/staff/configuration/{id}/publication", {
                          params: { path: { id: p.proposalId } },
                        })
                        .then(value),
                    "Approved exact version published.",
                  )
                }
              >
                Publish approved version
              </button>
            )}
          </article>
        ))}
      </section>
    </>
  );
}
