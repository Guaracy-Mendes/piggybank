import { useCallback, useEffect, useRef, useState } from "react";
import {
  Alert,
  Badge,
  message,
  useSession,
  value,
  type Schema,
} from "@piggybank/web-core";
const availability: Record<
  Schema["PeriodicDisclosure"]["availability"],
  string
> = {
  CURRENT_PREVIEW: "Current preview",
  UPCOMING_PREVIEW: "Upcoming preview",
  ENDED_PREVIEW: "Earlier preview",
};
export function PeriodicDisclosures({
  accountId,
  selected,
}: {
  accountId: string;
  selected?: string;
}) {
  const { client } = useSession();
  const [page, setPage] = useState<Schema["PeriodicDisclosurePage"]>(),
    [detail, setDetail] = useState<Schema["PeriodicDisclosure"]>();
  const [error, setError] = useState("");
  const generation = useRef(0),
    scope = useRef(accountId + ":" + (selected ?? ""));
  const currentScope = accountId + ":" + (selected ?? "");
  scope.current = currentScope;
  const load = useCallback(async () => {
    const request = ++generation.current;
    try {
      const [list, item] = await Promise.all([
        client
          .GET("/api/v1/accounts/{id}/periodic-disclosures", {
            params: { path: { id: accountId } },
          })
          .then((result) =>
            result.response.status === 422 ? undefined : value(result),
          ),
        selected
          ? client
              .GET("/api/v1/accounts/{id}/periodic-disclosures/{policyId}", {
                params: { path: { id: accountId, policyId: selected } },
              })
              .then(value)
          : Promise.resolve(undefined),
      ]);
      if (request !== generation.current || scope.current !== currentScope)
        return;
      if (
        (list && list.accountId !== accountId) ||
        (item &&
          (item.material.accountId !== accountId ||
            item.material.policy.id !== selected))
      )
        throw new Error("Disclosure account or rule does not match this page.");
      setPage(list);
      setDetail(item);
      setError(
        list
          ? ""
          : "The complete rule list exceeds the supported limit. A saved rule link can still be read.",
      );
    } catch (e) {
      if (request === generation.current && scope.current === currentScope) {
        setPage(undefined);
        setDetail(undefined);
        setError(message(e));
      }
    }
  }, [client, accountId, selected, currentScope]);
  useEffect(() => {
    setPage(undefined);
    setDetail(undefined);
    void load();
    return () => {
      generation.current++;
    };
  }, [load]);
  return (
    <section
      className="card periodic-disclosures"
      aria-label="Periodic rule previews"
    >
      <div className="spread">
        <div>
          <span className="eyebrow">Understand the rule</span>
          <h2>Periodic rule previews</h2>
        </div>
        <button className="quiet" onClick={() => void load()}>
          Refresh rule previews
        </button>
      </div>
      <p>
        These approved synthetic rules are previews. You are not enrolled by
        reading them, and no fee debit or interest credit is enabled.
      </p>
      {error && <Alert>{error}</Alert>}
      {page && !page.items.length && (
        <p className="muted">
          No current or upcoming periodic rule previews have been published for
          this account product.
        </p>
      )}
      {page && (
        <div className="disclosure-register">
          {page.items.map((item) => (
            <a
              className="disclosure-link"
              key={item.material.policy.id}
              href={
                "#/accounts/" +
                accountId +
                "/periodic/" +
                item.material.policy.id
              }
              aria-current={
                selected === item.material.policy.id ? "page" : undefined
              }
            >
              <strong>{item.material.title}</strong>
              <Badge>{availability[item.availability]}</Badge>
              <span>
                {item.material.policy.code} · Version{" "}
                {item.material.policy.version}
              </span>
              <small>
                {item.material.policy.effectiveFrom} to{" "}
                {item.material.policy.effectiveUntil} exclusive
              </small>
            </a>
          ))}
        </div>
      )}
      {detail &&
        detail.material.accountId === accountId &&
        detail.material.policy.id === selected && (
          <article aria-label="Selected periodic disclosure">
            <h3>{detail.material.title}</h3>
            <Badge>{availability[detail.availability]}</Badge>
            <div
              className="disclosure-text"
              data-testid="periodic-disclosure-text"
            >
              {detail.material.text}
            </div>
            <details>
              <summary>Version and verification</summary>
              <dl className="details">
                <dt>Disclosure version</dt>
                <dd>{detail.material.formatVersion}</dd>
                <dt>Rule reference</dt>
                <dd>{detail.material.policy.id}</dd>
                <dt>Account reference</dt>
                <dd>{detail.material.accountId}</dd>
                <dt>Approved policy digest</dt>
                <dd>{detail.material.policyDigest}</dd>
                <dt>Disclosure digest</dt>
                <dd data-testid="periodic-disclosure-digest">
                  {detail.digest}
                </dd>
                <dt>Observed at (UTC)</dt>
                <dd>{detail.observedAt}</dd>
              </dl>
            </details>
          </article>
        )}
    </section>
  );
}
