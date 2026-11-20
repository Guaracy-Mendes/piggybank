import { useCallback, useEffect, useRef, useState } from "react";
import { Alert, Badge, message, useSession, value } from "@piggybank/web-core";

/* UI-03 local template: prove you own the email held by the sign-in service with a 6-digit code. Delivery is
   simulated (a local synthetic outbox); no real email or SMS is sent. */
type State = {
  destination: string;
  verified: boolean;
  pending?: {
    expiresAt: string;
    resendAfter: string;
    attemptsLeft: number;
    expired: boolean;
    locked: boolean;
  } | null;
  result?: string;
  attemptsLeft?: number;
};
const path = { params: { path: { trust: "customer" as const } } };
const reasons: Record<number, string> = {
  409: "This email is already verified, or it changed since the code was sent.",
  422: "No email is held for this sign-in.",
  429: "Please wait before asking for another code.",
};

export function ContactVerification() {
  const { client } = useSession();
  const [state, setState] = useState<State>();
  const [digits, setDigits] = useState<string[]>(Array(6).fill(""));
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [busy, setBusy] = useState(false);
  const [now, setNow] = useState(() => Date.now());
  const boxes = useRef<(HTMLInputElement | null)[]>([]);
  const fail = (e: unknown) => {
    const status = (e as { status?: number })?.status;
    setError((status && reasons[status]) || message(e));
  };
  const load = useCallback(
    () =>
      client
        .GET("/auth/{trust}/contact-verification", path)
        .then(value)
        .then((s) => setState(s as State))
        .catch(fail),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [client],
  );
  useEffect(() => {
    void load();
  }, [load]);
  useEffect(() => {
    const t = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(t);
  }, []);
  if (!state) return null;
  const pending = state.pending ?? undefined;
  const wait = pending
    ? Math.max(0, Math.ceil((Date.parse(pending.resendAfter) - now) / 1000))
    : 0;
  const usable = pending && !pending.expired && !pending.locked;
  async function send() {
    setBusy(true);
    setError("");
    setNotice("");
    try {
      const s = await client
        .POST("/auth/{trust}/contact-verification", path)
        .then(value);
      setState(s as State);
      setDigits(Array(6).fill(""));
      setNotice(
        `Code sent to ${state!.destination} (simulated outbox). It expires in 10 minutes.`,
      );
      boxes.current[0]?.focus();
    } catch (e) {
      fail(e);
    } finally {
      setBusy(false);
    }
  }
  async function confirm(code: string) {
    setBusy(true);
    setError("");
    setNotice("");
    try {
      const s = (await client
        .POST("/auth/{trust}/contact-verification/confirm", {
          ...path,
          body: { code },
        })
        .then(value)) as State;
      setState(s);
      if (s.result === "VERIFIED") setNotice("Email verified.");
      else if (s.result === "MISMATCH")
        setError(`That code did not match. ${s.attemptsLeft} attempt(s) left.`);
      else if (s.result === "LOCKED")
        setError("Too many attempts. Ask for a new code.");
      else setError("That code has expired. Ask for a new code.");
      if (s.result !== "VERIFIED") setDigits(Array(6).fill(""));
    } catch (e) {
      fail(e);
    } finally {
      setBusy(false);
    }
  }
  function enter(i: number, raw: string) {
    const typed = raw.replace(/\D/g, "");
    const next = [...digits];
    if (typed.length > 1) {
      // Paste or one-time-code autofill: spread the digits across the boxes.
      typed
        .slice(0, 6 - i)
        .split("")
        .forEach((d, k) => (next[i + k] = d));
    } else next[i] = typed;
    setDigits(next);
    const filled = next.findIndex((d) => !d);
    if (filled === -1) void confirm(next.join(""));
    else if (typed) boxes.current[Math.min(filled, 5)]?.focus();
  }
  return (
    <section className="card" aria-labelledby="contact-verification-heading">
      <h2 id="contact-verification-heading">Verify your email</h2>
      {error && <Alert>{error}</Alert>}
      {notice && (
        <p className="notice" role="status">
          {notice}
        </p>
      )}
      {state.verified ? (
        <p>
          <strong>{state.destination}</strong>{" "}
          <Badge tone="good">Verified</Badge>
        </p>
      ) : (
        <>
          <p>
            We send a 6-digit code to <strong>{state.destination}</strong>, the
            email held by the sign-in service.
          </p>
          <p className="muted">
            Local demo: the code is delivered to a simulated outbox, not a real
            inbox. No email or SMS is sent.
          </p>
          {usable && (
            <form
              onSubmit={(e) => {
                e.preventDefault();
                if (digits.every(Boolean)) void confirm(digits.join(""));
              }}
            >
              <fieldset className="otp" disabled={busy}>
                <legend>Verification code</legend>
                {digits.map((d, i) => (
                  <input
                    key={i}
                    ref={(el) => {
                      boxes.current[i] = el;
                    }}
                    aria-label={`Digit ${i + 1} of 6`}
                    inputMode="numeric"
                    autoComplete={i === 0 ? "one-time-code" : "off"}
                    maxLength={i === 0 ? 6 : 1}
                    value={d}
                    onChange={(e) => enter(i, e.target.value)}
                    onKeyDown={(e) => {
                      if (e.key === "Backspace" && !d && i > 0)
                        boxes.current[i - 1]?.focus();
                    }}
                  />
                ))}
              </fieldset>
              <p className="muted">
                {pending!.attemptsLeft} attempt(s) left · expires{" "}
                {new Date(pending!.expiresAt).toLocaleTimeString()}
              </p>
              <button disabled={busy || !digits.every(Boolean)}>
                Verify code
              </button>
            </form>
          )}
          <button
            className="secondary"
            disabled={busy || wait > 0}
            onClick={() => void send()}
          >
            {pending
              ? wait > 0
                ? `Resend code in ${wait}s`
                : "Resend code"
              : "Send code"}
          </button>
        </>
      )}
    </section>
  );
}
