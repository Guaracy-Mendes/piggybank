import { useState, useCallback, useEffect, type FormEvent } from "react";
import {
  Alert,
  Badge,
  Busy,
  Icon,
  message,
  short,
  time,
  useSession,
  value,
  type Schema,
} from "@piggybank/web-core";
import type { Application, Product } from "../types";
import { navigate, stableKey, doneKey } from "../lib/navigation";
import { StateBadge, PageTitle } from "../lib/display";

export function ApplicationList({
  applications,
}: {
  applications: Application[];
}) {
  return applications.length ? (
    <div className="stack">
      {applications.map((app) => (
        <article className="card application-row" key={app.id}>
          <div>
            <span className="eyebrow">Everyday account</span>
            <h3>
              Application <span className="reference">{short(app.id)}</span>
            </h3>
            <StateBadge state={app.state} />
          </div>
          <button
            className="secondary"
            onClick={() => navigate("/applications/" + app.id)}
          >
            {app.state === "DRAFT" || app.state === "INFORMATION_REQUIRED"
              ? "Continue application"
              : "View progress"}{" "}
            <Icon name="arrow" />
          </button>
        </article>
      ))}
    </div>
  ) : (
    <div className="card">
      <h3>A fresh page.</h3>
      <p className="muted">
        You have no saved applications. Choose an account to begin.
      </p>
    </div>
  );
}

