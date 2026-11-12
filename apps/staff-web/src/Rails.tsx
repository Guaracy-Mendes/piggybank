import { useCallback, useEffect, useState } from "react";
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
export function RailsView({ access }: { access: Access }) {
  const { client } = useSession();
  const [data, setData] = useState<Schema["RailConfiguration"]>();
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [busy, setBusy] = useState(false);
  const can = (s: string) => access.capabilities.includes(s);
  const load = useCallback(async () => {
    try {
      setData(await client.GET("/api/v1/staff/rails").then(value));
      setError("");
    } catch (e) {
      setError(message(e));
    }
  }, [client]);
  useEffect(() => {
    void load();
  }, [load]);
  function run(action: () => Promise<unknown>, success: string) {
    setBusy(true);
    setError("");
    void action()
      .then(() => {
        setNotice(success);
        return load();
      })
      .catch((e) => setError(message(e)))
      .finally(() => setBusy(false));
  }
  return (
    <>
      <div className="page-title">
        <div>
          <p className="eyebrow">SUBMISSION AUTHORITY</p>
          <h1>Control new rail submissions.</h1>
          <p>
            Pausing new instructions preserves inquiries and recovery for
            existing payments.
          </p>
        </div>
        <button className="secondary" onClick={() => void load()}>
          Refresh rail controls
        </button>
      </div>
      {error && <Alert>{error}</Alert>}
      {notice && (
        <p role="status" className="notice">
          {notice}
        </p>
      )}
      {data && (
        <>
          <section className="panel">
            <div className="panel-title">
              <h2>Simulated external rail</h2>
              <Badge tone={data.snapshot.paused ? "attention" : "good"}>
                {data.snapshot.paused
                  ? "New submissions paused"
                  : "New submissions enabled"}
              </Badge>
            </div>
            <p>
              Synthetic TST only · UTC calendar, available every day. Revision{" "}
              {data.snapshot.revision}.
            </p>
            {data.snapshot.policy && (
              <dl className="facts">
                <div>
                  <dt>Effective policy</dt>
                  <dd>
                    Version {data.snapshot.policy.version} ·{" "}
                    {short(data.snapshot.policy.id)}
                  </dd>
                </div>
                <div>
                  <dt>Per payment limit</dt>
                  <dd>{money(data.snapshot.policy.paymentLimit)}</dd>
                </div>
                <div>
                  <dt>Validity</dt>
                  <dd>
                    {data.snapshot.policy.effectiveFrom} to{" "}
                    {data.snapshot.policy.effectiveUntil} (exclusive)
                  </dd>
                </div>
              </dl>
            )}
            {can("rail.manage") &&
              (data.snapshot.paused ? (
                <button
                  disabled={busy}
                  onClick={() =>
                    run(
                      () =>
                        client
                          .POST("/api/v1/staff/rails/resume-proposal")
                          .then(value),
                      "Resume proposal saved. An independent approver must approve before application.",
                    )
                  }
                >
                  Propose resume
                </button>
              ) : (
                <button
                  disabled={busy}
                  onClick={() =>
                    run(
                      () =>
                        client.POST("/api/v1/staff/rails/pause").then(value),
                      "New submissions paused. Existing inquiries continue.",
                    )
                  }
                >
                  Pause new submissions
                </button>
              ))}
          </section>
          {can("rail.manage") && (
            <section className="panel">
              <h2>Propose a policy version</h2>
              <p>
                A change requires independent approval and application. An
                emergency pause stays in effect when a policy is published.
              </p>
              <form
                className="field-grid"
                onSubmit={(e) => {
                  e.preventDefault();
                  const f = new FormData(e.currentTarget);
                  const body: Schema["RailPolicy"] = {
                    id: crypto.randomUUID(),
                    rail: "SIMULATED",
                    version: Number(f.get("version")),
                    label: "SYNTHETIC",
                    enabled: f.get("enabled") === "on",
                    effectiveFrom: String(f.get("from")),
                    effectiveUntil: String(f.get("until")),
                    paymentLimit: {
                      minorUnits: String(f.get("limit")),
                      currency: { code: "TST", scale: 2 },
                    },
                    calendar: "SYNTHETIC_UTC_24X7",
                  };
                  run(
                    () =>
                      client.POST("/api/v1/staff/rails", { body }).then(value),
                    "Policy proposal saved for independent approval.",
                  );
                }}
              >
                <label>
                  Version
                  <input
                    name="version"
                    type="number"
                    min="1"
                    required
                    defaultValue={(data.snapshot.policy?.version ?? 0) + 1}
                  />
                </label>
                <label>
                  Per payment limit · TST minor units
                  <input
                    name="limit"
                    inputMode="numeric"
                    pattern="[1-9][0-9]{0,18}"
                    required
                    defaultValue={
                      data.snapshot.policy?.paymentLimit.minorUnits ?? "1000000"
                    }
                  />
                </label>
                <label>
                  Effective from · UTC
                  <input
                    name="from"
                    type="date"
                    required
                    defaultValue={new Date().toISOString().slice(0, 10)}
                  />
                </label>
                <label>
                  Effective until · exclusive
                  <input
                    name="until"
                    type="date"
                    required
                    defaultValue="2030-01-01"
                  />
                </label>
                <label>
                  <input name="enabled" type="checkbox" defaultChecked /> Enable
                  this policy
                </label>
                <button disabled={busy}>Propose rail policy</button>
              </form>
            </section>
          )}
          <section className="panel">
            <h2>Changes awaiting action and application receipts</h2>
            {data.changes.map((c) => (
              <article className="case-row" key={c.proposalId}>
                <div>
                  <strong>
                    {c.kind === "RESUME"
                      ? "Resume submissions"
                      : "Publish policy"}{" "}
                    · {short(c.proposalId)}
                  </strong>
                  <Badge>
                    {c.actedAt ? "Applied" : "Awaiting approval or application"}
                  </Badge>
                </div>
                <p>
                  Maker {short(c.makerPerson)} · {time(c.createdAt)}
                </p>
                {c.kind === "POLICY" ? (
                  <p>
                    Limit {money(c.details.paymentLimit as Schema["Money"])} ·
                    Version {String(c.details.version)} ·{" "}
                    {String(c.details.effectiveFrom)} to{" "}
                    {String(c.details.effectiveUntil)} ·{" "}
                    {c.details.enabled ? "Enabled" : "Disabled"}
                  </p>
                ) : (
                  <p>
                    Resume approval bound to revision{" "}
                    {String(c.details.revision)}.
                  </p>
                )}
                {!c.actedAt && (
                  <div className="actions">
                    {can("rail.approve") && (
                      <button
                        disabled={busy}
                        onClick={() =>
                          run(
                            () =>
                              client
                                .POST(
                                  "/api/v1/staff/rails/changes/{id}/approval",
                                  { params: { path: { id: c.proposalId } } },
                                )
                                .then(value),
                            "Independent approval recorded.",
                          )
                        }
                      >
                        Approve rail change
                      </button>
                    )}
                    {can("rail.manage") && (
                      <button
                        disabled={busy}
                        onClick={() =>
                          run(
                            () =>
                              client
                                .POST(
                                  "/api/v1/staff/rails/changes/{id}/application",
                                  { params: { path: { id: c.proposalId } } },
                                )
                                .then(value),
                            "Approved rail change applied.",
                          )
                        }
                      >
                        Apply approved change
                      </button>
                    )}
                  </div>
                )}
              </article>
            ))}
          </section>
        </>
      )}
    </>
  );
}
