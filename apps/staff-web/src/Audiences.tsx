import { useEffect, useState } from "react";
import {
  Alert,
  Badge,
  message,
  short,
  time,
  useSession,
  value,
} from "@piggybank/web-core";

/* WP-38 audiences and the local journey-partner simulator role: consent-checked builds, pseudonymous exports and
   outcome reports with explicit denominators and late-data flags. */
type Report = {
  id: string;
  status: "COMPLETE" | "INCOMPLETE";
  denominators: {
    candidates: number;
    members: number;
    exported: number;
    withdrawnSinceBuild: number;
  };
  outcomes: {
    creditedAfterBuild: number;
    insightsCreated: number;
    insightsDismissed: number;
    partnerRecommendationsAccepted: number;
  };
  completeness: { code: string }[];
  createdAt: string;
};
type Audience = {
  id: string;
  segment: string;
  segmentVersion: number;
  purpose: string;
  candidates: number;
  members: number;
  excluded: { noMarketingConsent?: number; restricted?: number };
  builtAt: string;
  exports: {
    id: string;
    partner: string;
    exported: number;
    withdrawnExcluded: number;
    deliveries: { delivered: number; withheld: number }[];
  }[];
  reports: Report[];
};
type View = {
  segments: {
    code: string;
    version: number;
    purpose: string;
    definition: string;
  }[];
  partners: { code: string; status: string; description: string }[];
  audiences: Audience[];
  partnerMessages: {
    messageId: string;
    result: string;
    reason: string | null;
    receivedAt: string;
  }[];
};
const words = (code: string) => code.toLowerCase().replaceAll("_", " ");

export function AudiencesView() {
  const { client } = useSession();
  const [view, setView] = useState<View | null>(null);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const load = () =>
    client
      .GET("/api/v1/staff/audiences")
      .then(value)
      .then((v) => setView(v as unknown as View))
      .catch((e) => setError(message(e)));
  useEffect(() => {
    void load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [client]);
  async function act(
    run: () => Promise<{ data?: unknown; response: Response }>,
    done: (v: Record<string, unknown>) => string,
  ) {
    setError("");
    setNotice("");
    try {
      const r = await run();
      if (r.response.status === 403) {
        setNotice(
          "Request denied and recorded. Check the purpose and the partner's permitted purposes.",
        );
      } else {
        setNotice(done((await value(r)) as Record<string, unknown>));
      }
      void load();
    } catch (e) {
      setError(message(e));
    }
  }
  return (
    <section aria-labelledby="audiences-heading">
      <h1 id="audiences-heading">Audiences & journeys</h1>
      <p className="muted">
        Consent is checked for every member at build, export and delivery.
        Partners get pseudonymous tokens and can only send advice. One local
        partner simulator; real partners need selection (DEC-05/06/09).
      </p>
      {error && <Alert>{error}</Alert>}
      {notice && (
        <p className="notice" role="status">
          {notice}
        </p>
      )}
      {view && (
        <>
          <h2>Segments</h2>
          <ul className="card-list">
            {view.segments.map((s) => (
              <li className="card" key={`${s.code}-${s.version}`}>
                <strong>
                  {s.code} v{s.version}
                </strong>{" "}
                <Badge
                  tone={s.purpose === "MARKETING" ? "attention" : "neutral"}
                >
                  {words(s.purpose)}
                </Badge>
                <p>{s.definition}</p>
                <button
                  onClick={() =>
                    void act(
                      () =>
                        client.POST("/api/v1/staff/audiences", {
                          body: { segment: s.code, version: s.version },
                        }),
                      (v) =>
                        `Audience built: ${String(v.members)} of ${String(v.candidates)} candidates are members.`,
                    )
                  }
                >
                  Build {s.code} audience
                </button>
              </li>
            ))}
          </ul>
          <h2>Audiences</h2>
          {view.audiences.length === 0 ? (
            <p className="muted">No audiences yet.</p>
          ) : (
            <ul className="card-list">
              {view.audiences.map((a) => (
                <li className="card" key={a.id}>
                  <strong>
                    {a.segment} v{a.segmentVersion} · {short(a.id)}
                  </strong>{" "}
                  · {words(a.purpose)} · built {time(a.builtAt)}
                  <p>
                    {a.members} members of {a.candidates} candidates ·{" "}
                    {a.excluded.noMarketingConsent ?? 0} without marketing
                    consent · {a.excluded.restricted ?? 0} restricted
                  </p>
                  {a.exports.map((e) => (
                    <p key={e.id}>
                      Export {short(e.id)} to {e.partner}: {e.exported} tokens,{" "}
                      {e.withdrawnExcluded} withdrawn excluded
                      {e.deliveries.length > 0 &&
                        ` · last delivery ${e.deliveries[e.deliveries.length - 1].delivered} sent, ${e.deliveries[e.deliveries.length - 1].withheld} withheld`}
                    </p>
                  ))}
                  {a.reports.slice(-1).map((r) => (
                    <div key={r.id}>
                      <p>
                        Latest outcome report{" "}
                        <Badge
                          tone={r.status === "COMPLETE" ? "good" : "attention"}
                        >
                          {words(r.status)}
                        </Badge>{" "}
                        · {r.outcomes.creditedAfterBuild} of{" "}
                        {r.denominators.members} members credited after build ·{" "}
                        {r.denominators.withdrawnSinceBuild} withdrew since ·{" "}
                        {r.outcomes.partnerRecommendationsAccepted} partner
                        suggestions accepted
                      </p>
                      {r.completeness.length > 0 && (
                        <p>
                          Flags:{" "}
                          {r.completeness.map((f) => words(f.code)).join(", ")}
                        </p>
                      )}
                    </div>
                  ))}
                  <div className="actions">
                    {view.partners.map((p) => (
                      <button
                        key={p.code}
                        className="secondary"
                        onClick={() =>
                          void act(
                            () =>
                              client.POST(
                                "/api/v1/staff/audiences/{id}/exports",
                                {
                                  params: { path: { id: a.id } },
                                  body: { partner: p.code as "SIM_JOURNEY" },
                                },
                              ),
                            (v) =>
                              `Exported ${String(v.exported)} pseudonymous tokens; ${String(v.withdrawnExcluded)} withdrawn members excluded.`,
                          )
                        }
                      >
                        Export {short(a.id)} to {p.code}
                      </button>
                    ))}
                    <button
                      className="secondary"
                      onClick={() =>
                        void act(
                          () =>
                            client.POST(
                              "/api/v1/staff/audiences/{id}/outcome-reports",
                              { params: { path: { id: a.id } } },
                            ),
                          (v) =>
                            `Outcome report recorded (${words(String(v.status))}).`,
                        )
                      }
                    >
                      Report outcomes for {short(a.id)}
                    </button>
                  </div>
                </li>
              ))}
            </ul>
          )}
          <h2>Partner messages</h2>
          {view.partnerMessages.length === 0 ? (
            <p className="muted">No partner messages yet.</p>
          ) : (
            <ul>
              {view.partnerMessages.map((m) => (
                <li key={m.messageId}>
                  {m.messageId} · {words(m.result)}
                  {m.reason ? ` · ${words(m.reason)}` : ""} ·{" "}
                  {time(m.receivedAt)}
                </li>
              ))}
            </ul>
          )}
        </>
      )}
    </section>
  );
}
