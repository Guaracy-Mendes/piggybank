import { useCallback, useEffect, useRef, useState } from "react";
import {
  Alert,
  Badge,
  message,
  useSession,
  value,
  type Schema,
} from "@piggybank/web-core";
import type { Access } from "./App";
type Offer = Schema["PeriodicOfferRequest"];
type Activation = Schema["PeriodicActivationRequest"];
async function key(scope: string, body: unknown, consumed = false) {
  const hash = Array.from(
    new Uint8Array(
      await crypto.subtle.digest(
        "SHA-256",
        new TextEncoder().encode(JSON.stringify(body)),
      ),
    ),
    (b) => b.toString(16).padStart(2, "0"),
  ).join("");
  const name = "pb:finance-periodic:" + scope + ":" + hash;
  let k = sessionStorage.getItem(name);
  if (!k) {
    k = crypto.randomUUID();
    sessionStorage.setItem(name, k);
  }
  if (consumed) sessionStorage.removeItem(name);
  return k;
}
export function FinancePeriodic({
  access,
  kind,
  selected,
}: {
  access: Access;
  kind?: string;
  selected?: string;
}) {
  const { client } = useSession();
  const generation = useRef(0);
  const scope = useRef("");
  scope.current = (kind ?? "") + ":" + (selected ?? "");
  const [offers, setOffers] = useState<Offer[]>([]),
    [activations, setActivations] = useState<Activation[]>([]),
    [runs, setRuns] = useState<Schema["PeriodicRun"][]>([]);
  const [offer, setOffer] = useState<Offer>(),
    [activation, setActivation] = useState<Activation>();
  const [error, setError] = useState(""),
    [notice, setNotice] = useState(""),
    [busy, setBusy] = useState(false),
    [reviewed, setReviewed] = useState("");
  const load = useCallback(async () => {
    const g = ++generation.current,
      current = (kind ?? "") + ":" + (selected ?? "");
    try {
      const [o, a, r, od, ad] = await Promise.all([
        client.GET("/api/v1/staff/finance/periodic-offers").then(value),
        client.GET("/api/v1/staff/finance/periodic-activations").then(value),
        client.GET("/api/v1/staff/finance/periodic-runs").then(value),
        selected && kind === "offers"
          ? client
              .GET("/api/v1/staff/finance/periodic-offers/{id}", {
                params: { path: { id: selected } },
              })
              .then(value)
          : undefined,
        selected && kind === "activations"
          ? client
              .GET("/api/v1/staff/finance/periodic-activations/{id}", {
                params: { path: { id: selected } },
              })
              .then(value)
          : undefined,
      ]);
      if (g !== generation.current || current !== scope.current) return;
      setOffers(o);
      setActivations(a);
      setRuns(r);
      setOffer(od);
      setActivation(ad);
      setError("");
    } catch (e) {
      if (g === generation.current) {
        setOffer(undefined);
        setActivation(undefined);
        setError(message(e));
      }
    }
  }, [client, kind, selected]);
  useEffect(() => {
    setOffer(undefined);
    setActivation(undefined);
    setReviewed("");
    void load();
    return () => {
      generation.current++;
    };
  }, [load]);
  async function perform(work: () => Promise<unknown>, result: string) {
    setBusy(true);
    setError("");
    setNotice("");
    try {
      await work();
      setReviewed("");
      await load();
      setNotice(result);
    } catch (e) {
      setError(message(e));
    } finally {
      setBusy(false);
    }
  }
  async function stage(
    form: HTMLFormElement,
    type: "offer" | "activation" | "run",
  ) {
    const f = new FormData(form),
      get = (n: string) => String(f.get(n) ?? "");
    await perform(async () => {
      if (type === "offer") {
        const body = { policyId: get("policy"), reason: get("reason") };
        const r = await client
          .POST("/api/v1/staff/finance/periodic-offers", {
            params: {
              header: {
                "Idempotency-Key": await key(access.personId + ":offer", body),
              },
            },
            body,
          })
          .then(value);
        location.hash = "/finance/periodic/offers/" + r.id;
      } else if (type === "activation") {
        const body = {
          consentId: get("consent"),
          counterAccountId: get("counter"),
        };
        const r = await client
          .POST("/api/v1/staff/finance/periodic-activations", {
            params: {
              header: {
                "Idempotency-Key": await key(
                  access.personId + ":activation",
                  body,
                ),
              },
            },
            body,
          })
          .then(value);
        location.hash = "/finance/periodic/activations/" + r.id;
      } else {
        const body = {
          activationId: get("activation"),
          periodStart: get("period"),
        };
        await client
          .POST("/api/v1/staff/finance/periodic-runs", {
            params: {
              header: {
                "Idempotency-Key": await key(access.personId + ":run", body),
              },
            },
            body,
          })
          .then(value);
        await key(access.personId + ":run", body, true);
      }
    }, "Request retained. Refresh to inspect its current native receipt.");
  }
  async function offerAction(action: "approval" | "publication") {
    if (!offer) return;
    const options = {
      params: { path: { id: offer.id } },
      body: { detailsDigest: offer.digest },
    };
    await perform(
      () =>
        action === "approval"
          ? client
              .POST(
                "/api/v1/staff/finance/periodic-offers/{id}/approval",
                options,
              )
              .then(value)
          : client
              .POST(
                "/api/v1/staff/finance/periodic-offers/{id}/publication",
                options,
              )
              .then(value),
      "Financial terms " + action + " recorded.",
    );
  }
  async function activationAction(action: "approval" | "execution") {
    if (!activation) return;
    const options = {
      params: { path: { id: activation.id } },
      body: { detailsDigest: activation.digest },
    };
    await perform(
      () =>
        action === "approval"
          ? client
              .POST(
                "/api/v1/staff/finance/periodic-activations/{id}/approval",
                options,
              )
              .then(value)
          : client
              .POST(
                "/api/v1/staff/finance/periodic-activations/{id}/execution",
                options,
              )
              .then(value),
      "Activation " + action + " recorded.",
    );
  }
  const maker = access.capabilities.includes("finance.recover"),
    checker = access.capabilities.includes("finance.approve");
  return (
    <div className="periodic-finance">
      <div className="page-title">
        <div>
          <p className="eyebrow">PERIODIC FINANCE</p>
          <h1>Enroll, activate, post.</h1>
          <p>
            Explicit financial terms and independent activation for synthetic
            TST accounts.
          </p>
        </div>
        <button
          className="secondary"
          disabled={busy}
          onClick={() => void load()}
        >
          Refresh periodic finance
        </button>
      </div>
      {error && <Alert>{error}</Alert>}
      {notice && (
        <p role="status" className="notice">
          {notice}
        </p>
      )}
      <div className="queue-layout">
        <section className="panel">
          <h2>Financial offers</h2>
          {!offers.length && (
            <p>No financial offers yet. Publish a calculation policy first.</p>
          )}
          {offers.map((o) => (
            <a
              className="case-row"
              key={o.id}
              href={"#/finance/periodic/offers/" + o.id}
            >
              {o.material.policy.code} · {o.id} <Badge>{o.state}</Badge>
            </a>
          ))}
        </section>
        <section className="panel">
          <h2>Account activations</h2>
          {!activations.length && <p>No activation requests.</p>}
          {activations.map((a) => (
            <a
              className="case-row"
              key={a.id}
              href={"#/finance/periodic/activations/" + a.id}
            >
              {a.id} <Badge>{a.state}</Badge>
            </a>
          ))}
        </section>
      </div>
      {offer && (
        <section className="panel" aria-label="Financial offer review">
          <h2>Review financial terms</h2>
          <Badge>{offer.state}</Badge>
          <h3>{offer.material.title}</h3>
          <p className="disclosure-text">{offer.material.text}</p>
          <dl className="evidence-details">
            <dt>Offer reference</dt>
            <dd>{offer.id}</dd>
            <dt>Terms digest</dt>
            <dd>{offer.digest}</dd>
            <dt>Maker</dt>
            <dd>{offer.makerPerson}</dd>
            <dt>Checker</dt>
            <dd>{offer.checkerPerson ?? "Pending"}</dd>
            <dt>Reason</dt>
            <dd>{offer.reason}</dd>
          </dl>
          <details>
            <summary>Exact financial rule and calendar</summary>
            <pre>{JSON.stringify(offer.material, null, 2)}</pre>
          </details>
          {checker &&
            offer.makerPerson !== access.personId &&
            ["STAGED", "PENDING", "APPROVED"].includes(offer.state) && (
              <>
                <label>
                  <input
                    type="checkbox"
                    checked={reviewed === offer.digest}
                    onChange={(e) =>
                      setReviewed(e.target.checked ? offer.digest : "")
                    }
                  />
                  I reviewed these financial terms, policy, calendar and digest.
                </label>
                <button
                  disabled={busy || reviewed !== offer.digest}
                  onClick={() => void offerAction("approval")}
                >
                  Approve financial terms
                </button>
              </>
            )}
          {maker &&
            offer.makerPerson === access.personId &&
            offer.state === "APPROVED" && (
              <button
                disabled={busy}
                onClick={() => void offerAction("publication")}
              >
                Publish financial offer
              </button>
            )}
        </section>
      )}
      {activation && (
        <section className="panel" aria-label="Periodic activation review">
          <h2>Review account activation</h2>
          <details open>
            <summary>Accepted financial terms and account</summary>
            <p>Account: {activation.enrollment.consent.accountId}</p>
            <p>{activation.enrollment.consent.offer.material.text}</p>
            <p>{activation.enrollment.consent.statement}</p>
            <p>Accepted at: {activation.enrollment.consent.acceptedAt}</p>
          </details>
          <Badge>{activation.state}</Badge>
          <dl className="evidence-details">
            <dt>Consent reference</dt>
            <dd>{activation.details.consentId}</dd>
            <dt>Consent digest</dt>
            <dd>{activation.details.consentDigest}</dd>
            <dt>Counter account</dt>
            <dd>{activation.details.counterAccountId}</dd>
            <dt>Activation digest</dt>
            <dd>{activation.digest}</dd>
            <dt>First whole period</dt>
            <dd>
              {activation.receipt?.activation.firstPeriodStart ??
                "Determined at execution"}
            </dd>
          </dl>
          {checker &&
            activation.makerPerson !== access.personId &&
            ["STAGED", "PENDING", "APPROVED"].includes(activation.state) && (
              <>
                <label>
                  <input
                    type="checkbox"
                    checked={reviewed === activation.digest}
                    onChange={(e) =>
                      setReviewed(e.target.checked ? activation.digest : "")
                    }
                  />
                  I reviewed the consent digest, counter account and activation
                  digest.
                </label>
                <button
                  disabled={busy || reviewed !== activation.digest}
                  onClick={() => void activationAction("approval")}
                >
                  Approve account activation
                </button>
              </>
            )}
          {maker &&
            activation.makerPerson === access.personId &&
            activation.state === "APPROVED" && (
              <button
                disabled={busy}
                onClick={() => void activationAction("execution")}
              >
                Activate periodic finance
              </button>
            )}
          {maker && activation.state === "ACTIVE" && (
            <form
              onSubmit={(e) => {
                e.preventDefault();
                const reason = String(
                  new FormData(e.currentTarget).get("reason"),
                );
                void perform(
                  () =>
                    client
                      .POST(
                        "/api/v1/staff/finance/periodic-activations/{id}/stop",
                        {
                          params: { path: { id: activation.id } },
                          body: { reason },
                        },
                      )
                      .then(value),
                  "Future periodic effects stopped.",
                );
              }}
            >
              <label>
                Reason to stop future effects
                <input name="reason" minLength={4} maxLength={240} required />
              </label>
              <button className="secondary" disabled={busy}>
                Stop periodic finance
              </button>
              <p>
                A stopped activation cannot resume. Original evidence and
                corrections are retained.
              </p>
            </form>
          )}
        </section>
      )}
      <section className="panel">
        <h2>Period runs</h2>
        <p>
          Only a posted Core receipt proves completion. Zero accruals have a
          completed receipt without a journal. Failed attempts retain an owned
          Finance case.
        </p>
        {!runs.length && <p>No due periods have been queued.</p>}
        {runs.map((r) => (
          <article className="case-row" key={r.id}>
            <strong>
              {r.periodStart} · {r.activationId}
            </strong>
            <Badge>
              {r.receipt?.posted
                ? "POSTED"
                : r.receipt
                  ? "FINALIZED"
                  : "QUEUED"}
            </Badge>
            <p>
              Amount minor units:{" "}
              {r.receipt?.amountMinor ?? "Awaiting native calculation"} · Batch:{" "}
              {r.receipt?.batchId ?? "None"} · {r.lastError}
            </p>
            <details>
              <summary>Period evidence {r.id}</summary>
              <pre>{JSON.stringify(r.receipt, null, 2)}</pre>
            </details>
            {r.receipt?.batchId && (
              <a href="#/finance/corrections">
                Request an independently approved correction
              </a>
            )}
          </article>
        ))}
      </section>
      {maker && (
        <section className="panel">
          <h2>Prepare financial enrollment</h2>
          <form
            className="configuration-form"
            onSubmit={(e) => {
              e.preventDefault();
              void stage(e.currentTarget, "offer");
            }}
          >
            <label htmlFor="offer-policy">Published policy ID</label>
            <input
              id="offer-policy"
              name="policy"
              required
              pattern="[a-f0-9-]{36}"
            />
            <label htmlFor="offer-reason">Financial terms reason</label>
            <input
              id="offer-reason"
              name="reason"
              required
              minLength={4}
              maxLength={240}
            />
            <button disabled={busy}>Stage financial terms</button>
          </form>
          <h2>Prepare account activation</h2>
          <form
            className="configuration-form"
            onSubmit={(e) => {
              e.preventDefault();
              void stage(e.currentTarget, "activation");
            }}
          >
            <label htmlFor="activation-consent">Customer consent ID</label>
            <input
              id="activation-consent"
              name="consent"
              required
              pattern="[a-f0-9-]{36}"
            />
            <label htmlFor="activation-counter">
              Explicit counter account ID
            </label>
            <input
              id="activation-counter"
              name="counter"
              required
              pattern="[a-f0-9-]{36}"
            />
            <button disabled={busy}>Stage account activation</button>
          </form>
          <p>
            The current approved chart must classify the explicit counter as fee
            revenue or interest expense.
          </p>
          <h2>Queue a closed period</h2>
          <p>
            Due periods are discovered automatically while the original maker
            retains current authority. This command also resumes a retained
            closed period after resolving its exception.
          </p>
          <form
            className="configuration-form"
            onSubmit={(e) => {
              e.preventDefault();
              void stage(e.currentTarget, "run");
            }}
          >
            <label htmlFor="run-activation">Activation ID</label>
            <input
              id="run-activation"
              name="activation"
              required
              pattern="[a-f0-9-]{36}"
            />
            <label htmlFor="run-period">Closed period start</label>
            <input id="run-period" name="period" required type="date" />
            <button disabled={busy}>Queue due period</button>
          </form>
        </section>
      )}
    </div>
  );
}
