import qrcode from "qrcode-generator";

/* Renders the QR matrix as one SVG path built from qrcode-generator's module grid; no generated markup is injected.
   Error correction M with a 4-module quiet zone, as scanners expect. */
export function QrCode({ payload, label }: { payload: string; label: string }) {
  const qr = qrcode(0, "M");
  qr.addData(payload, "Byte");
  qr.make();
  const n = qr.getModuleCount(),
    quiet = 4,
    size = n + quiet * 2;
  let d = "";
  for (let r = 0; r < n; r++)
    for (let c = 0; c < n; c++)
      if (qr.isDark(r, c)) d += `M${c + quiet} ${r + quiet}h1v1h-1z`;
  return (
    <figure className="payment-qr">
      <svg
        role="img"
        aria-label={label}
        viewBox={`0 0 ${size} ${size}`}
        shapeRendering="crispEdges"
        data-testid="payment-qr"
        data-payload={payload}
        data-modules={n}
      >
        <rect width={size} height={size} fill="#fff" />
        <path d={d} fill="#000" />
      </svg>
      <figcaption>
        {/* A span, not <code>: the receiving reference stays the only code element in its panel. */}
        <span className="payment-qr-text">{payload}</span>
      </figcaption>
    </figure>
  );
}
