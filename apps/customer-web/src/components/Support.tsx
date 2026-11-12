import { useState } from "react";
import {
  Alert,
  message,
  short,
  time,
  useSession,
  value,
} from "@piggybank/web-core";
import type { Cases } from "../types";
import { stableKey, doneKey } from "../lib/navigation";
import { PageTitle, StateBadge } from "../lib/display";

export function SupportView({
  cases,
  changed,
}: {
  cases: Cases;
  changed: () => Promise<void>;
}) {
  const { client } = useSession();
  const [content, setContent] = useState<Record<string, string>>({});
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  return (
    <>
      <PageTitle
        eyebrow="A little help along the way"
        title="Your support space."
        description="Keep track of requests and provide additional information when our team needs it."
      />
      {error ? <Alert>{error}</Alert> : null}
      <section className="card">
        <div className="section-title">
          <h2>Your requests</h2>
          <button className="secondary" onClick={() => void changed()}>
            Refresh requests
          </button>
        </div>
        {cases.length ? (
          <div className="stack">
            {cases.map((item) => (
              <article className="support-item" key={item.id}>
                <div className="spread">
                  <div>
                    <strong>
                      Request{" "}
                      <span className="reference">{short(item.id)}</span>
                    </strong>
                    <p className="hint">{time(item.createdAt)}</p>
                  </div>
                  <StateBadge state={item.state} />
                </div>
                <p className="muted">{item.message}</p>
                {item.state === "INFORMATION_REQUIRED" ? (
                  <form
                    onSubmit={(event) => {
                      event.preventDefault();
                      setBusy(true);
                      const scope = "pb:case-info:" + item.id;
                      void client
                        .POST("/api/v1/cases/{id}/information", {
                          params: { path: { id: item.id } },
                          body: {
                            requestId: stableKey(scope),
                            content: content[item.id] ?? "",
                          },
                        })
                        .then(value)
                        .then(() => {
                          doneKey(scope);
                          return changed();
                        })
                        .catch((e) => setError(message(e)))
                        .finally(() => setBusy(false));
                    }}
                  >
                    <label>
                      Additional synthetic information
                      <textarea
                        required
                        maxLength={32768}
                        value={content[item.id] ?? ""}
                        onChange={(e) =>
                          setContent((old) => ({
                            ...old,
                            [item.id]: e.target.value,
                          }))
                        }
                      />
                    </label>
                    <button className="secondary form-actions" disabled={busy}>
                      Send information
                    </button>
                  </form>
                ) : null}
              </article>
            ))}
          </div>
        ) : (
          <div className="empty">
            You have no support requests. Open an account's details to create a
            request linked to it.
          </div>
        )}
      </section>
    </>
  );
}
