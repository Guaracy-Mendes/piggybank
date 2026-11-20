import { useCallback, useEffect, useState } from "react";
import { api, tst, value } from "../api";
import { Banner, Busy, Button, Card, Line, Title } from "../ui";

type Version = { id: string; version: number; disposed: boolean; closingMinor?: string };
type Period = { periodStart: string; readiness: { ready: boolean; reasons: string[] }; versions: Version[] };
type Docs = { accounts: { accountId: string; periods: Period[] }[] };
type Opened = { document: { periodStart: string; version: number; openingMinor: string; closingMinor: string; totals: { creditsMinor: string; debitsMinor: string } }; contentDigest: string; supersededBy: string | null };
const month = (d: string) => new Date(d + "T00:00:00Z").toLocaleDateString("en", { month: "long", year: "numeric", timeZone: "UTC" });

/* Statements: each view uses a fresh two-minute single-use link bound to this person. */
export function Documents({ refreshKey }: { refreshKey: number }) {
  const [docs, setDocs] = useState<Docs>();
  const [opened, setOpened] = useState<Opened>();
  const [error, setError] = useState("");
  const load = useCallback(async () => {
    try {
      setDocs((await api.GET("/api/v1/documents").then(value)) as unknown as Docs);
    } catch (e) {
      setError((e as Error).message);
    }
  }, []);
  useEffect(() => {
    void load();
  }, [load, refreshKey]);
  async function view(id: string) {
    setError("");
    try {
      const link = (await api.POST("/api/v1/documents/statements/{id}/links", { params: { path: { id } } }).then(value)) as unknown as { token: string };
      setOpened((await api.GET("/api/v1/documents/downloads/{id}", { params: { path: { id: link.token }, query: { format: "json" } } }).then(value)) as unknown as Opened);
    } catch (e) {
      setError((e as Error).message);
    }
  }
  if (!docs) return error ? <Banner tone="error" text={error} /> : <Busy label="Loading statements" />;
  return (
    <>
      <Title sub="Monthly statements, every version kept.">Statements</Title>
      {error ? <Banner tone="error" text={error} /> : null}
      {opened ? (
        <Card label="Opened statement">
          <Line strong>
            {month(opened.document.periodStart)} · version {opened.document.version}
          </Line>
          <Line>Opening {tst(opened.document.openingMinor)}</Line>
          <Line>Money in {tst(opened.document.totals.creditsMinor)}</Line>
          <Line>Money out {tst(opened.document.totals.debitsMinor)}</Line>
          <Line strong>Closing {tst(opened.document.closingMinor)}</Line>
          <Line muted>{opened.supersededBy ? "Replaced by a newer version" : "Current version"}</Line>
          <Button quiet label="Close statement" onPress={() => setOpened(undefined)} />
        </Card>
      ) : null}
      {docs.accounts.map((a) =>
        a.periods
          .filter((p) => !p.readiness.reasons.includes("ACCOUNT_NOT_OPEN_IN_PERIOD"))
          .map((p) => {
            const v = p.versions[0];
            return (
              <Card key={a.accountId + p.periodStart} label={month(p.periodStart)}>
                <Line strong>{month(p.periodStart)}</Line>
                {v && !v.disposed ? (
                  <>
                    <Line>Closing {tst(v.closingMinor)}</Line>
                    <Button quiet label={"View " + month(p.periodStart)} onPress={() => void view(v.id)} />
                  </>
                ) : (
                  <Line muted>{p.readiness.reasons.includes("PERIOD_OPEN") ? "Available after the month closes." : p.readiness.ready ? "Not created yet." : "Waiting for month-end processing."}</Line>
                )}
              </Card>
            );
          }),
      )}
    </>
  );
}
