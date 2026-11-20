import { useCallback, useEffect, useState } from "react";
import {
  Alert,
  Busy,
  message,
  short,
  time,
  useSession,
  value,
} from "@piggybank/web-core";
import type { Account } from "../types";
import { PageTitle, StateBadge } from "../lib/display";
import { doneKey, navigate, stableKey } from "../lib/navigation";

/* WP-33 connected apps (customer consent to registered partners) and shared account access (delegation). */
type Consent = {
  id: string;
  clientId: string;
  partnerName: string;
  scopes: string[];
  durationDays: number;
  state: string;
  accountIds: string[];
  authorizationDeadline: string;
  authorizedAt: string | null;
  expiresAt: string | null;
  revokedAt: string | null;
  revokedBy: string | null;
};
type Delegation = {
  id: string;
  accountId: string;
  authority: "VIEW" | "PAY";
  paymentLimit: { minorUnits: string } | null;
  state: string;
  expiresAt: string;
  revokedAt: string | null;
};
const scopeLabels: Record<string, string> = {
  "accounts:read": "See balances and transactions",
  "payments:initiate":
    "Prepare payments for you to approve here (it can never approve them)",
};
function tst(minor: string) {
  return `TST ${(Number(minor) / 100).toFixed(2)}`;
}
function accountName(id: string, accounts: Account[]) {
  return accounts.some((a) => a.id === id)
    ? `Your account ${short(id)}`
    : `Account ${short(id)}`;
}

export function SharingView({
  accounts,
  consentId,
}: {
  accounts: Account[];
  consentId?: string;
}) {
  const { client } = useSession();
  const [consents, setConsents] = useState<Consent[]>([]);
  const [granted, setGranted] = useState<Delegation[]>([]);
  const [received, setReceived] = useState<Delegation[]>([]);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const load = useCallback(async () => {
    try {
      const [list, delegations] = await Promise.all([
        client.GET("/api/v1/consents").then(value),
        client.GET("/api/v1/delegations").then(value),
      ]);
      setConsents(list as unknown as Consent[]);
      const d = delegations as unknown as {
        granted: Delegation[];
        received: Delegation[];
      };
      setGranted(d.granted);
      setReceived(d.received);
    } catch (e) {
      setError(message(e));
    }
  }, [client]);
  useEffect(() => {
    void load();
  }, [load]);
  return (
    <>
      <PageTitle
        eyebrow="You decide who sees what"
        title="Connected apps & sharing"
        description="Apps you have connected and people you share an account with. You can stop any of them at any time; it takes effect straight away."
      />
      {error ? <Alert>{error}</Alert> : null}
      {notice ? (
        <div className="toast" role="status">
          {notice}
        </div>
      ) : null}
      {consentId ? (
        <ConsentRequest
          id={consentId}
          accounts={accounts}
          done={(text) => {
            setNotice(text);
            navigate("/sharing");
            void load();
          }}
        />
      ) : null}
      <Assistance changed={setNotice} />
      <section className="section-space" aria-labelledby="apps-heading">
        <h2 id="apps-heading">Connected apps</h2>
        {consents.filter((c) => c.state !== "AWAITING_AUTHORIZATION").length ===
        0 ? (
          <p className="muted">You have not connected any apps.</p>
        ) : (
          <ul className="card-list">
            {consents
              .filter((c) => c.state !== "AWAITING_AUTHORIZATION")
              .map((c) => (
                <li className="card" key={c.id}>
                  <ConnectedApp
                    consent={c}
                    accounts={accounts}
                    changed={(text) => {
                      setNotice(text);
                      void load();
                    }}
                  />
                </li>
              ))}
          </ul>
        )}
      </section>
      <SharedAccess
        accounts={accounts}
        granted={granted}
        received={received}
        changed={(text) => {
          setNotice(text);
          void load();
        }}
      />
    </>
  );
}

