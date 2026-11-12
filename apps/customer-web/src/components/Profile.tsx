import { useState, useCallback, useEffect } from "react";
import {
  Alert,
  Icon,
  message,
  time,
  useSession,
  value,
  type Schema,
} from "@piggybank/web-core";
import { stableKey, doneKey } from "../lib/navigation";
import { PageTitle } from "../lib/display";

export function ProfileView({ hasAccount }: { hasAccount: boolean }) {
  const { client } = useSession();
  const [profile, setProfile] = useState<Schema["CustomerProfile"] | null>(
    null,
  );
  const [email, setEmail] = useState("");
  const [change, setChange] = useState<Schema["ContactStatus"] | null>(null);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const load = useCallback(async () => {
    if (!hasAccount) return;
    try {
      setProfile(await value(await client.GET("/api/v1/profile")));
      setError("");
    } catch (e) {
      setError(message(e));
    }
  }, [client, hasAccount]);
  useEffect(() => {
    void load();
  }, [load]);
  useEffect(() => {
    if (!change || !["PENDING", "REVIEW"].includes(change.state)) return;
    const timer = setInterval(() => {
      void client
        .GET("/api/v1/profile/contact-changes/{id}", {
          params: { path: { id: change.id } },
        })
        .then(value)
        .then((current) => {
          setChange(current);
          if (current.state === "VERIFIED") void load();
        })
        .catch((e) => setError(message(e)));
    }, 2500);
    return () => clearInterval(timer);
  }, [change?.id, change?.state, client, load]);
  return (
    <>
      <PageTitle
        eyebrow="Your profile"
        title="Keep your details in step."
        description="Contact changes are checked before they replace your verified information."
      />
      <div className="account-detail-grid">
        <section className="card">
          <h2>Verified contact</h2>
          {error ? <Alert>{error}</Alert> : null}
          {!hasAccount ? (
            <p className="muted">
              Complete your application to manage verified account contact
              details.
            </p>
          ) : (
            <>
              <dl className="profile-facts">
                <div>
                  <dt>Email</dt>
                  <dd>
                    {profile?.email ?? "No separately verified contact yet"}
                  </dd>
                </div>
                {profile?.verifiedAt ? (
                  <div>
                    <dt>Verified</dt>
                    <dd>{time(profile.verifiedAt)}</dd>
                  </div>
                ) : null}
              </dl>
              <form
                onSubmit={(event) => {
                  event.preventDefault();
                  setBusy(true);
                  setError("");
                  const scope = "pb:contact-change";
                  void client
                    .POST("/api/v1/profile/contact-changes", {
                      params: {
                        header: { "Idempotency-Key": stableKey(scope) },
                      },
                      body: { email },
                    })
                    .then(value)
                    .then((current) => {
                      setChange(current);
                      doneKey(scope);
                    })
                    .catch((e) => setError(message(e)))
                    .finally(() => setBusy(false));
                }}
              >
                <label>
                  New synthetic email
                  <input
                    type="email"
                    required
                    pattern="[^@]+@example\.invalid"
                    placeholder="name@example.invalid"
                    value={email}
                    onChange={(e) => setEmail(e.target.value)}
                  />
                </label>
                <button className="secondary form-actions" disabled={busy}>
                  Request verified change
                </button>
              </form>
              {change ? (
                <div className="toast" role="status">
                  {change.state === "VERIFIED"
                    ? "Your verified contact is updated."
                    : change.state === "DENIED"
                      ? "This contact change could not be approved."
                      : change.state === "REVIEW"
                        ? "Your change requires review. Your current verified value remains in place."
                        : "Your contact change is being checked. Your current value remains in place."}
                </div>
              ) : null}
              <p className="hint">
                Verification in this demonstration uses synthetic control
                outcomes.
              </p>
            </>
          )}
        </section>
        <section className="card calm-card">
          <span className="feature-icon">
            <Icon name="shield" />
          </span>
          <h2>A little more peace of mind.</h2>
          <p className="muted">
            Manage your sign-in, authentication factors and active devices with
            your identity account.
          </p>
          <a className="button-link" href="/auth/customer/security">
            Open security settings <Icon name="arrow" />
          </a>
          <div className="divider" />
          <p className="muted">
            Legal identity corrections need a reviewed support request. Your
            original application evidence stays linked to its decision.
          </p>
          <a className="text-link" href="#/support">
            Visit support
          </a>
        </section>
      </div>
    </>
  );
}
