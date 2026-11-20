import { useCallback, useEffect, useState } from "react";
import { Alert, message, useSession, value } from "@piggybank/web-core";
import { Box } from "./components";

/* WP-38 advisory insights: service notices always; suggestions (bank or partner) only while marketing is allowed.
   Each insight explains why it is shown, links only to a page in this app and never acts on the customer's behalf. */
type Insight = {
  id: string;
  source: "BANK_RULE" | "PARTNER";
  purpose: "MARKETING" | "SERVICE_OUTREACH";
  title: string;
  body: string;
  actionRoute: string | null;
  why: string;
  createdAt: string;
};
type View = { marketingPermitted: boolean; insights: Insight[] };

export function Insights({ refresh }: { refresh: number }) {
  const { client } = useSession(),
    [view, setView] = useState<View>(),
    [error, setError] = useState("");
  const load = useCallback(async () => {
    try {
      setView(
        (await client.GET("/api/v1/insights").then(value)) as unknown as View,
      );
    } catch (e) {
      setError(message(e));
    }
  }, [client]);
  useEffect(() => {
    void load();
  }, [load, refresh]);
  async function dismiss(id: string) {
    try {
      setView(
        (await client
          .POST("/api/v1/insights/{id}/dismissal", {
            params: { path: { id } },
          })
          .then(value)) as unknown as View,
      );
    } catch (e) {
      setError(message(e));
    }
  }
  return (
    <Box title="Suggestions and notices">
      {error && <Alert>{error}</Alert>}
      <p className="ui-caption">
        {view?.marketingPermitted
          ? "You allow product announcements, so suggestions can appear here. Turn them off at any time."
          : "Product announcements are off, so only service notices appear here."}
      </p>
      {view?.insights.map((i) => (
        <article className="ui-mini-card" key={i.id}>
          <strong>{i.title}</strong>{" "}
          <span className="ui-pill">
            {i.purpose === "MARKETING" ? "Suggestion" : "Service notice"}
          </span>
          <p>{i.body}</p>
          <details>
            <summary>Why am I seeing this?</summary>
            <p className="ui-caption">{i.why}</p>
          </details>
          <div className="ui-actions">
            {i.actionRoute && <a href={i.actionRoute}>Open</a>}
            <button
              className="secondary"
              onClick={() => void dismiss(i.id)}
              aria-label={`Dismiss ${i.title}`}
            >
              Dismiss
            </button>
          </div>
        </article>
      ))}
      {view && !view.insights.length && (
        <p className="ui-caption">Nothing to show right now.</p>
      )}
    </Box>
  );
}
