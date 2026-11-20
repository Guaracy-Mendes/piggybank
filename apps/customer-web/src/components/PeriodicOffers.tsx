import { useCallback, useEffect, useRef, useState } from "react";
import {
  Alert,
  Badge,
  message,
  money,
  useSession,
  value,
  type Schema,
} from "@piggybank/web-core";
import { stableKey, doneKey } from "../lib/navigation";
export function PeriodicOffers({
  accountId,
  selected,
}: {
  accountId: string;
  selected?: string;
}) {
  const { client } = useSession();
  const [items, setItems] = useState<Schema["PublishedPeriodicOffer"][]>([]),
    [offer, setOffer] = useState<Schema["PublishedPeriodicOffer"]>(),
    [enrollment, setEnrollment] = useState<Schema["PeriodicEnrollment"]>();
  const [error, setError] = useState(""),
    [checked, setChecked] = useState(false),
    [busy, setBusy] = useState(false),
    [loading, setLoading] = useState(true);
  const generation = useRef(0),
    scope = useRef("");
  const current = accountId + ":" + (selected ?? "");
  scope.current = current;
  const load = useCallback(async () => {
    const g = ++generation.current;
    setLoading(true);
    try {
      const list = await client
        .GET("/api/v1/accounts/{id}/periodic-offers", {
          params: { path: { id: accountId } },
        })
        .then(value);
      let o: Schema["PublishedPeriodicOffer"] | undefined,
        r: Schema["PeriodicEnrollment"] | undefined;
      if (selected) {
        o = await client
          .GET("/api/v1/accounts/{id}/periodic-offers/{offerId}", {
            params: { path: { id: accountId, offerId: selected } },
          })
          .then(value);
        const reply = await client.GET(
          "/api/v1/accounts/{id}/periodic-offers/{offerId}/enrollment",
          { params: { path: { id: accountId, offerId: selected } } },
        );
        r = reply.response.status === 404 ? undefined : await value(reply);
        if (
          !o ||
          o.material.id !== selected ||
          (r &&
            (r.consent.accountId !== accountId ||
              r.consent.offer.material.id !== selected ||
              r.consent.offer.digest !== o.digest))
        )
          throw new Error(
            "Financial terms do not match this account and offer.",
          );
      }
      if (g !== generation.current || scope.current !== current) return;
      setItems(list);
      setOffer(o);
      setEnrollment(r);
      setError("");
    } catch (e) {
      if (g === generation.current && scope.current === current) {
        setItems([]);
        setOffer(undefined);
        setEnrollment(undefined);
        setChecked(false);
        setError(message(e));
      }
    } finally {
      if (g === generation.current && scope.current === current)
        setLoading(false);
    }
  }, [client, accountId, selected, current]);
  useEffect(() => {
    setOffer(undefined);
    setEnrollment(undefined);
    setChecked(false);
    void load();
    return () => {
      generation.current++;
    };
  }, [load]);
  async function act(withdraw: boolean) {
    if (!offer || !selected || busy || loading || (!withdraw && !checked))
      return;
    const g = ++generation.current,
      shown = offer.digest,
      name =
        "pb:financial-enrollment:" +
        current +
        ":" +
        (withdraw ? "withdraw:" : "accept:") +
        (withdraw ? enrollment?.consentDigest : shown);
    setBusy(true);
    setError("");
    try {
      const params = {
        path: { id: accountId, offerId: selected },
        header: { "Idempotency-Key": stableKey(name) },
      };
      const r = withdraw
        ? await client
            .POST(
              "/api/v1/accounts/{id}/periodic-offers/{offerId}/withdrawal",
              {
                params,
                body: {
                  consentDigest: enrollment!.consentDigest,
                  withdraw: true,
                },
              },
            )
            .then(value)
        : await client
            .POST(
              "/api/v1/accounts/{id}/periodic-offers/{offerId}/enrollment",
              { params, body: { offerDigest: shown, accepted: true } },
            )
            .then(value);
      if (
        r.consent.accountId !== accountId ||
        r.consent.offer.material.id !== selected ||
        r.consent.offer.digest !== shown
      )
        throw new Error("Consent receipt scope conflict.");
      if (g !== generation.current || scope.current !== current) return;
      doneKey(name);
      setEnrollment(r);
      setChecked(false);
    } catch (e) {
      if (g === generation.current && scope.current === current) {
        setEnrollment(undefined);
        setOffer(undefined);
        setChecked(false);
        setError(message(e));
      }
    } finally {
      if (g === generation.current && scope.current === current) setBusy(false);
    }
  }
  const rule = offer?.material.policy.calculation;
  return (
    <section
      className="card section-space periodic-offers"
      aria-label="Periodic financial enrollment"
    >
      <div className="section-title">
        <div>
          <h2>Periodic financial offers</h2>
          <p>Review the terms and choose whether to request enrollment.</p>
        </div>
        <button
          className="secondary"
          disabled={busy || loading}
          onClick={() => void load()}
        >
          Refresh financial offers
        </button>
      </div>
      {error && <Alert>{error}</Alert>}
      {!loading && !items.length && !error && (
        <p>No financial offers are available for this account.</p>
      )}
      <div>
        {items.map((o) => (
          <a
            className="case-row"
            key={o.material.id}
            href={"#/accounts/" + accountId + "/offers/" + o.material.id}
          >
            {o.material.policy.code} · {o.material.title}
          </a>
        ))}
      </div>
      {offer && (
        <article aria-label="Selected financial offer">
          <h3>{offer.material.title}</h3>
          <div className="disclosure-text" data-testid="financial-offer-text">
            {offer.material.text}
          </div>
          <p>
            {rule?.kind === "MONTHLY_FIXED_FEE"
              ? money(rule.amount) + " per complete calendar month"
              : rule?.annualBasisPoints +
                " annual basis points · ACT/365 fixed · half up each day"}
          </p>
          <dl className="details">
            <dt>Effective period</dt>
            <dd>
              {offer.material.policy.effectiveFrom} inclusive to{" "}
              {offer.material.policy.effectiveUntil} exclusive
            </dd>
            <dt>Offer reference</dt>
            <dd>{offer.material.id}</dd>
            <dt>Exact terms digest</dt>
            <dd>{offer.digest}</dd>
            <dt>Calendar</dt>
            <dd>
              {offer.material.policy.calendar.code} ·{" "}
              {offer.material.policy.calendar.cutoff} UTC · weekdays{" "}
              {offer.material.policy.calendar.businessWeekdays.join(", ")} ·
              holidays{" "}
              {offer.material.policy.calendar.holidays.join(", ") || "None"}
            </dd>
          </dl>
          <details>
            <summary>Full financial rule and calendar</summary>
            <pre>{JSON.stringify(offer.material.policy, null, 2)}</pre>
          </details>
          {enrollment ? (
            <div aria-label="Financial enrollment receipt">
              <Badge>{enrollment.status}</Badge>
              <p>{enrollment.consent.statement}</p>
              <dl className="details">
                <dt>Consent reference</dt>
                <dd data-testid="financial-consent-id">
                  {enrollment.consent.id}
                </dd>
                <dt>Accepted at (UTC)</dt>
                <dd>{enrollment.consent.acceptedAt}</dd>
                <dt>Consent digest</dt>
                <dd data-testid="financial-consent-digest">
                  {enrollment.consentDigest}
                </dd>
                <dt>Earliest whole period</dt>
                <dd>{enrollment.consent.earliestPeriodStart}</dd>
                <dt>Financial activation</dt>
                <dd>{enrollment.financialActivation}</dd>
                {enrollment.activation && (
                  <>
                    <dt>First activated period</dt>
                    <dd>{enrollment.activation.activation.firstPeriodStart}</dd>
                  </>
                )}
              </dl>
              {enrollment.status === "PENDING_ACTIVATION" && (
                <button
                  className="secondary"
                  disabled={busy || loading}
                  onClick={() => void act(true)}
                >
                  Withdraw pending enrollment
                </button>
              )}
              {enrollment.status === "WITHDRAWN" && (
                <p>
                  Your pending request was withdrawn. The original terms and
                  receipt remain available.
                </p>
              )}
              {enrollment.status === "ACTIVE" && (
                <p>
                  Periodic finance is active for future whole periods. Posted
                  fees, credits and linked corrections appear in transaction
                  history. Contact support to stop future effects.
                </p>
              )}
              {enrollment.status === "PAUSED" && (
                <p>
                  Future periodic effects are stopped. Your original receipts
                  remain available.
                </p>
              )}
            </div>
          ) : (
            <>
              <label className="review-checkbox">
                <input
                  type="checkbox"
                  checked={checked}
                  disabled={busy || loading}
                  onChange={(e) => setChecked(e.target.checked)}
                />
                <span>
                  I accept these exact synthetic financial terms and request
                  enrollment pending separate activation. Future fees or
                  interest credits follow the displayed rule.
                </span>
              </label>
              <button
                disabled={!checked || busy || loading}
                onClick={() => void act(false)}
              >
                Request financial enrollment
              </button>
              <p>
                Reading terms and saving a preview review do not accept this
                offer.
              </p>
            </>
          )}
        </article>
      )}
    </section>
  );
}
