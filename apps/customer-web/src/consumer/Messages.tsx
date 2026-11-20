import { useCallback, useEffect, useState } from "react";
import { Alert, message, useSession, value } from "@piggybank/web-core";
import { Box } from "./components";
import { Insights } from "./Insights";

/* WP-26 safe messages: captured synthetic deliveries with no amounts, balances or links that carry secrets. */
type Purpose = {
  code: "SECURITY" | "SERVICE" | "ACTIVITY" | "MARKETING";
  mandatory: boolean;
  description: string;
  enabled: boolean;
};
type Message = {
  id: string;
  purpose: string;
  channel: "SYNTHETIC_EMAIL" | "IN_APP";
  recipient: string;
  state: "PENDING" | "DELIVERED" | "FAILED";
  createdAt: string;
  deliveredAt: string | null;
  subject: string;
  body: string;
};
type Inbox = { purposes: Purpose[]; messages: Message[] };
const purposeName: Record<Purpose["code"], string> = {
  SECURITY: "Security notices",
  SERVICE: "Statements & requests",
  ACTIVITY: "Payment activity",
  MARKETING: "Product announcements",
};

export function Messages() {
  const { client } = useSession(),
    [inbox, setInbox] = useState<Inbox>(),
    [error, setError] = useState(""),
    [busy, setBusy] = useState(false),
    [changed, setChanged] = useState(0);
  const load = useCallback(async () => {
    try {
      setInbox(
        (await client
          .GET("/api/v1/notifications")
          .then(value)) as unknown as Inbox,
      );
    } catch (e) {
      setError(message(e));
    }
  }, [client]);
  useEffect(() => {
    void load();
    const timer = setInterval(() => void load(), 5000);
    return () => clearInterval(timer);
  }, [load]);
  async function toggle(p: Purpose) {
    setBusy(true);
    setError("");
    try {
      setInbox(
        (await client
          .POST("/api/v1/notifications/preferences", {
            body: { purpose: p.code, enabled: !p.enabled },
          })
          .then(value)) as unknown as Inbox,
      );
      setChanged((n) => n + 1);
    } catch (e) {
      setError(message(e));
    } finally {
      setBusy(false);
    }
  }
  return (
    <div className="ui-grid two">
      <Box title="Messages">
        {error && <Alert>{error}</Alert>}
        <p className="ui-caption">
          Copies of the emails and inbox messages we send. They never include
          amounts, balances or sign-in links.
        </p>
        {inbox?.messages.map((m) => (
          <article className="ui-mini-card" key={m.id}>
            <strong>{m.subject}</strong>{" "}
            <span className="ui-pill">
              {m.state === "DELIVERED"
                ? m.channel === "SYNTHETIC_EMAIL"
                  ? "Emailed to " + m.recipient
                  : "In your inbox"
                : m.state === "PENDING"
                  ? "Sending…"
                  : "Not delivered"}
            </span>
            <p>{m.body}</p>
            <small className="ui-caption">
              {new Date(m.createdAt).toLocaleString()}
            </small>
          </article>
        ))}
        {inbox && !inbox.messages.length && (
          <p className="ui-caption">No messages yet.</p>
        )}
      </Box>
      <Box title="Message preferences">
        <ul className="ui-version-list" aria-label="Message purposes">
          {inbox?.purposes.map((p) => (
            <li key={p.code}>
              <span>
                <strong>{purposeName[p.code]}</strong>
                <br />
                <span className="ui-caption">{p.description}</span>
              </span>
              {p.mandatory ? (
                <span className="ui-caption">Always on</span>
              ) : (
                <label className="review-checkbox">
                  <input
                    type="checkbox"
                    checked={p.enabled}
                    disabled={busy}
                    onChange={() => void toggle(p)}
                  />
                  {p.enabled ? "On" : "Off"}
                  <span className="visually-hidden">
                    {" "}
                    for {purposeName[p.code]}
                  </span>
                </label>
              )}
            </li>
          ))}
        </ul>
      </Box>
      <Insights refresh={changed} />
    </div>
  );
}