export function ApplicationView({
  id,
  products,
  changed,
}: {
  id: string;
  products: Product[];
  changed: () => Promise<void>;
}) {
  const { client } = useSession();
  const [app, setApp] = useState<Application | null>(null);
  const [terms, setTerms] = useState<Schema["TermsDraft"] | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [saved, setSaved] = useState("");
  const [accepted, setAccepted] = useState(false);
  const [evidence, setEvidence] = useState("");
  const load = useCallback(async () => {
    try {
      const current = await value(
        await client.GET("/api/v1/applications/{id}", {
          params: { path: { id } },
        }),
      );
      setApp(current);
      setTerms(
        await value(
          await client.GET("/api/v1/products/{id}/terms", {
            params: { path: { id: current.productVersionId } },
          }),
        ),
      );
      setError("");
      return current;
    } catch (e) {
      setError(message(e));
      return null;
    }
  }, [client, id]);
  useEffect(() => {
    void load();
  }, [load]);
  useEffect(() => {
    if (
      !app ||
      !["SUBMITTING", "CONTROLS_PENDING", "MANUAL_REVIEW"].includes(app.state)
    )
      return;
    const interval = setInterval(() => {
      void load().then((current) => {
        if (current?.state === "ACTIVE") void changed();
      });
    }, 2500);
    return () => clearInterval(interval);
  }, [app?.state, load, changed]);
  async function act(action: () => Promise<unknown>, text: string) {
    setBusy(true);
    setError("");
    setSaved("");
    try {
      await action();
      setSaved(text);
      await load();
      await changed();
    } catch (e) {
      setError(message(e));
    } finally {
      setBusy(false);
    }
  }
  if (!app)
    return (
      <>
        {error ? <Alert>{error}</Alert> : <Busy />}
        <button className="secondary" onClick={() => void load()}>
          Refresh application
        </button>
      </>
    );
  const editable = ["DRAFT", "INFORMATION_REQUIRED"].includes(app.state);
  const currentTerms = !!app.acceptanceId && terms?.id === app.termsId;
  const ready = !!app.profile && !!app.evidenceId && currentTerms;
  const product = products.find((p) => p.id === app.productVersionId);
  return (
    <>
      <a className="back-link" href="#/applications">
        ← All applications
      </a>
      <PageTitle
        eyebrow={"Application " + short(app.id)}
        title={
          app.state === "ACTIVE"
            ? "Your account is ready."
            : editable
              ? "A few steps. A fresh start."
              : "Your progress, saved."
        }
        description={
          app.state === "ACTIVE"
            ? "Your account reference is saved with this application."
            : app.state === "MANUAL_REVIEW"
              ? "Our team is reviewing your application. You can sign out and return here for updates."
              : app.state === "CONTROLS_PENDING" || app.state === "SUBMITTING"
                ? "Your application is being checked. This page updates as your request progresses."
                : "Complete your details, attach synthetic evidence and accept the current terms."
        }
      />
      <div className="application-layout">
        <section className="stack">
          {error ? <Alert>{error}</Alert> : null}
          {saved ? (
            <div className="toast" role="status">
              {saved}
            </div>
          ) : null}
          {editable ? (
            <>
              <section className="card">
                <div className="section-title">
                  <h2>
                    <span className="step-number">1</span>Your details
                  </h2>
                  {app.profile ? <Badge tone="good">Saved</Badge> : null}
                </div>
                <ProfileForm
                  key={app.id + "-" + app.version}
                  initial={app.profile}
                  busy={busy}
                  onSave={(profile) =>
                    act(
                      () =>
                        client
                          .PUT("/api/v1/applications/{id}/profile", {
                            params: { path: { id } },
                            body: { version: app.version, profile },
                          })
                          .then(value),
                      "Your details are saved.",
                    )
                  }
                />
              </section>
              <section className="card">
                <div className="section-title">
                  <h2>
                    <span className="step-number">2</span>Identity evidence
                  </h2>
                  {app.evidenceId ? <Badge tone="good">Attached</Badge> : null}
                </div>
                <p className="muted">
                  Use synthetic evidence for this demonstration. Your attachment
                  is restricted to this application and authorized review.
                </p>
                <form
                  onSubmit={(event) => {
                    event.preventDefault();
                    void act(
                      () =>
                        client
                          .POST("/api/v1/applications/{id}/evidence", {
                            params: { path: { id } },
                            body: { content: evidence },
                          })
                          .then(value),
                      "Your evidence is attached.",
                    );
                  }}
                >
                  <label>
                    Synthetic evidence
                    <textarea
                      required
                      maxLength={32768}
                      value={evidence}
                      onChange={(e) => setEvidence(e.target.value)}
                      placeholder="Describe or paste a synthetic identity evidence fixture."
                    />
                  </label>
                  <div className="form-actions">
                    <button
                      className="secondary"
                      type="submit"
                      disabled={busy || !evidence.trim()}
                    >
                      Attach evidence
                    </button>
                    <label className="file-label">
                      Choose a text file
                      <input
                        type="file"
                        accept=".txt,.json,text/plain,application/json"
                        onChange={async (e) => {
                          const file = e.target.files?.[0];
                          if (!file) return;
                          if (file.size > 32768) {
                            setError(
                              "Choose a synthetic text file of 32 KB or less.",
                            );
                            return;
                          }
                          setEvidence(await file.text());
                        }}
                      />
                    </label>
                  </div>
                </form>
              </section>
              <section className="card">
                <div className="section-title">
                  <h2>
                    <span className="step-number">3</span>Your account terms
                  </h2>
                  {currentTerms ? <Badge tone="good">Accepted</Badge> : null}
                </div>
                {terms ? (
                  <>
                    <p className="muted">
                      {terms.title} · Version {terms.version}
                    </p>
                    <div
                      className="terms"
                      tabIndex={0}
                      aria-label="Exact account terms"
                    >
                      {terms.text}
                    </div>
                    {!currentTerms ? (
                      <>
                        <label className="check term-check">
                          <input
                            type="checkbox"
                            checked={accepted}
                            onChange={(e) => setAccepted(e.target.checked)}
                          />
                          I have read and accept version {terms.version} of
                          these synthetic account terms.
                        </label>
                        <button
                          className="secondary"
                          disabled={!accepted || busy}
                          onClick={() =>
                            void act(
                              () =>
                                client
                                  .POST("/api/v1/applications/{id}/terms", {
                                    params: { path: { id } },
                                    body: { termsId: terms.id },
                                  })
                                  .then(value),
                              "Your acceptance is saved with the exact terms version.",
                            )
                          }
                        >
                          Accept terms
                        </button>
                      </>
                    ) : null}
                  </>
                ) : (
                  <Busy />
                )}
              </section>
              <section className="card submit-card">
                <div>
                  <h2>Ready for the next step?</h2>
                  <p className="muted">
                    We'll check your application and keep its status here.
                  </p>
                </div>
                <button
                  disabled={!ready || busy}
                  onClick={() =>
                    void act(async () => {
                      const scope = "pb:submit:" + id;
                      await value(
                        await client.POST("/api/v1/applications/{id}/submit", {
                          params: {
                            path: { id },
                            header: { "Idempotency-Key": stableKey(scope) },
                          },
                        }),
                      );
                      doneKey(scope);
                    }, "Your application was submitted. Check this page for its current status.")
                  }
                >
                  Submit application <Icon name="arrow" />
                </button>
              </section>
            </>
          ) : (
            <section className="card">
              <div className="spread">
                <h2>Application status</h2>
                <StateBadge state={app.state} />
              </div>
              <p className="muted">
                {app.state === "ACTIVE"
                  ? "Your account has been opened with no initial funding."
                  : app.state === "DECLINED"
                    ? "Your application could not be approved. You can contact support for the next permitted step."
                    : app.state === "EXPIRED"
                      ? "This application has expired. Start a new application with current information."
                      : "Your submitted information is saved. Checks and review must complete before an account becomes available."}
              </p>
              <div className="divider" />
              {app.accountId ? (
                <button onClick={() => navigate("/accounts/" + app.accountId)}>
                  View your account <Icon name="arrow" />
                </button>
              ) : (
                <button className="secondary" onClick={() => void load()}>
                  Refresh status
                </button>
              )}
              <a className="text-link inline-gap" href="#/support">
                View support requests
              </a>
            </section>
          )}
        </section>
        <aside
          className="card application-summary"
          aria-label="Application progress"
        >
          <span className="eyebrow">Your application</span>
          <h2>{product?.name ?? "Everyday account"}</h2>
          <StateBadge state={app.state} />
          <ol className="progress-list">
            {[
              ["Details saved", !!app.profile],
              ["Evidence attached", !!app.evidenceId],
              ["Current terms accepted", currentTerms],
              ["Account ready", app.state === "ACTIVE"],
            ].map(([label, complete]) => (
              <li key={String(label)} className={complete ? "complete" : ""}>
                <span>{complete ? <Icon name="check" /> : null}</span>
                {label}
              </li>
            ))}
          </ol>
          <div className="divider" />
          <p className="muted">
            Your progress is linked to your signed-in identity. Return from
            another session to continue.
          </p>
          <small className="muted">
            Application expires {time(app.expiresAt)}
          </small>
        </aside>
      </div>
    </>
  );
}