function ConsentRequest({
  id,
  accounts,
  done,
}: {
  id: string;
  accounts: Account[];
  done: (text: string) => void;
}) {
  const { client } = useSession();
  const [consent, setConsent] = useState<Consent | null>(null);
  const [chosen, setChosen] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  useEffect(() => {
    client
      .GET("/api/v1/consents/{id}", { params: { path: { id } } })
      .then(value)
      .then((v) => setConsent(v as unknown as Consent))
      .catch((e) => setError(message(e)));
  }, [client, id]);
  const active = accounts.filter((a) => a.state === "ACTIVE");
  async function decide(approve: boolean) {
    setBusy(true);
    setError("");
    try {
      await client
        .POST("/api/v1/consents/{id}/authorization", {
          params: { path: { id } },
          body: { approve, accountIds: approve ? chosen : [] },
        })
        .then(value);
      done(
        approve
          ? `${consent?.partnerName ?? "The app"} is now connected.`
          : "You declined the request. Nothing was shared.",
      );
    } catch (e) {
      setError(message(e));
      setBusy(false);
    }
  }
  if (error && !consent) {
    return (
      <Alert>
        This request is no longer available. Ask the app to start again.
      </Alert>
    );
  }
  if (!consent) return <Busy />;
  if (consent.state !== "AWAITING_AUTHORIZATION") {
    return (
      <p className="muted" role="status">
        You already answered this request ({consent.state.toLowerCase()}).
      </p>
    );
  }
  return (
    <section className="card consent-request" aria-labelledby="request-heading">
      <span className="eyebrow">Access request</span>
      <h2 id="request-heading">
        {consent.partnerName} would like to connect to your accounts
      </h2>
      <p>For {consent.durationDays} days, it asks to:</p>
      <ul>
        {consent.scopes.map((s) => (
          <li key={s}>{scopeLabels[s] ?? s}</li>
        ))}
      </ul>
      <fieldset>
        <legend>Choose the accounts to share</legend>
        {active.length === 0 ? (
          <p className="muted">You have no active accounts to share.</p>
        ) : (
          active.map((a) => (
            <label className="check" key={a.id}>
              <input
                type="checkbox"
                checked={chosen.includes(a.id)}
                onChange={(e) =>
                  setChosen(
                    e.target.checked
                      ? [...chosen, a.id]
                      : chosen.filter((x) => x !== a.id),
                  )
                }
              />
              {accountName(a.id, accounts)}
            </label>
          ))
        )}
      </fieldset>
      <p className="muted">
        Answer before {time(consent.authorizationDeadline)}. You can disconnect
        the app here at any time.
      </p>
      {error ? <Alert>{error}</Alert> : null}
      <div className="row">
        <button
          onClick={() => void decide(true)}
          disabled={busy || chosen.length === 0}
        >
          Allow access
        </button>
        <button
          className="secondary"
          onClick={() => void decide(false)}
          disabled={busy}
        >
          Decline
        </button>
      </div>
    </section>
  );
}

function ConnectedApp({
  consent,
  accounts,
  changed,
}: {
  consent: Consent;
  accounts: Account[];
  changed: (text: string) => void;
}) {
  const { client } = useSession();
  const [confirming, setConfirming] = useState(false);
  const [error, setError] = useState("");
  async function revoke() {
    try {
      await client
        .POST("/api/v1/consents/{id}/revocation", {
          params: { path: { id: consent.id } },
          body: { reason: "Customer disconnected the app" },
        })
        .then(value);
      changed(`${consent.partnerName} can no longer access your accounts.`);
    } catch (e) {
      setError(message(e));
    }
  }
  return (
    <div>
      <div className="spread">
        <h3>{consent.partnerName}</h3>
        <StateBadge state={consent.state} />
      </div>
      <ul className="muted">
        {consent.scopes.map((s) => (
          <li key={s}>{scopeLabels[s] ?? s}</li>
        ))}
      </ul>
      <p className="muted">
        {consent.accountIds.map((a) => accountName(a, accounts)).join(", ")}
        {consent.expiresAt && consent.state === "ACTIVE"
          ? ` · until ${time(consent.expiresAt)}`
          : ""}
        {consent.revokedAt ? ` · disconnected ${time(consent.revokedAt)}` : ""}
      </p>
      {error ? <Alert>{error}</Alert> : null}
      {consent.state === "ACTIVE" ? (
        confirming ? (
          <div className="row" role="group" aria-label="Confirm disconnect">
            <span>
              Disconnect {consent.partnerName}? Payments you already approved
              still complete.
            </span>
            <button onClick={() => void revoke()}>Disconnect</button>
            <button className="secondary" onClick={() => setConfirming(false)}>
              Keep
            </button>
          </div>
        ) : (
          <button className="secondary" onClick={() => setConfirming(true)}>
            Disconnect {consent.partnerName}
          </button>
        )
      ) : null}
    </div>
  );
}

