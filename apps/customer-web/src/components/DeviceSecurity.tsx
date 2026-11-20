import { useCallback, useEffect, useState } from "react";
import { Alert, message, useSession, value } from "@piggybank/web-core";
import { stableKey, doneKey } from "../lib/navigation";

/* WP-28 lost-device protection: revokes every issuer session (this browser, the app and any other device). */
type Recovery = {
  id: string;
  kind: string;
  channel: string;
  state: string;
  createdAt: string;
  executedAt: string | null;
};
export function DeviceSecurity() {
  const { client } = useSession();
  const [history, setHistory] = useState<Recovery[]>([]);
  const [confirm, setConfirm] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const load = useCallback(async () => {
    try {
      setHistory(
        (await client
          .GET("/api/v1/security/recoveries")
          .then(value)) as unknown as Recovery[],
      );
    } catch (e) {
      setError(message(e));
    }
  }, [client]);
  useEffect(() => {
    void load();
  }, [load]);
  async function revoke() {
    setBusy(true);
    setError("");
    const scope = "pb:device-revocation";
    try {
      await client
        .POST("/api/v1/security/device-revocation", {
          params: { header: { "Idempotency-Key": stableKey(scope) } },
          body: { reason: "Customer signed out every device" },
        })
        .then(value);
      doneKey(scope);
      // This browser's session was revoked too.
      window.location.assign("/");
    } catch (e) {
      setError(message(e));
      setBusy(false);
    }
  }
  return (
    <div>
      <h3>Lost a phone or shared a device?</h3>
      <p className="muted">
        Sign out everywhere ends every session, on this browser, the Piggybank
        app and any other device. You will need your password to sign in again,
        and we send you a security notice.
      </p>
      {error && <Alert>{error}</Alert>}
      <label className="review-checkbox">
        <input
          type="checkbox"
          checked={confirm}
          onChange={(e) => setConfirm(e.target.checked)}
        />
        I understand every device will be signed out
      </label>
      <button
        className="secondary"
        disabled={!confirm || busy}
        onClick={() => void revoke()}
      >
        {busy ? "Signing out…" : "Sign out everywhere"}
      </button>
      {history.length > 0 && (
        <ul>
          {history.slice(0, 5).map((r) => (
            <li key={r.id}>
              {r.kind === "LOST_DEVICE"
                ? "All devices signed out"
                : "Password reset required"}{" "}
              · {r.state.toLowerCase().replaceAll("_", " ")} ·{" "}
              {r.createdAt.slice(0, 10)}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
