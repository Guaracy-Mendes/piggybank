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
import { FinanceBases } from "./FinanceBases";
type Record = Schema["PeriodicPolicyRequest"];
const addDays = (date: string, days: number) => {
  const result = new Date(date + "T00:00:00Z");
  result.setUTCDate(result.getUTCDate() + days);
  return result.toISOString().slice(0, 10);
};
export function FinancePolicies({
  access,
  selected,
  basisId,
}: {
  access: Access;
  selected?: string;
  basisId?: string;
}) {
  const { client } = useSession();
  const generation = useRef(0);
  const selection = useRef(selected);
  selection.current = selected;
  const [items, setItems] = useState<Record[]>([]),
    [detail, setDetail] = useState<Record>();
  const [error, setError] = useState(""),
    [notice, setNotice] = useState(""),
    [busy, setBusy] = useState(false);
  const [reviewed, setReviewed] = useState<string>(),
    [preview, setPreview] = useState<Schema["PeriodicPreview"]>();
  const [kind, setKind] = useState<
    "MONTHLY_FIXED_FEE" | "DAILY_CREDIT_ACCRUAL"
  >("MONTHLY_FIXED_FEE");
  const load = useCallback(async () => {
    const request = ++generation.current;
    try {
      const [records, record] = await Promise.all([
        client.GET("/api/v1/staff/finance/periodic-policies").then(value),
        selected
          ? client
              .GET("/api/v1/staff/finance/periodic-policies/{id}", {
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
    } catch (e) {
      if (request === generation.current && selection.current === selected)
        setError(message(e));
    }
  }, [client, selected]);
  useEffect(() => {
    setDetail(undefined);
    setPreview(undefined);
    setReviewed(undefined);
    void load();
  }, [load]);
  async function propose(form: HTMLFormElement) {
    setBusy(true);
    setError("");
    setNotice("");
    try {
      const fields = new FormData(form),
        get = (name: string) => String(fields.get(name) ?? "");
      const until = get("until"),
        from = get("from"),
        version = Number(get("version"));
      const calculation: Schema["PeriodicCalculation"] =
        kind === "MONTHLY_FIXED_FEE"
          ? {
              kind,
              amount: {
                minorUnits: get("amount"),
                currency: { code: "TST", scale: 2 },
              },
            }
          : {
              kind,
              annualBasisPoints: Number(get("rate")),
              dayCount: "ACT_365_FIXED",
              basis: "NONNEGATIVE_CLOSING_BOOKED",
              rounding: "HALF_UP_EACH_DAY",
            };
      const material = {
        code: get("code"),
        version,
        label: "SYNTHETIC" as const,
        activation: "PREVIEW_ONLY" as const,
        productVersionId: get("product"),
        effectiveFrom: from,
        effectiveUntil: until,
        calendar: {
          code: get("calendar"),
          version,
          timezone: "UTC" as const,
          coverageFrom: from,
          coverageUntil: addDays(until, 15),
          businessWeekdays: get("weekdays").split(",").map(Number),
          holidays: get("holidays").split(/\s+/).filter(Boolean).sort(),
          cutoff:
            get("cutoff").length === 5 ? get("cutoff") + ":00" : get("cutoff"),
          rollConvention: "FOLLOWING_14_DAYS" as const,
        },
        calculation,
      };
      const reason = get("reason");
      const bytes = new Uint8Array(
        await crypto.subtle.digest(
          "SHA-256",
          new TextEncoder().encode(JSON.stringify({ material, reason })),
        ),
      );
      const scope =
        "pb:periodic-policy:" +
        access.personId +
        ":" +
        Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("");
      const saved = sessionStorage.getItem(scope);
      const ids: { key: string; policy: string; calendar: string } = saved
        ? JSON.parse(saved)
        : {
            key: crypto.randomUUID(),
            policy: crypto.randomUUID(),
            calendar: crypto.randomUUID(),
          };
      sessionStorage.setItem(scope, JSON.stringify(ids));
      const result = await client
        .POST("/api/v1/staff/finance/periodic-policies", {
          params: { header: { "Idempotency-Key": ids.key } },
          body: {
            policy: {
              ...material,
              id: ids.policy,
              calendar: { ...material.calendar, id: ids.calendar },
            },
            reason,
          },
        })
        .then(value);
      location.hash = "/finance/policies/" + result.id;
      setNotice(
        "Policy request saved. Independent approval and publication are required for previews.",
      );
      if (selection.current === result.id) await load();
    } catch (e) {
      setError(message(e));
    } finally {
      setBusy(false);
    }
  }
  async function act(action: "approval" | "publication") {
    if (!detail) return;
    setBusy(true);
    setError("");
    setNotice("");
    try {
      const options = {
        params: { path: { id: detail.id } },
        body: { detailsDigest: detail.policyDigest },
      };
      const result =
        action === "approval"
          ? await client
              .POST(
                "/api/v1/staff/finance/periodic-policies/{id}/approval",
                options,
              )
              .then(value)
          : await client
              .POST(
                "/api/v1/staff/finance/periodic-policies/{id}/publication",
                options,
              )
              .then(value);
      if (selection.current === result.id) setDetail(result);
      setNotice(
        action === "approval"
          ? "Independent policy approval recorded."
          : "Preview policy published. Customer charges remain disabled.",
      );
      await load();
    } catch (e) {
      setError(message(e));
    } finally {
      setBusy(false);
    }
  }
  async function calculate(form: HTMLFormElement) {
    if (!detail) return;
    setBusy(true);
    setError("");
    setPreview(undefined);
    try {
      const fields = new FormData(form),
        basis = String(fields.get("basis") ?? "");
      setPreview(
        await client
          .POST("/api/v1/staff/finance/periodic-policies/{id}/preview", {
            params: { path: { id: detail.id } },
            body: {
              periodStart: String(fields.get("period")),
              evaluatedAt: String(fields.get("clock")),
              closingBooked:
                detail.policy.calculation.kind === "DAILY_CREDIT_ACCRUAL"
                  ? { minorUnits: basis, currency: { code: "TST", scale: 2 } }
                  : null,
            },
          })
          .then(value),
      );
    } catch (e) {
      setError(message(e));
    } finally {
      setBusy(false);
    }
  }
  return (
    <>
      <div className="page-title">
        <div>
          <p className="eyebrow">PERIODIC ACCOUNTING</p>
          <h1>Review the rule before a charge.</h1>
          <p>
            Versioned synthetic policies, explicit business calendars and exact
            calculation previews.
          </p>
        </div>
        <button className="secondary" onClick={() => void load()}>
          Refresh policies
        </button>
      </div>
      <p className="notice">
        Preview only. These policies do not enroll accounts or post fees or
        interest. Manual previews use operator supplied balances. Native bases
        capture Core journal evidence.
      </p>
      {error && <Alert>{error}</Alert>}
      {notice && (
        <p role="status" className="notice">
          {notice}
        </p>
      )}
      <div className="queue-layout">
        <section className="panel">
          <h2>Policy register</h2>
          <p>
            Most recent 100 requests. Each publication preserves its original
            calendar and calculation.
          </p>
          {!items.length && <p>No periodic policies have been requested.</p>}
          {items.map((r) => (
            <a
              className="case-row"
              key={r.id}
              href={"#/finance/policies/" + r.id}
              aria-current={r.id === selected ? "page" : undefined}
            >
              <div>
                <strong>
                  {r.policy.code} · Version {r.policy.version}
                </strong>
                <Badge>{r.state}</Badge>
              </div>
              <p>
                {r.policy.calculation.kind === "MONTHLY_FIXED_FEE"
                  ? "Monthly fixed fee"
                  : "Daily credit accrual"}{" "}
                · {r.policy.effectiveFrom} to {r.policy.effectiveUntil}
              </p>
              <small>
                {short(r.id)} · {time(r.createdAt)}
              </small>
            </a>
          ))}
        </section>
        {detail && detail.id === selected && (
          <section className="panel" aria-label="Selected periodic policy">
            <h2>Review periodic policy</h2>
            <Badge>{detail.state}</Badge>
            <p>{detail.reason}</p>
            <dl className="evidence-details">
              <dt>Rule</dt>
              <dd>
                {detail.policy.code} · Version {detail.policy.version} ·{" "}
                {detail.policy.activation}
              </dd>
              <dt>Product version</dt>
              <dd className="mono">{detail.policy.productVersionId}</dd>
              <dt>Effective period</dt>
              <dd>
                {detail.policy.effectiveFrom} inclusive to{" "}
                {detail.policy.effectiveUntil} exclusive
              </dd>
              <dt>Calculation</dt>
              <dd>
                {detail.policy.calculation.kind === "MONTHLY_FIXED_FEE"
                  ? `${money(detail.policy.calculation.amount)} per complete calendar month`
                  : `${detail.policy.calculation.annualBasisPoints} annual basis points · ACT/365 fixed · nonnegative closing booked balance · half up each day`}
              </dd>
              <dt>Calendar snapshot</dt>
              <dd>
                {detail.policy.calendar.code} · Version{" "}
                {detail.policy.calendar.version} · {detail.policy.calendar.id}
              </dd>
              <dt>Calendar coverage</dt>
              <dd>
                {detail.policy.calendar.coverageFrom} inclusive to{" "}
                {detail.policy.calendar.coverageUntil} exclusive
              </dd>
              <dt>Business weekdays</dt>
              <dd>
                {detail.policy.calendar.businessWeekdays.join(", ")} · ISO
                Monday = 1
              </dd>
              <dt>Holidays</dt>
              <dd>
                {detail.policy.calendar.holidays.join(", ") || "None declared"}
              </dd>
              <dt>Due rule</dt>
              <dd>
                {detail.policy.calendar.cutoff} UTC on the first business day on
                or after period end, within 14 days. Equality at cutoff is due.
              </dd>
              <dt>Policy digest</dt>
              <dd className="mono">{detail.policyDigest}</dd>
              <dt>Canonical maker</dt>
              <dd className="mono">{detail.makerPerson}</dd>
              <dt>Canonical checker</dt>
              <dd className="mono">{detail.checkerPerson ?? "Pending"}</dd>
              <dt>Approval expires</dt>
              <dd>{time(detail.expiresAt)}</dd>
              <dt>Registration</dt>
              <dd>{detail.registrationReason}</dd>
              {detail.publication && (
                <>
                  <dt>Published</dt>
                  <dd>{time(detail.publication.publishedAt)}</dd>
                </>
              )}
            </dl>
            {access.capabilities.includes("finance.approve") &&
              detail.makerPerson !== access.personId &&
              ["STAGED", "PENDING", "APPROVED"].includes(detail.state) && (
                <>
                  <label className="revision-review">
                    <input
                      type="checkbox"
                      checked={reviewed === detail.policyDigest}
                      onChange={(e) =>
                        setReviewed(
                          e.target.checked ? detail.policyDigest : undefined,
                        )
                      }
                    />
                    I reviewed this policy, calendar and digest.
                  </label>
                  <button
                    disabled={busy || reviewed !== detail.policyDigest}
                    onClick={() => void act("approval")}
                  >
                    Approve preview policy
                  </button>
                </>
              )}
            {access.capabilities.includes("finance.recover") &&
              detail.makerPerson === access.personId &&
              detail.state === "APPROVED" && (
                <button disabled={busy} onClick={() => void act("publication")}>
                  Publish preview policy
                </button>
              )}
            {detail.publication && (
              <form
                className="configuration-form"
                onSubmit={(e) => {
                  e.preventDefault();
                  void calculate(e.currentTarget);
                }}
                key={detail.id}
              >
                <h3 className="wide">Calculate an exact preview</h3>
                <label htmlFor="preview-period">Period start</label>
                <input
                  id="preview-period"
                  name="period"
                  type="date"
                  defaultValue={detail.policy.effectiveFrom}
                  required
                />
                {detail.policy.calculation.kind === "DAILY_CREDIT_ACCRUAL" && (
                  <>
                    <label htmlFor="preview-basis">
                      Closing booked minor units
                    </label>
                    <input
                      id="preview-basis"
                      name="basis"
                      inputMode="numeric"
                      pattern="0|[1-9][0-9]{0,18}"
                      maxLength={19}
                      defaultValue="0"
                      required
                    />
                    <p className="wide">
                      Operator supplied TST minor units; this is not a captured
                      Core account balance.
                    </p>
                  </>
                )}
                <label htmlFor="preview-clock">Evaluate at UTC</label>
                <input
                  id="preview-clock"
                  name="clock"
                  type="text"
                  defaultValue={new Date().toISOString()}
                  required
                />
                <button disabled={busy}>Calculate preview</button>
              </form>
            )}
            {preview && preview.policyId === detail.policy.id && (
              <div role="status" className="notice">
                <h3>Calculation preview</h3>
                <dl className="evidence-details">
                  <dt>Amount</dt>
                  <dd>{money(preview.amount)}</dd>
                  <dt>Exact minor units</dt>
                  <dd>{preview.amount.minorUnits}</dd>
                  <dt>Period</dt>
                  <dd>
                    {preview.periodStart} inclusive to {preview.periodEnd}{" "}
                    exclusive
                  </dd>
                  <dt>Due at (UTC)</dt>
                  <dd>{preview.dueAt}</dd>
                  <dt>Evaluated at (UTC)</dt>
                  <dd>{preview.evaluatedAt}</dd>
                  <dt>Due at evaluated time</dt>
                  <dd>{preview.isDue ? "Yes" : "No"}</dd>
                  <dt>Exact fraction</dt>
                  <dd>
                    {preview.exactNumerator} / {preview.exactDenominator}
                  </dd>
                  <dt>Basis source</dt>
                  <dd>Operator supplied preview only</dd>
                  <dt>Policy digest</dt>
                  <dd className="mono">{preview.policyDigest}</dd>
                </dl>
              </div>
            )}
          </section>
        )}
      </div>
      {detail?.publication && detail.id === selected && (
        <FinanceBases
          key={detail.id}
          access={access}
          request={detail}
          selected={basisId}
        />
      )}
      {access.capabilities.includes("finance.recover") && (
        <section className="panel">
          <h2>Propose a preview policy</h2>
          <p>
            Versions use disjoint effective periods. A later version starts in
            the future and cannot rewrite an earlier period. Calendars are
            frozen within each policy.
          </p>
          <form
            className="configuration-form"
            onSubmit={(e) => {
              e.preventDefault();
              void propose(e.currentTarget);
            }}
          >
            <label htmlFor="policy-product">Product version ID</label>
            <input
              id="policy-product"
              name="product"
              pattern="[a-f0-9-]{36}"
              required
            />
            <label htmlFor="policy-code">Rule code</label>
            <input
              id="policy-code"
              name="code"
              pattern="[A-Z][A-Z0-9_]{0,31}"
              maxLength={32}
              required
            />
            <label htmlFor="policy-version">Policy version</label>
            <input
              id="policy-version"
              name="version"
              type="number"
              min="1"
              max="2147483647"
              defaultValue="1"
              required
            />
            <label htmlFor="policy-from">Effective from</label>
            <input id="policy-from" name="from" type="date" required />
            <label htmlFor="policy-until">Effective until, exclusive</label>
            <input id="policy-until" name="until" type="date" required />
            <label htmlFor="policy-kind">Calculation kind</label>
            <select
              id="policy-kind"
              value={kind}
              onChange={(e) => setKind(e.target.value as typeof kind)}
            >
              <option value="MONTHLY_FIXED_FEE">Monthly fixed fee</option>
              <option value="DAILY_CREDIT_ACCRUAL">Daily credit accrual</option>
            </select>
            {kind === "MONTHLY_FIXED_FEE" ? (
              <>
                <label htmlFor="policy-amount">Monthly fee minor units</label>
                <input
                  id="policy-amount"
                  name="amount"
                  inputMode="numeric"
                  pattern="[1-9][0-9]{0,18}"
                  maxLength={19}
                  required
                />
                <p className="wide">
                  Whole calendar months only; no proration.
                </p>
              </>
            ) : (
              <>
                <label htmlFor="policy-rate">Annual basis points</label>
                <input
                  id="policy-rate"
                  name="rate"
                  type="number"
                  min="1"
                  max="10000"
                  required
                />
                <p className="wide">
                  Actual days / fixed 365, nonnegative closing booked basis,
                  half up in minor units each day.
                </p>
              </>
            )}
            <label htmlFor="policy-calendar">Calendar code</label>
            <input
              id="policy-calendar"
              name="calendar"
              pattern="[A-Z][A-Z0-9_]{0,31}"
              defaultValue="SYNTHETIC_UTC"
              required
            />
            <label htmlFor="policy-weekdays">
              Business weekdays, comma separated
            </label>
            <input
              id="policy-weekdays"
              name="weekdays"
              pattern="[1-7](,[1-7]){0,6}"
              defaultValue="1,2,3,4,5"
              required
            />
            <p className="wide">ISO Monday = 1; ascending and unique.</p>
            <label htmlFor="policy-holidays">
              Holidays, one YYYY-MM-DD per line
            </label>
            <textarea
              id="policy-holidays"
              name="holidays"
              rows={3}
              maxLength={4026}
            />
            <label htmlFor="policy-cutoff">Cutoff in UTC</label>
            <input
              id="policy-cutoff"
              name="cutoff"
              type="time"
              step="1"
              defaultValue="17:00:00"
              required
            />
            <p className="wide">
              Calendar coverage begins at effective from and ends 15 days after
              effective until. Period end rolls forward to the first declared
              business day within 14 days.
            </p>
            <label htmlFor="policy-reason">Reason for policy</label>
            <textarea
              id="policy-reason"
              name="reason"
              minLength={4}
              maxLength={240}
              required
              rows={3}
            />
            <button disabled={busy}>Save preview policy request</button>
          </form>
        </section>
      )}
    </>
  );
}
