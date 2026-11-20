import { useCallback, useEffect, useRef, useState } from "react";
import {
  Alert,
  Badge,
  message,
  useSession,
  value,
  type Schema,
} from "@piggybank/web-core";
import { stableKey, doneKey } from "../lib/navigation";

const statement =
  "I acknowledge reviewing this exact synthetic preview disclosure. This does not enroll my account, accept financial terms, authorize a charge or interest credit, or activate a policy.";
export function PeriodicReview({
  accountId,
  policyId,
  disclosure,
}: {
  accountId: string;
  policyId: string;
  disclosure?: Schema["PeriodicDisclosure"];
}) {
  const { client } = useSession();
  const [receipt, setReceipt] = useState<Schema["PreviewReviewReceipt"]>(),
    [error, setError] = useState("");
  const [checked, setChecked] = useState(false),
    [loading, setLoading] = useState(true),
    [saving, setSaving] = useState(false);
  const generation = useRef(0),
    scope = useRef(accountId + ":" + policyId);
  const currentScope = accountId + ":" + policyId;
  scope.current = currentScope;
  const valid = useCallback(
    (r: Schema["PreviewReviewReceipt"]) => {
      if (
        r.review.accountId !== accountId ||
        r.review.policyId !== policyId ||
        r.review.material.accountId !== accountId ||
        r.review.material.policy.id !== policyId ||
        r.review.kind !== "PREVIEW_REVIEW_ONLY"
      )
        throw new Error("Saved review does not match this account and rule.");
      return r;
    },
    [accountId, policyId],
  );
  const load = useCallback(async () => {
    const request = ++generation.current;
    setLoading(true);
    try {
      const result = await client.GET(
        "/api/v1/accounts/{id}/periodic-disclosures/{policyId}/review",
        { params: { path: { id: accountId, policyId } } },
      );
      const r =
        result.response.status === 404 ? undefined : valid(await value(result));
      if (request !== generation.current || scope.current !== currentScope)
        return;
      setReceipt(r);
      setError("");
    } catch (e) {
      if (request === generation.current && scope.current === currentScope) {
        setReceipt(undefined);
        setError(message(e));
      }
    } finally {
      if (request === generation.current && scope.current === currentScope)
        setLoading(false);
    }
  }, [client, accountId, policyId, currentScope, valid]);
  useEffect(() => {
    setReceipt(undefined);
    setChecked(false);
    void load();
    return () => {
      generation.current++;
    };
  }, [load]);
  useEffect(() => {
    setChecked(false);
  }, [disclosure?.digest]);
  async function save() {
    if (!checked || !disclosure || saving || loading) return;
    const shown = disclosure.digest,
      request = ++generation.current,
      keyScope = "pb:periodic-review:" + currentScope + ":" + shown;
    setSaving(true);
    setError("");
    try {
      const r = valid(
        await value(
          await client.POST(
            "/api/v1/accounts/{id}/periodic-disclosures/{policyId}/review",
            {
              params: {
                path: { id: accountId, policyId },
                header: { "Idempotency-Key": stableKey(keyScope) },
              },
              body: { disclosureDigest: shown, acknowledged: true },
            },
          ),
        ),
      );
      if (r.review.materialDigest !== shown)
        throw new Error("Saved material differs from the reviewed preview.");
      if (request !== generation.current || scope.current !== currentScope)
        return;
      doneKey(keyScope);
      setReceipt(r);
      setChecked(false);
    } catch (e) {
      if (request === generation.current && scope.current === currentScope) {
        setReceipt(undefined);
        setError(message(e));
      }
    } finally {
      if (request === generation.current && scope.current === currentScope)
        setSaving(false);
    }
  }
  return (
    <section className="periodic-review" aria-label="Saved preview review">
      <div className="spread">
        <h3>Review acknowledgment</h3>
        <button
          className="quiet"
          disabled={saving || loading}
          onClick={() => void load()}
        >
          Refresh saved review
        </button>
      </div>
      <p>
        A saved review retains the exact information you saw. Saving this review
        does not authorize financial enrollment or charges.
      </p>
      {error && <Alert>{error}</Alert>}
      {receipt ? (
        <article aria-label="Original preview review receipt">
          <Badge>Preview review saved</Badge>
          <p>{receipt.review.statement}</p>
          <dl className="details">
            <dt>Review reference</dt>
            <dd data-testid="periodic-review-id">{receipt.review.id}</dd>
            <dt>Reviewed at (UTC)</dt>
            <dd data-testid="periodic-reviewed-at">
              {receipt.review.reviewedAt}
            </dd>
            <dt>Reviewed material digest</dt>
            <dd>{receipt.review.materialDigest}</dd>
            <dt>Receipt digest</dt>
            <dd data-testid="periodic-review-digest">{receipt.digest}</dd>
            <dt>Attribution digest</dt>
            <dd>{receipt.review.attributionDigest}</dd>
          </dl>
          <details>
            <summary>Exact saved preview material</summary>
            <p>
              This is the retained historical material. It does not establish
              current policy validity, enrollment or a financial receipt.
            </p>
            <div
              className="disclosure-text"
              data-testid="periodic-review-material"
            >
              {receipt.review.material.text}
            </div>
          </details>
        </article>
      ) : disclosure ? (
        <>
          <label className="review-checkbox">
            <input
              type="checkbox"
              checked={checked}
              disabled={saving || loading}
              onChange={(e) => setChecked(e.target.checked)}
            />
            <span>{statement}</span>
          </label>
          <button
            disabled={!checked || saving || loading}
            onClick={() => void save()}
          >
            {saving ? "Saving review…" : "Save review acknowledgment"}
          </button>
        </>
      ) : (
        <p className="muted">
          Open a verified preview to save a new review. A previously saved
          review can still be retrieved here.
        </p>
      )}
    </section>
  );
}
