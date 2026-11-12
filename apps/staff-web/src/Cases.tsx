import { useCallback, useEffect, useState } from "react";
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
import type { Access } from "./App";
type Case = Schema["CaseRecord"];
type Context = Schema["CaseContext"];
export function CasesView({
  access,
  selected,
}: {
  access: Access;
  selected?: string;
}) {
  const { client } = useSession();
  const [items, setItems] = useState<Case[]>([]);
  const [context, setContext] = useState<Context>();
  const [account, setAccount] = useState<Schema["SupportAccount"]>();
  const [error, setError] = useState("");
  const [pending, setPending] = useState(false);
  const [notice, setNotice] = useState("");
  const [disposition, setDisposition] =
    useState<Schema["CaseProposal"]["disposition"]>("ESCALATE");
  const load = useCallback(async () => {
    try {
      const list = await client
        .GET("/api/v1/staff/cases", { params: { query: { limit: 100 } } })
        .then(value);
      setItems(list);
      if (selected)
        setContext(
          await client
            .GET("/api/v1/staff/cases/{id}/context", {
              params: { path: { id: selected } },
            })
            .then(value),
        );
      else setContext(undefined);
      setError("");
    } catch (e) {
      setError(message(e));
    }
  }, [client, selected]);
  useEffect(() => {
    setContext(undefined);
    setAccount(undefined);
    setNotice("");
    setDisposition("ESCALATE");
    void load();
  }, [load]);
  async function act(work: () => Promise<unknown>, note: string) {
    setPending(true);
    setNotice("");
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
  const c = context?.case;
  const owner = c?.ownerPerson === access.personId;
  const review = access.capabilities.includes("case.review");
  const approve = access.capabilities.includes("case.approve");
  const proposals =
    context?.proposals.filter((p) => p.state === "PENDING") ?? [];
  const reviewable = context?.checks.some(
    (c) =>
      c.originalOutcome === "REVIEW" &&
      c.state === "REVIEW" &&
      c.expiresAt &&
      new Date(c.expiresAt) > new Date(),
  );
  return (
    <>
      <div className="page-title">
        <div>
          <p className="eyebrow">REVIEW & RESOLVE</p>
          <h1>A clear view of what needs you.</h1>
          <p>Current queues, evidence and the next permitted action.</p>
        </div>
        <button
          className="secondary"
          onClick={() => {
            setNotice("");
            void load();
          }}
        >
          Refresh queues
        </button>
      </div>
      {error && <Alert>{error}</Alert>}
      {notice && (
        <p role="status" className="notice">
          {notice}
        </p>
      )}
      <div className="queue-layout">
        <section className="panel queue-list" aria-label="Case queue">
          <div className="panel-title">
            <h2>Your queues</h2>
            <Badge>{items.length} shown</Badge>
          </div>
          {items.length === 0 ? (
            <p>No open cases in your current queues.</p>
          ) : (
            items.map((item) => (
              <a
                className={
                  "case-row " + (selected === item.id ? "selected" : "")
                }
                href={"#/cases/" + item.id}
                key={item.id}
              >
                <div>
                  <strong>
                    {item.kind === "CONTROL"
                      ? "Control review"
                      : item.kind === "SUPPORT"
                        ? "Customer support"
                        : item.kind.toLowerCase() + " handling"}
                  </strong>
                  <Badge
                    tone={item.severity === "CRITICAL" ? "danger" : "attention"}
                  >
                    {item.state.replaceAll("_", " ")}
                  </Badge>
                </div>
                <p>
                  {short(item.id)} · {item.queue}
                </p>
                <small>Due {time(item.deadline)}</small>
              </a>
            ))
          )}
        </section>
        <section className="panel case-detail" aria-label="Case detail">
          {selected && !context ? (
            <Busy />
          ) : !c ? (
            <div className="empty">
              <h2>Make the next step clear.</h2>
              <p>
                Select a case to review its source, evidence and disposition
                history.
              </p>
            </div>
          ) : (
            <>
              <div className="panel-title">
                <h2>Case {short(c.id)}</h2>
                <Badge tone={c.state === "CLOSED" ? "good" : "attention"}>
                  {c.state}
                </Badge>
              </div>
              <dl className="facts">
                <div>
                  <dt>Resource</dt>
                  <dd>
                    <code>{c.resourceId}</code>
                  </dd>
                </div>
                <div>
                  <dt>Next action</dt>
                  <dd>{c.nextAction.replaceAll("_", " ")}</dd>
                </div>
                <div>
                  <dt>Assigned person</dt>
                  <dd>{c.ownerPerson ? short(c.ownerPerson) : "Unassigned"}</dd>
                </div>
                <div>
                  <dt>Deadline</dt>
                  <dd>{time(c.deadline)}</dd>
                </div>
              </dl>
              {!c.ownerPerson && review && c.state !== "CLOSED" && (
                <button
                  disabled={pending}
                  onClick={() =>
                    void act(
                      () =>
                        client
                          .POST("/api/v1/staff/cases/{id}/claim", {
                            params: { path: { id: c.id } },
                          })
                          .then(value),
                      "Case assigned to you.",
                    )
                  }
                >
                  Claim case
                </button>
              )}
              {context?.checks.map((check) => (
                <article className="control" key={check.id}>
                  <h3>
                    {check.kind} · Original decision:{" "}
                    {check.originalOutcome ?? "Pending"}
                  </h3>
                  <p>
                    Current state: <strong>{check.state}</strong> · Policy{" "}
                    {check.policyVersion}
                  </p>
                  <p>
                    Reason: {check.reasonCode ?? "Awaiting evidence"} ·{" "}
                    {check.expiresAt
                      ? "Expires " + time(check.expiresAt)
                      : "No valid result"}
                  </p>
                  <small>
                    Provider evidence {check.providerEvidenceId ?? "pending"} ·
                    Input {check.inputDigest}
                  </small>
                </article>
              ))}
              <h3>Evidence objects</h3>
              {context?.objects.length ? (
                context.objects.map((o) => (
                  <div className="evidence" key={o.id}>
                    <span>
                      <strong>{short(o.id)}</strong> · {o.size} bytes
                      <small>SHA256 {o.digest}</small>
                    </span>
                    {o.state === "READY" && (
                      <a
                        href={`/api/v1/staff/cases/${c.id}/evidence/${o.id}`}
                        download
                      >
                        Download evidence
                      </a>
                    )}
                  </div>
                ))
              ) : (
                <p>No downloadable objects are attached to this case.</p>
              )}
              {c.kind === "SUPPORT" && owner && c.state !== "CLOSED" && (
                <article className="control">
                  <h3>Account context</h3>
                  {account ? (
                    <>
                      <p>Account {short(account.accountId)}</p>
                      <p>
                        Available{" "}
                        <strong>{money(account.balance.available)}</strong> ·
                        Held {money(account.balance.held)}
                      </p>
                      <small>
                        {account.balance.source} · {time(account.balance.asOf)}
                      </small>
                    </>
                  ) : (
                    <button
                      className="secondary"
                      disabled={pending}
                      onClick={() =>
                        void act(
                          async () =>
                            setAccount(
                              await client
                                .GET("/api/v1/staff/cases/{id}/account", {
                                  params: { path: { id: c.id } },
                                })
                                .then(value),
                            ),
                          "Current account context loaded.",
                        )
                      }
                    >
                      Read account context
                    </button>
                  )}
                </article>
              )}
              {owner && review && c.state === "OPEN" && (
                <section className="actions">
                  <h3>Review actions</h3>
                  <button
                    className="secondary"
                    disabled={pending}
                    onClick={() =>
                      void act(
                        () =>
                          client
                            .POST("/api/v1/staff/cases/{id}/information", {
                              params: { path: { id: c.id } },
                              body: {
                                reasonCode: "IDENTITY_EVIDENCE_REQUIRED",
                              },
                            })
                            .then(value),
                        "Customer information requested.",
                      )
                    }
                  >
                    Request information
                  </button>
                  <form
                    onSubmit={(e) => {
                      e.preventDefault();
                      void act(
                        () =>
                          client
                            .POST("/api/v1/staff/cases/{id}/proposals", {
                              params: { path: { id: c.id } },
                              body: { disposition },
                            })
                            .then(value),
                        "Disposition saved for independent approval.",
                      );
                    }}
                  >
                    <label>
                      Disposition
                      <select
                        value={disposition}
                        onChange={(e) =>
                          setDisposition(e.target.value as typeof disposition)
                        }
                      >
                        <option value="ESCALATE">
                          Escalate for further review
                        </option>
                        {c.kind === "SUPPORT" && (
                          <option value="RESOLVE_SERVICE">
                            Resolve support request
                          </option>
                        )}
                        {c.kind === "CONTROL" && reviewable && (
                          <>
                            <option value="CLEAR_CONTROL">
                              Clear current review
                            </option>
                            <option value="DECLINE">
                              Decline current review
                            </option>
                          </>
                        )}
                      </select>
                    </label>
                    <button disabled={pending}>Propose disposition</button>
                  </form>
                </section>
              )}
              {context?.proposals.length !== 0 && (
                <section className="actions">
                  <h3>Disposition history</h3>
                  {context?.proposals.map((p) => (
                    <article className="proposal" key={p.id}>
                      <strong>{p.disposition.replaceAll("_", " ")}</strong>
                      <p>
                        {p.state} · Maker {short(p.makerPerson)} · Expires{" "}
                        {time(p.expiresAt)}
                      </p>
                      {p.state === "PENDING" &&
                        approve &&
                        p.makerPerson !== access.personId && (
                          <button
                            disabled={pending}
                            onClick={() =>
                              void act(
                                () =>
                                  client
                                    .POST("/api/v1/staff/cases/{id}/approval", {
                                      params: { path: { id: c.id } },
                                      body: { proposalId: p.id },
                                    })
                                    .then(value),
                                "Independent disposition approved. The owning workflow will resume if its current gates allow it.",
                              )
                            }
                          >
                            Approve disposition
                          </button>
                        )}
                    </article>
                  ))}
                  {owner && review && proposals.length > 0 && (
                    <button
                      className="secondary"
                      disabled={pending}
                      onClick={() =>
                        void act(
                          () =>
                            client
                              .POST("/api/v1/staff/cases/{id}/withdrawal", {
                                params: { path: { id: c.id } },
                              })
                              .then(value),
                          "Proposal withdrawn. Review the current case before proposing again.",
                        )
                      }
                    >
                      Withdraw pending proposal
                    </button>
                  )}
                </section>
              )}
            </>
          )}
        </section>
      </div>
    </>
  );
}