function SharedAccess({
  accounts,
  granted,
  received,
  changed,
}: {
  accounts: Account[];
  granted: Delegation[];
  received: Delegation[];
  changed: (text: string) => void;
}) {
  const { client } = useSession();
  const [reference, setReference] = useState("");
  const [account, setAccount] = useState("");
  const [code, setCode] = useState("");
  const [authority, setAuthority] = useState<"VIEW" | "PAY">("VIEW");
  const [limit, setLimit] = useState("50.00");
  const [days, setDays] = useState(30);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const owned = accounts.filter((a) => a.state === "ACTIVE");
  async function myReference() {
    try {
      const v = (await client
        .POST("/api/v1/delegations/reference")
        .then(value)) as unknown as { reference: string };
      setReference(v.reference);
    } catch (e) {
      setError(message(e));
    }
  }
  async function share() {
    setBusy(true);
    setError("");
    const scope = "pb:delegation:" + account + code;
    try {
      const minor = Math.round(Number(limit) * 100);
      await client
        .POST("/api/v1/delegations", {
          params: { header: { "Idempotency-Key": stableKey(scope) } },
          body: {
            accountId: account,
            reference: code.trim(),
            authority,
            durationDays: days,
            ...(authority === "PAY" ? { paymentLimitMinor: minor } : {}),
          },
        })
        .then(value);
      doneKey(scope);
      setCode("");
      changed("Access shared. They can use it straight away.");
    } catch (e) {
      setError(message(e));
    } finally {
      setBusy(false);
    }
  }
  async function stop(d: Delegation, mine: boolean) {
    try {
      await client
        .POST("/api/v1/delegations/{id}/revocation", {
          params: { path: { id: d.id } },
          body: {
            reason: mine ? "Owner stopped sharing" : "Delegate gave up access",
          },
        })
        .then(value);
      changed(mine ? "Sharing stopped." : "You no longer have that access.");
    } catch (e) {
      setError(message(e));
    }
  }
  const describe = (d: Delegation) =>
    d.authority === "PAY" && d.paymentLimit
      ? `Can view and pay up to ${tst(d.paymentLimit.minorUnits)} per payment`
      : "View only";
  return (
    <section className="section-space" aria-labelledby="sharing-heading">
      <h2 id="sharing-heading">Shared account access</h2>
      {error ? <Alert>{error}</Alert> : null}
      <div className="card">
        <h3>Someone sharing with you?</h3>
        <p className="muted">Give them your reference. It lasts 24 hours.</p>
        {reference ? (
          <p>
            Your reference:{" "}
            <strong data-testid="delegate-reference">{reference}</strong>
          </p>
        ) : (
          <button className="secondary" onClick={() => void myReference()}>
            Show my reference
          </button>
        )}
      </div>
      {owned.length ? (
        <form
          className="card"
          onSubmit={(e) => {
            e.preventDefault();
            void share();
          }}
        >
          <h3>Share one of your accounts</h3>
          <label htmlFor="share-account">
            Account to share
            <select
              id="share-account"
              value={account}
              onChange={(e) => setAccount(e.target.value)}
              required
            >
              <option value="">Choose an account</option>
              {owned.map((a) => (
                <option key={a.id} value={a.id}>
                  {accountName(a.id, accounts)}
                </option>
              ))}
            </select>
          </label>
          <label>
            Their reference
            <input
              value={code}
              onChange={(e) => setCode(e.target.value)}
              placeholder="DLG-XXXXXXXX"
              pattern="[Dd][Ll][Gg]-[A-Za-z2-9]{8}"
              required
            />
          </label>
          <fieldset>
            <legend>What can they do?</legend>
            <label className="check">
              <input
                type="radio"
                name="authority"
                checked={authority === "VIEW"}
                onChange={() => setAuthority("VIEW")}
              />
              View balances and activity
            </label>
            <label className="check">
              <input
                type="radio"
                name="authority"
                checked={authority === "PAY"}
                onChange={() => setAuthority("PAY")}
              />
              View and make payments up to a limit
            </label>
          </fieldset>
          {authority === "PAY" ? (
            <label>
              Limit per payment (TST)
              <input
                inputMode="decimal"
                value={limit}
                onChange={(e) => setLimit(e.target.value)}
                pattern="[0-9]{1,6}(\.[0-9]{1,2})?"
                required
              />
            </label>
          ) : null}
          <label>
            For how long
            <select
              value={days}
              onChange={(e) => setDays(Number(e.target.value))}
            >
              <option value={7}>7 days</option>
              <option value={30}>30 days</option>
              <option value={90}>90 days</option>
            </select>
          </label>
          <button disabled={busy}>Share access</button>
        </form>
      ) : null}
      <h3>People you share with</h3>
      {granted.length === 0 ? (
        <p className="muted">You are not sharing any account.</p>
      ) : (
        <ul className="card-list">
          {granted.map((d) => (
            <li className="card spread" key={d.id}>
              <span>
                {accountName(d.accountId, accounts)} · {describe(d)}
                {d.state === "ACTIVE" ? ` · until ${time(d.expiresAt)}` : ""}
              </span>
              <span className="row">
                <StateBadge state={d.state} />
                {d.state === "ACTIVE" ? (
                  <button
                    className="secondary"
                    onClick={() => void stop(d, true)}
                  >
                    Stop sharing
                  </button>
                ) : null}
              </span>
            </li>
          ))}
        </ul>
      )}
      <h3>Accounts shared with you</h3>
      {received.length === 0 ? (
        <p className="muted">Nobody is sharing an account with you.</p>
      ) : (
        <ul className="card-list">
          {received.map((d) => (
            <li className="card spread" key={d.id}>
              <span>
                {accountName(d.accountId, accounts)} · {describe(d)}
              </span>
              <span className="row">
                <StateBadge state={d.state} />
                {d.state === "ACTIVE" ? (
                  <button
                    className="secondary"
                    onClick={() => void stop(d, false)}
                  >
                    Give up access
                  </button>
                ) : null}
              </span>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

/* WP-34: staff in a branch, private banker or call center can act for you only after you authorize the session here. */
type AssistedSession = {
  id: string;
  channel: string;
  purpose: string;
  state: string;
  operations: string[];
  expiresAt: string | null;
  authorizationDeadline: string;
};
const channelLabels: Record<string, string> = {
  BRANCH: "a branch colleague",
  PRIVATE_BANKER: "your private banker",
  CALL_CENTER: "our call center",
};
function Assistance({ changed }: { changed: (text: string) => void }) {
  const { client } = useSession();
  const [sessions, setSessions] = useState<AssistedSession[]>([]);
  const [error, setError] = useState("");
  const load = useCallback(async () => {
    try {
      setSessions(
        (await client
          .GET("/api/v1/assisted")
          .then(value)) as unknown as AssistedSession[],
      );
    } catch (e) {
      setError(message(e));
    }
  }, [client]);
  useEffect(() => {
    void load();
  }, [load]);
  const live = sessions.filter(
    (s) => s.state === "AWAITING_CUSTOMER" || s.state === "AUTHORIZED",
  );
  async function act(s: AssistedSession, approve: boolean | null) {
    try {
      if (approve === null)
        await client
          .POST("/api/v1/assisted/{id}/closure", {
            params: { path: { id: s.id } },
          })
          .then(value);
      else
        await client
          .POST("/api/v1/assisted/{id}/authorization", {
            params: { path: { id: s.id } },
            body: { approve },
          })
          .then(value);
      changed(
        approve === null
          ? "Assistance ended."
          : approve
            ? `You allowed ${channelLabels[s.channel] ?? "our colleague"} to help with your accounts.`
            : "You declined the request.",
      );
      void load();
    } catch (e) {
      setError(message(e));
    }
  }
  if (!live.length && !error) return null;
  return (
    <section className="section-space" aria-labelledby="assist-heading">
      <h2 id="assist-heading">Assistance requests</h2>
      {error ? <Alert>{error}</Alert> : null}
      <ul className="card-list">
        {live.map((s) => (
          <li className="card" key={s.id}>
            <p>
              <strong>{channelLabels[s.channel] ?? s.channel}</strong> asked to
              help: {s.purpose}
            </p>
            <p className="muted">
              They could{" "}
              {s.operations
                .map((o) => o.toLowerCase().replaceAll("_", " "))
                .join(" and ")}
              . They can never make payments for you.
              {s.state === "AUTHORIZED" && s.expiresAt
                ? ` Allowed until ${time(s.expiresAt)}.`
                : ""}
            </p>
            <div className="row">
              {s.state === "AWAITING_CUSTOMER" ? (
                <>
                  <button onClick={() => void act(s, true)}>
                    Allow assistance
                  </button>
                  <button
                    className="secondary"
                    onClick={() => void act(s, false)}
                  >
                    Decline
                  </button>
                </>
              ) : (
                <button className="secondary" onClick={() => void act(s, null)}>
                  End assistance
                </button>
              )}
            </div>
          </li>
        ))}
      </ul>
    </section>
  );
}
