import { useCallback, useEffect, useRef, useState } from "react";
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
type Request = Schema["FinanceCorrectionRequest"];
const amount = (minorUnits: string) =>
  money({ minorUnits, currency: { code: "TST", scale: 2 } });
const observation: Record<
  Schema["FinanceCorrectionAttempt"]["observedReason"],
  string
> = {
  EXECUTION_REQUESTED_RECEIPT_NOT_YET_OBSERVED:
    "Execution requested; native receipt not yet observed",
  NATIVE_RECEIPT_OBSERVED: "Native receipt observed",
  NATIVE_RECEIPT_INQUIRY_UNAVAILABLE: "Native receipt inquiry unavailable",
  NATIVE_RECEIPT_NOT_OBSERVED: "Native receipt not observed at this inquiry",
};
export function FinanceCorrections({
  access,
  selected,
}: {
  access: Access;
  selected?: string;
}) {
  const { client } = useSession();
  const [items, setItems] = useState<Request[]>([]),
    [detail, setDetail] = useState<Request>();
  const [error, setError] = useState(""),
    [notice, setNotice] = useState(""),
    [busy, setBusy] = useState(false),
    [reviewed, setReviewed] = useState<string>(),
    [intent, setIntent] = useState<string>();
  const generation = useRef(0),
    selection = useRef(selected);
  selection.current = selected;
  const load = useCallback(async () => {
    const request = ++generation.current;
    try {
      const [records, record] = await Promise.all([
        client.GET("/api/v1/staff/finance/corrections").then(value),
        selected
          ? client
              .GET("/api/v1/staff/finance/corrections/{id}", {
                params: { path: { id: selected } },
              })
              .then(value)
          : Promise.resolve(undefined),
      ]);
      if (request !== generation.current || selection.current !== selected)
        return;
      setItems(records);
      setDetail(record);
      setError("");
      return record;
    } catch (e) {
      if (request === generation.current && selection.current === selected)
        setError(message(e));
    }
  }, [client, selected]);
  useEffect(() => {
    setDetail(undefined);
    setReviewed(undefined);
    setIntent(undefined);
    void load();
  }, [load]);
  async function propose(form: HTMLFormElement) {
    setBusy(true);
    setError("");
    setNotice("");
    try {
      const fields = new FormData(form),
        body = {
          originalBatchId: String(fields.get("original")),
          reason: String(fields.get("reason")),
        };
      const bytes = new Uint8Array(
        await crypto.subtle.digest(
          "SHA-256",
          new TextEncoder().encode(JSON.stringify(body)),
        ),
      );
      const scope =
        "pb:funding-correction:" +
        access.personId +
        ":" +
        Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("");
      const key = sessionStorage.getItem(scope) ?? crypto.randomUUID();
      sessionStorage.setItem(scope, key);
      const result = await client
        .POST("/api/v1/staff/finance/corrections", {
          params: { header: { "Idempotency-Key": key } },
          body,
        })
        .then(value);
      location.hash = "/finance/corrections/" + result.id;
      setNotice(
        "Correction request saved. Review the frozen source and obtain independent approval.",
      );
      if (selection.current === result.id) await load();
    } catch (e) {
      setError(message(e));
    } finally {
      setBusy(false);
    }
  }
  async function act(action: "approval" | "execution") {
    if (!detail) return;
    setBusy(true);
    setError("");
    setNotice("");
    try {
      const options = {
        params: { path: { id: detail.id } },
        body: { detailsDigest: detail.detailsDigest },
      };
      const result =
        action === "approval"
          ? await client
              .POST("/api/v1/staff/finance/corrections/{id}/approval", options)
              .then(value)
          : await client
              .POST("/api/v1/staff/finance/corrections/{id}/execution", options)
              .then(value);
      if (selection.current === result.id) setDetail(result);
      setNotice(
        action === "approval"
          ? "Independent correction approval recorded."
          : "Correction confirmed by the original native Core receipt.",
      );
      await load();
    } catch (e) {
      setError(message(e));
      if (action === "execution") {
        setNotice(
          "The execution response was interrupted or unconfirmed. The saved request shows current native evidence; retain its original reference.",
        );
        const recovered = await load();
        if (recovered?.state === "EXECUTED" && recovered.receipt)
          setNotice(
            "Recovered the original native correction receipt after the interrupted response.",
          );
      }
    } finally {
      setBusy(false);
    }
  }
  return (
    <>
      <div className="page-title">
        <div>
          <p className="eyebrow">CONTROLLED FINANCIAL CORRECTIONS</p>
          <h1>Preserve the source. Record the correction.</h1>
          <p>
            Full reversals of eligible synthetic funding, fees and interest,
            with exact journal lineage and independent approval.
          </p>
        </div>
        <button className="secondary" onClick={() => void load()}>
          Refresh corrections
        </button>
      </div>
      <p className="notice">
        Original synthetic funding and native periodic fee or interest journals
        are eligible. Execution creates one new linked journal and checks
        current authority, restrictions and available funds, including holds.
      </p>
      {error && <Alert>{error}</Alert>}
      {notice && (
        <p className="notice" role="status">
          {notice}
        </p>
      )}
      <div className="queue-layout">
        <section className="panel">
          <h2>Correction register</h2>
          <p>
            Most recent 100 requests. Registration recovery never executes a
            financial correction automatically.
          </p>
          {!items.length && (
            <p>No financial corrections have been requested.</p>
          )}
          {items.map((r) => (
            <a
              className="case-row"
              key={r.id}
              href={"#/finance/corrections/" + r.id}
              aria-current={r.id === selected ? "page" : undefined}
            >
              <div>
                <strong>Funding {short(r.details.originalBatchId)}</strong>
                <Badge>{r.state}</Badge>
              </div>
              <p>{r.details.reason}</p>
              <small>
                {short(r.id)} · {time(r.createdAt)}
              </small>
            </a>
          ))}
        </section>
        {detail && detail.id === selected && (
          <section className="panel" aria-label="Selected financial correction">
            <h2>Review financial correction</h2>
            <Badge>{detail.state}</Badge>
            <p>{detail.details.reason}</p>
            <dl className="evidence-details">
              <dt>Original journal</dt>
              <dd>{detail.details.originalBatchId}</dd>
              <dt>Original operation</dt>
              <dd>{detail.details.sourceOperationId}</dd>
              <dt>Source business date</dt>
              <dd>{detail.details.sourceBusinessDate} · UTC</dd>
              <dt>Source kind</dt>
              <dd>{detail.source.batch.journal.postingKind}</dd>
              <dt>Source digest</dt>
              <dd>{detail.details.sourceDigest}</dd>
              <dt>Native source fingerprint</dt>
              <dd>{detail.details.sourceFingerprint}</dd>
              <dt>Approval digest</dt>
              <dd>{detail.detailsDigest}</dd>
              <dt>Canonical maker</dt>
              <dd>{detail.makerPerson}</dd>
              <dt>Canonical checker</dt>
              <dd>{detail.checkerPerson ?? "Pending"}</dd>
              <dt>Approval expires</dt>
              <dd>{time(detail.expiresAt)}</dd>
              <dt>Registration</dt>
              <dd>{detail.registrationReason}</dd>
            </dl>
            <h3>Exact original and reversing legs</h3>
            {detail.details.legs.map((leg) => (
              <div className="lineage-item" key={leg.accountId}>
                <h4>
                  {leg.accountKind === "CUSTOMER"
                    ? "Customer account"
                    : "Settlement account"}
                </h4>
                <dl className="evidence-details">
                  <dt>Core account</dt>
                  <dd>{leg.accountId}</dd>
                  <dt>Original amount</dt>
                  <dd>{amount(leg.originalMinor)}</dd>
                  <dt>Correction amount</dt>
                  <dd>{amount(leg.reversingMinor)}</dd>
                  <dt>Exact reversing minor units</dt>
                  <dd>{leg.reversingMinor}</dd>
                </dl>
              </div>
            ))}
            {detail.originalExportId && (
              <p>
                <a href={"#/finance/" + detail.originalExportId}>
                  Original GL export
                </a>
              </p>
            )}
            {access.capabilities.includes("finance.approve") &&
              detail.makerPerson !== access.personId &&
              ["STAGED", "PENDING", "APPROVED"].includes(detail.state) && (
                <>
                  <label className="revision-review">
                    <input
                      type="checkbox"
                      checked={reviewed === detail.detailsDigest}
                      onChange={(e) =>
                        setReviewed(
                          e.target.checked ? detail.detailsDigest : undefined,
                        )
                      }
                    />
                    I reviewed the original source, exact reversing legs, reason
                    and digest.
                  </label>
                  <button
                    disabled={busy || reviewed !== detail.detailsDigest}
                    onClick={() => void act("approval")}
                  >
                    Approve financial correction
                  </button>
                </>
              )}
            {access.capabilities.includes("finance.recover") &&
              detail.makerPerson === access.personId &&
              detail.state === "APPROVED" && (
                <>
                  <label className="revision-review">
                    <input
                      type="checkbox"
                      checked={intent === detail.detailsDigest}
                      onChange={(e) =>
                        setIntent(
                          e.target.checked ? detail.detailsDigest : undefined,
                        )
                      }
                    />
                    I intend to reverse this exact synthetic posting and apply
                    the displayed reversing legs to the customer account.
                  </label>
                  <button
                    disabled={busy || intent !== detail.detailsDigest}
                    onClick={() => void act("execution")}
                  >
                    Execute approved correction
                  </button>
                </>
              )}
            {detail.state === "SOURCE_ALREADY_CORRECTED" && (
              <p className="notice">
                This source already has a linked correction. This request cannot
                execute another reversal.
              </p>
            )}
            {detail.receipt && (
              <div className="notice" role="status">
                <h3>Native correction receipt</h3>
                <dl className="evidence-details">
                  <dt>Correction journal</dt>
                  <dd>{detail.receipt.batchId}</dd>
                  <dt>Original execution</dt>
                  <dd>{detail.receipt.executionId}</dd>
                  <dt>Linked original</dt>
                  <dd>{detail.receipt.originalBatchId}</dd>
                  <dt>Business date</dt>
                  <dd>
                    {detail.receipt.financialBatch.journal.businessDate} · UTC
                  </dd>
                  <dt>Receipt time (UTC)</dt>
                  <dd>{detail.receipt.recordedAt}</dd>
                  <dt>Bound approval digest</dt>
                  <dd>{detail.receipt.detailsDigest}</dd>
                </dl>
                {detail.correctionExportId && (
                  <a href={"#/finance/" + detail.correctionExportId}>
                    Correction GL export
                  </a>
                )}
              </div>
            )}
            {!!detail.attempts.length && (
              <>
                <h3>Execution observations</h3>
                <p>
                  Latest 50 observations. A missing receipt at one inquiry does
                  not establish financial failure.
                </p>
                {detail.attempts.map((a) => (
                  <div className="lineage-item" key={a.id}>
                    <p>{observation[a.observedReason]}</p>
                    <small>{time(a.createdAt)}</small>
                  </div>
                ))}
              </>
            )}
          </section>
        )}
      </div>
      {access.capabilities.includes("finance.recover") && (
        <section className="panel">
          <h2>Request a financial correction</h2>
          <p>
            Use the original Core funding or periodic journal reference. Every
            source and amount is captured from Core before approval. A changed
            reason creates a different proposal.
          </p>
          <form
            className="configuration-form"
            onSubmit={(e) => {
              e.preventDefault();
              void propose(e.currentTarget);
            }}
          >
            <label htmlFor="correction-original">
              Original financial journal ID
            </label>
            <input
              id="correction-original"
              name="original"
              pattern="[a-fA-F0-9-]{36}"
              required
            />
            <label htmlFor="correction-reason">Correction reason</label>
            <textarea
              id="correction-reason"
              name="reason"
              minLength={4}
              maxLength={240}
              rows={3}
              required
            />
            <button disabled={busy}>Save financial correction request</button>
          </form>
        </section>
      )}
    </>
  );
}
