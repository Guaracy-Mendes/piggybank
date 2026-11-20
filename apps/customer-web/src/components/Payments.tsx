import { useCallback, useEffect, useRef, useState } from "react";
import {
  Alert,
  Badge,
  Busy,
  message,
  money,
  short,
  time,
  useSession,
  value,
  type Schema,
} from "@piggybank/web-core";
import { Credits } from "./Credits";
import type { Account } from "../types";
import { navigate, stableKey, doneKey } from "../lib/navigation";
import { PageTitle } from "../lib/display";
import { QrCode } from "./QrCode";
import { amount } from "../consumer/model";
import { encodePaymentCode, parsePaymentCode } from "../lib/paymentCode";
type Payment = Schema["PaymentStatus"];
const labels: Record<Payment["state"], string> = {
  DRAFT: "Approval needed",
  ACCEPTED: "Accepted for checks",
  CONTROLS_PENDING: "Checks in progress",
  CONTROL_REVIEW: "With our team",
  EXPIRED: "Expired",
  DECLINED: "Declined",
  CANCELLED: "Cancelled",
  CANCELLATION_PENDING: "Cancellation requested · pending",
  BLOCKED: "Instruction blocked",
  SETTLED: "Completed",
  RESERVED: "Funds reserved",
  SUBMITTING: "Sending to provider",
  SUBMITTED: "Provider acknowledged · pending",
  OUTCOME_UNKNOWN: "Outcome pending",
  RAIL_PAUSED: "Submissions paused",
  RECONCILING: "Checking final evidence",
};
export function PaymentBadge({
  state,
  returns,
}: {
  state: Payment["state"];
  returns?: Schema["ReturnSummary"];
}) {
  const returned = returns?.status;
  const label =
    returned && returned !== "NONE"
      ? (
          {
            RETURN_RECEIVED: "Return received · pending",
            PARTIALLY_RETURNED: "Partially returned",
            RETURNED: "Returned",
          } as const
        )[returned]
      : labels[state];
  return (
    <Badge
      tone={
        state === "SETTLED"
          ? "good"
          : ["DECLINED", "BLOCKED", "EXPIRED"].includes(state)
            ? "danger"
            : "attention"
      }
    >
      {label}
    </Badge>
  );
}
async function inputScope(value: unknown) {
  const hash = await crypto.subtle.digest(
    "SHA-256",
    new TextEncoder().encode(JSON.stringify(value)),
  );
  return Array.from(new Uint8Array(hash), (b) =>
    b.toString(16).padStart(2, "0"),
  ).join("");
}
export function PaymentsView({
  accounts,
  selected,
}: {
  accounts: Account[];
  selected?: string;
}) {
  const { client } = useSession();
  const [payments, setPayments] = useState<Payment[]>([]);
  const [beneficiaries, setBeneficiaries] = useState<Schema["Beneficiary"][]>(
    [],
  );
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [pending, setPending] = useState(false);
  const referenceInput = useRef<HTMLInputElement>(null);
  const [codeInfo, setCodeInfo] = useState("");
  const [savedQuote, setSavedQuote] = useState<Schema["PaymentQuote"]>();
  const load = useCallback(async () => {
    try {
      const [p, b] = await Promise.all([
        client.GET("/api/v1/payments").then(value),
        client.GET("/api/v1/beneficiaries").then(value),
      ]);
      setPayments(p);
      setBeneficiaries(b);
      setError("");
    } catch (e) {
      setError(message(e));
    }
  }, [client]);
  useEffect(() => {
    void load();
  }, [load]);
  async function saveIntent(quote: Schema["PaymentQuote"]) {
    const scope = "pb:payment:" + quote.id;
    const result = await client
      .POST("/api/v1/payments", {
        params: { header: { "Idempotency-Key": stableKey(scope) } },
        body: { quoteId: quote.id },
      })
      .then(value);
    doneKey(scope);
    navigate("/payments/" + result.id);
    await load();
  }
  const active = accounts.filter(
    (a) => a.state === "ACTIVE" && !a.debitBlocked,
  );
  if (selected) return <PaymentDetail id={selected} />;
  return (
    <>
      <PageTitle
        eyebrow="CLEAR BEFORE YOU CONFIRM"
        title="Move money, with a clear view."
        description="Review the recipient, exact fee and total before you approve a TST payment."
      />
      {error && <Alert>{error}</Alert>}
      {notice && (
        <p className="notice" role="status">
          {notice}
        </p>
      )}
      <div className="payment-workspace">
        <section className="card payment-card">
          <h2>Make a payment</h2>
          {!active.length ? (
            <p>
              No account is currently available for outgoing payments. Review
              its status in <a href="#/accounts">Accounts</a>.
            </p>
          ) : !beneficiaries.some((b) => b.state === "VERIFIED") ? (
            <p>
              Add a recipient using a receiving reference shared by another
              synthetic account holder.
            </p>
          ) : (
            <form
              className="field-grid"
              onChange={() => setSavedQuote(undefined)}
              onSubmit={(e) => {
                e.preventDefault();
                const f = new FormData(e.currentTarget);
                setPending(true);
                setError("");
                void (async () => {
                  const quote = await client
                    .POST("/api/v1/payment-quotes", {
                      body: {
                        payerAccountId: String(f.get("payer")),
                        beneficiaryId: String(f.get("beneficiary")),
                        amount: {
                          minorUnits: String(f.get("amount")),
                          currency: { code: "TST", scale: 2 },
                        },
                        requestedDate: new Date().toISOString().slice(0, 10),
                      },
                    })
                    .then(value);
                  setSavedQuote(quote);
                  await saveIntent(quote);
                })()
                  .catch((e) => setError(message(e)))
                  .finally(() => setPending(false));
              }}
            >
              <label>
                From account
                <select name="payer" required>
                  {active.map((a) => (
                    <option key={a.id} value={a.id}>
                      Everyday account · {short(a.id)}
                    </option>
                  ))}
                </select>
              </label>
              <label>
                Recipient
                <select name="beneficiary" required>
                  {beneficiaries
                    .filter((b) => b.state === "VERIFIED")
                    .map((b) => (
                      <option key={b.id} value={b.id}>
                        {b.label} · {b.receivingReference.slice(-8)}
                      </option>
                    ))}
                </select>
              </label>
              <label>
                Amount · TST minor units
                <input
                  name="amount"
                  inputMode="numeric"
                  pattern="[1-9][0-9]{0,18}"
                  required
                  placeholder="10000"
                />
                <small>
                  100 minor units = TST 1.00. Exact whole minor units only.
                </small>
              </label>
              <div className="wide">
                <p className="muted">
                  Today's UTC business date · The recipient determines the rail.
                  Your exact fee and total appear on the review screen.
                </p>
                <button disabled={pending}>Review payment</button>
                {savedQuote && (
                  <button
                    className="secondary"
                    type="button"
                    disabled={pending}
                    onClick={() => {
                      setPending(true);
                      void saveIntent(savedQuote)
                        .catch((e) => setError(message(e)))
                        .finally(() => setPending(false));
                    }}
                  >
                    Resume saved quote
                  </button>
                )}
              </div>
            </form>
          )}
        </section>
        <Receiving accounts={accounts} />
      </div>
      <section className="card payment-card">
        <h2>Your recipients</h2>
        <p className="muted">
          References are shared by the receiving customer. Verification checks
          the selected destination. The simulated rail uses the fixture
          reference SIM00000000000000000000000000000001.
        </p>
        <form
          className="field-grid"
          onSubmit={(e) => {
            e.preventDefault();
            const f = new FormData(e.currentTarget);
            const body = {
              label: String(f.get("label")),
              rail: String(f.get("rail")) as "INTERNAL" | "SIMULATED",
              receivingReference: String(f.get("reference"))
                .trim()
                .toUpperCase(),
            };
            setPending(true);
            setError("");
            void (async () => {
              const scope = "pb:recipient:" + (await inputScope(body));
              await client
                .POST("/api/v1/beneficiaries", {
                  params: { header: { "Idempotency-Key": stableKey(scope) } },
                  body,
                })
                .then(value);
              doneKey(scope);
              setNotice("Recipient saved and verified.");
              await load();
            })()
              .catch((e) => setError(message(e)))
              .finally(() => setPending(false));
          }}
        >
          <label>
            Recipient label
            <input
              name="label"
              required
              maxLength={80}
              placeholder="Synthetic recipient"
            />
          </label>
          <label>
            Payment rail
            <select name="rail">
              <option value="INTERNAL">Internal · Same ledger</option>
              <option value="SIMULATED">Simulated external rail</option>
            </select>
          </label>
          <label className="wide">
            Payment code (text from a scanned QR, optional)
            <span className="code-row">
              <input
                name="code"
                maxLength={400}
                placeholder="pbtst:pay?ref=PB…"
              />
              <button
                type="button"
                className="secondary"
                onClick={(e) => {
                  const input = e.currentTarget.form?.elements.namedItem(
                    "code",
                  ) as HTMLInputElement | null;
                  const code = parsePaymentCode(input?.value ?? "");
                  if (!code || !referenceInput.current) {
                    setCodeInfo("");
                    setError("That is not a PiggyBank TST payment code.");
                    return;
                  }
                  referenceInput.current.value = code.reference;
                  setError("");
                  setCodeInfo(
                    `Reference ···· ${code.reference.slice(-8)} filled in` +
                      (code.amountMinor
                        ? ` · requested ${amount(code.amountMinor)}`
                        : "") +
                      (code.memo ? ` · memo “${code.memo}”` : "") +
                      ". Name the recipient and add them; nothing is paid yet.",
                  );
                }}
              >
                Use payment code
              </button>
            </span>
          </label>
          {codeInfo && (
            <p className="wide muted" role="status" data-testid="code-info">
              {codeInfo}
            </p>
          )}
          <label>
            Shared receiving reference
            <input
              ref={referenceInput}
              name="reference"
              required
              pattern="(PB|SIM)[A-Fa-f0-9]{32}"
              maxLength={35}
              placeholder="PB…"
            />
          </label>
          <div className="wide">
            <button disabled={pending}>Add recipient</button>
          </div>
        </form>
        <div className="recipient-list">
          {beneficiaries.map((b) => (
            <article className="recipient-row" key={b.id}>
              <div>
                <strong>{b.label}</strong>
                <p>
                  Reference ···· {b.receivingReference.slice(-8)} · Verified{" "}
                  {time(b.verifiedAt)}
                </p>
              </div>
              <Badge tone={b.state === "VERIFIED" ? "good" : "neutral"}>
                {b.state === "VERIFIED" ? "Verified" : "Revoked"}
              </Badge>
              {b.state === "VERIFIED" && (
                <button
                  className="secondary"
                  disabled={pending}
                  onClick={() => {
                    setPending(true);
                    void client
                      .POST("/api/v1/beneficiaries/{id}/revocation", {
                        params: { path: { id: b.id } },
                      })
                      .then(value)
                      .then(() => {
                        setNotice(
                          "Recipient revoked. Existing unused quotes cannot execute.",
                        );
                        return load();
                      })
                      .catch((e) => setError(message(e)))
                      .finally(() => setPending(false));
                  }}
                >
                  Revoke recipient
                </button>
              )}
            </article>
          ))}
        </div>
      </section>
      <Credits />
      <section className="card payment-card">
        <div className="spread">
          <h2>Your payments</h2>
          <button className="secondary" onClick={() => void load()}>
            Refresh payments
          </button>
        </div>
        {!payments.length ? (
          <p className="muted">
            Saved payment references will appear here, including requests
            awaiting approval or checks.
          </p>
        ) : (
          payments.map((p) => (
            <a className="payment-row" href={"#/payments/" + p.id} key={p.id}>
              <div>
                <strong>{p.beneficiaryLabel}</strong>
                <small>
                  {short(p.id)} · {time(p.createdAt)}
                </small>
              </div>
              <strong>{money(p.total)}</strong>
              <PaymentBadge state={p.state} returns={p.returns} />
            </a>
          ))
        )}
      </section>
    </>
  );
}
function Receiving({ accounts }: { accounts: Account[] }) {
  const { client } = useSession();
  const [reference, setReference] = useState("");
  const [error, setError] = useState("");
  const [pending, setPending] = useState(false);
  const eligible = accounts.filter(
    (a) => a.state === "ACTIVE" && !a.creditBlocked,
  );
  return (
    <section className="card payment-card">
      <h2>Share to receive</h2>
      <p className="muted">
        Create your receiving reference and share it with another synthetic
        account holder.
      </p>
      {error && <Alert>{error}</Alert>}
      {eligible.length ? (
        <form
          onChange={() => setReference("")}
          onSubmit={(e) => {
            e.preventDefault();
            const id = String(new FormData(e.currentTarget).get("account"));
            setPending(true);
            void client
              .POST("/api/v1/accounts/{id}/receiving-reference", {
                params: { path: { id } },
              })
              .then(value)
              .then((r) => {
                setReference(r.reference);
                setError("");
              })
              .catch((e) => setError(message(e)))
              .finally(() => setPending(false));
          }}
        >
          <label>
            Receiving account
            <select name="account">
              {eligible.map((a) => (
                <option key={a.id} value={a.id}>
                  Everyday account · {short(a.id)}
                </option>
              ))}
            </select>
          </label>
          <button className="secondary" disabled={pending}>
            Show receiving reference
          </button>
        </form>
      ) : (
        <p>No account is currently enabled to receive.</p>
      )}
      {reference && (
        <div className="receiving-reference" role="status">
          <strong>Your receiving reference</strong>
          <code>{reference}</code>
          <small>
            Only TST internal payments. This is a synthetic reference.
          </small>
          <QrCode
            payload={encodePaymentCode({ reference })}
            label="QR code for your receiving reference"
          />
          <small>
            Scanning this code only fills in a recipient. Every payment still
            needs its own quote and approval.
          </small>
        </div>
      )}
    </section>
  );
}
function PaymentDetail({ id }: { id: string }) {
  const { client } = useSession();
  const [payment, setPayment] = useState<Payment>();
  const [error, setError] = useState("");
  const [pending, setPending] = useState(false);
  const selectedId = useRef(id);
  selectedId.current = id;
  const acceptPayment = useCallback((next: Payment) => {
    if (next.id !== selectedId.current) return;
    setPayment((current) =>
      current?.id === next.id && current.version > next.version
        ? current
        : next,
    );
  }, []);
  const load = useCallback(async () => {
    try {
      acceptPayment(
        await client
          .GET("/api/v1/payments/{id}", { params: { path: { id } } })
          .then(value),
      );
      setError("");
    } catch (e) {
      setError(message(e));
    }
  }, [client, id, acceptPayment]);
  useEffect(() => {
    setPayment((current) => (current?.id === id ? current : undefined));
    void load();
  }, [load, id]);
  useEffect(() => {
    if (
      !payment ||
      (![
        "ACCEPTED",
        "CONTROLS_PENDING",
        "CONTROL_REVIEW",
        "RESERVED",
        "SUBMITTING",
        "SUBMITTED",
        "OUTCOME_UNKNOWN",
        "RAIL_PAUSED",
        "RECONCILING",
        "CANCELLATION_PENDING",
      ].includes(payment.state) &&
        (!payment.cancellation || payment.cancellation.state === "COMPLETED"))
    )
      return;
    const timer = window.setInterval(() => void load(), 2000);
    return () => window.clearInterval(timer);
  }, [payment?.state, load]);
  async function verify() {
    setPending(true);
    try {
      const current = await client
        .POST("/api/v1/payments/{id}/approval", { params: { path: { id } } })
        .then(value);
      if (
        !current.approvalUrl ||
        !/^\/auth\/customer\/step-up\/[a-f0-9-]{36}$/.test(current.approvalUrl)
      )
        throw new Error("Approval unavailable");
      sessionStorage.setItem("pb:return", "/payments/" + id);
      window.location.assign(current.approvalUrl);
    } catch (e) {
      setError(message(e));
      setPending(false);
    }
  }
  return (
    <>
      <a className="back-link" href="#/payments">
        ← Your payments
      </a>
      {error && <Alert>{error}</Alert>}
      {!payment ? (
        <Busy />
      ) : (
        <>
          <PageTitle
            eyebrow="YOUR PAYMENT REFERENCE"
            title={
              payment.returns.status !== "NONE"
                ? "Returned funds, clearly tracked."
                : payment.state === "SETTLED"
                  ? "Payment completed."
                  : payment.state === "DRAFT"
                    ? "Review your payment."
                    : "Your payment, clearly tracked."
            }
            description={
              "Reference " +
              short(payment.id) +
              " · " +
              payment.details.rail +
              " TST payment"
            }
          />
          <section className="card payment-card payment-review">
            <div className="spread">
              <h2>{payment.beneficiaryLabel}</h2>
              <PaymentBadge state={payment.state} returns={payment.returns} />
            </div>
            <dl className="payment-facts">
              <div>
                <dt>From account</dt>
                <dd>Everyday · {short(payment.details.payerAccountId)}</dd>
              </div>
              <div>
                <dt>Business date</dt>
                <dd>{payment.details.requestedDate} · UTC</dd>
              </div>
              <div>
                <dt>Amount to recipient</dt>
                <dd>{money(payment.details.amount)}</dd>
              </div>
              <div>
                <dt>Exact fee</dt>
                <dd>{money(payment.details.fee)}</dd>
              </div>
              <div className="payment-total">
                <dt>Total debit</dt>
                <dd>{money(payment.total)}</dd>
              </div>
            </dl>
            {payment.returns.status !== "NONE" && (
              <div className="notice">
                <strong>
                  {payment.returns.status === "RETURNED"
                    ? "Principal returned to your account."
                    : payment.returns.status === "PARTIALLY_RETURNED"
                      ? "Part of the principal has been returned."
                      : "Returned funds are awaiting checks."}
                </strong>
                <p>
                  Provider returned {money(payment.returns.received)} · Credited{" "}
                  {money(payment.returns.credited)}. The original fee is
                  preserved.
                </p>
              </div>
            )}
            {payment.allowedActions.includes("CANCEL_DRAFT") && (
              <button
                className="secondary"
                disabled={pending}
                onClick={() => {
                  setPending(true);
                  void client
                    .POST("/api/v1/payments/{id}/draft-cancellation", {
                      params: { path: { id } },
                    })
                    .then(value)
                    .then(acceptPayment)
                    .catch((e) => setError(message(e)))
                    .finally(() => setPending(false));
                }}
              >
                Cancel unconfirmed draft
              </button>
            )}
            {payment.allowedActions.includes("CANCEL_PAYMENT") && (
              <button
                className="secondary"
                disabled={pending}
                onClick={() => {
                  setPending(true);
                  void client
                    .POST("/api/v1/payments/{id}/cancellation", {
                      params: {
                        path: { id },
                        header: {
                          "Idempotency-Key": stableKey(
                            "pb:payment-cancellation:" + id,
                          ),
                        },
                      },
                    })
                    .then(value)
                    .then(() => load())
                    .catch((e) => setError(message(e)))
                    .finally(() => setPending(false));
                }}
              >
                Request cancellation
              </button>
            )}
            {payment.cancellation && (
              <p className="notice" role="status">
                {payment.state === "CANCELLED"
                  ? "Cancellation is confirmed. No payment was booked."
                  : payment.state === "SETTLED"
                    ? "The payment completed before cancellation could take effect."
                    : payment.cancellation.state !== "COMPLETED"
                      ? "Cancellation is requested. Any reserved funds remain unavailable until the outcome is confirmed."
                      : "Cancellation did not change the payment's final outcome."}
              </p>
            )}
            {payment.state === "DRAFT" ? (
              <>
                <p>
                  Review these exact details. Verification is specific to this
                  recipient, amount, fee, date and quote.
                </p>
                <p className="muted">
                  Quote expires {time(payment.expiresAt)}. Issuer verification
                  must also be current when execution starts.
                </p>
                {payment.approvalReady ? (
                  <>
                    <p className="notice" role="status">
                      Your verification is current. Confirm this exact payment
                      to continue.
                    </p>
                    <button
                      disabled={
                        pending || new Date(payment.expiresAt) <= new Date()
                      }
                      onClick={() => {
                        setPending(true);
                        void client
                          .POST("/api/v1/payments/{id}/confirmation", {
                            params: { path: { id } },
                            body: { quoteId: payment.quoteId },
                          })
                          .then(value)
                          .then(acceptPayment)
                          .catch((e) => setError(message(e)))
                          .finally(() => setPending(false));
                      }}
                    >
                      Confirm payment
                    </button>
                  </>
                ) : (
                  <button
                    disabled={
                      pending || new Date(payment.expiresAt) <= new Date()
                    }
                    onClick={() => void verify()}
                  >
                    Verify this payment
                  </button>
                )}
              </>
            ) : payment.state === "SETTLED" ? (
              <div className="notice">
                <strong>Payment booked.</strong>
                <p>
                  Your account was debited for the recipient amount and fee.
                </p>
                <small>Booking reference {payment.batchId}</small>
              </div>
            ) : ["EXPIRED", "DECLINED", "BLOCKED", "CANCELLED"].includes(
                payment.state,
              ) ? (
              <p role="status">
                This instruction did not complete a payment. Review its current
                details and start a new quote if appropriate.
              </p>
            ) : (
              <p role="status">
                Your original request is saved.{" "}
                {payment.details.rail === "SIMULATED"
                  ? "Any reserved funds remain unavailable while the provider outcome is pending"
                  : "Current checks are in progress"}
                {payment.state === "CONTROL_REVIEW" ? " with our team" : ""}.
                Completion has not been confirmed yet.
              </p>
            )}
            <div className="review-footer">
              <button className="secondary" onClick={() => void load()}>
                Refresh payment status
              </button>
              <span>Updated {time(payment.updatedAt)}</span>
            </div>
          </section>
          <Credits originalId={id} />
        </>
      )}
    </>
  );
}