export function ProfileForm({
  initial,
  busy,
  onSave,
}: {
  initial: Schema["ApplicantProfile"] | null;
  busy: boolean;
  onSave: (profile: Schema["ApplicantProfile"]) => Promise<void>;
}) {
  const [name, setName] = useState(initial?.fullName ?? "");
  const [email, setEmail] = useState(initial?.email ?? "");
  const [birth, setBirth] = useState(initial?.dateOfBirth ?? "");
  const [consent, setConsent] = useState(!!initial);
  const [marketing, setMarketing] = useState(initial?.marketing ?? false);
  function submit(event: FormEvent) {
    event.preventDefault();
    void onSave({
      fullName: name.trim(),
      email: email.trim(),
      dateOfBirth: birth,
      residency: "SYNTHETIC",
      consentVersion: "synthetic-consent-v1",
      marketing,
    });
  }
  return (
    <form onSubmit={submit}>
      <div className="field-grid">
        <label>
          Full name
          <input
            required
            maxLength={100}
            autoComplete="name"
            value={name}
            onChange={(e) => setName(e.target.value)}
          />
        </label>
        <label>
          Date of birth
          <input
            required
            type="date"
            autoComplete="bday"
            value={birth}
            max={new Date().toISOString().slice(0, 10)}
            onChange={(e) => setBirth(e.target.value)}
          />
        </label>
        <label className="wide">
          Synthetic contact email
          <input
            required
            type="email"
            aria-label="Synthetic contact email"
            autoComplete="email"
            pattern="[^@]+@example\.invalid"
            value={email}
            onChange={(e) => setEmail(e.target.value)}
            placeholder="name@example.invalid"
          />
          <span className="hint">
            Use an example.invalid address for the demonstration.
          </span>
        </label>
        <label className="check wide">
          <input
            required
            type="checkbox"
            checked={consent}
            onChange={(e) => setConsent(e.target.checked)}
          />
          I agree to the use of my synthetic information for this application.
        </label>
        <label className="check wide">
          <input
            type="checkbox"
            checked={marketing}
            onChange={(e) => setMarketing(e.target.checked)}
          />
          Allow optional product updates. I can withdraw this separately.
        </label>
      </div>
      <div className="form-actions">
        <button className="secondary" disabled={busy || !consent}>
          Save details
        </button>
        <span className="hint">Residency: synthetic demonstration</span>
      </div>
    </form>
  );
}
