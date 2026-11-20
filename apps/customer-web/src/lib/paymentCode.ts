/* Synthetic TST payment codes shared as text or QR. A code only carries a receiving reference and, for a request,
   the asked amount and memo; it never authorizes anything. Paying still needs a recipient, a quote and issuer
   approval. Format: pbtst:pay?ref=PB…[&amount=<minor units>][&memo=<text>] */
export type PaymentCode = {
  reference: string;
  amountMinor?: string;
  memo?: string;
};
const REFERENCE = /^(PB|SIM)[0-9A-F]{32}$/;
const PREFIX = "pbtst:pay?";

export function encodePaymentCode(code: PaymentCode): string {
  const reference = code.reference.trim().toUpperCase();
  if (!REFERENCE.test(reference))
    throw new Error("Invalid receiving reference");
  const q = new URLSearchParams({ ref: reference });
  if (code.amountMinor !== undefined) {
    if (!/^[1-9][0-9]{0,15}$/.test(code.amountMinor))
      throw new Error("Invalid amount");
    q.set("amount", code.amountMinor);
  }
  if (code.memo) q.set("memo", code.memo.slice(0, 120));
  return PREFIX + q.toString();
}

/** Accepts a bare receiving reference or a pbtst:pay code; anything else is rejected rather than guessed. */
export function parsePaymentCode(text: string): PaymentCode | null {
  const raw = text.trim();
  if (REFERENCE.test(raw.toUpperCase()))
    return { reference: raw.toUpperCase() };
  if (!raw.startsWith(PREFIX) || raw.length > 400) return null;
  const q = new URLSearchParams(raw.slice(PREFIX.length));
  const keys = [...q.keys()];
  if (keys.some((k) => !["ref", "amount", "memo"].includes(k))) return null;
  if (new Set(keys).size !== keys.length) return null;
  const reference = (q.get("ref") ?? "").toUpperCase();
  if (!REFERENCE.test(reference)) return null;
  const out: PaymentCode = { reference };
  const amountMinor = q.get("amount");
  if (amountMinor !== null) {
    if (!/^[1-9][0-9]{0,15}$/.test(amountMinor)) return null;
    out.amountMinor = amountMinor;
  }
  const memo = q.get("memo");
  if (memo !== null) {
    if (memo.length > 120) return null;
    out.memo = memo;
  }
  return out;
}
