import { useEffect, useState, useRef } from "react";
import { Alert, message, useSession, value } from "@piggybank/web-core";
import { useConsumer } from "./context";
import { Box, Head, Chart, Allocation } from "./components";
import {
  amount,
  shares,
  minor,
  ratio,
  text,
  type Asset,
  type Review,
  type Schedule,
  decimal,
} from "./model";
import { navigate } from "../lib/navigation";
export function Investments({ route }: { route: string }) {
  const c = useConsumer(),
    w = c.wealth,
    part = route.split("/")[2],
    symbol = route.split("/")[3];
  const [error, setError] = useState(""),
    [filter, setFilter] = useState(""),
    [range, setRange] = useState("ALL"),
    [accepted, setAccepted] = useState(false),
    [allocation, setAllocation] = useState("class");
  const assets = w.assets.filter((a) =>
    (a.symbol + " " + a.name).toLowerCase().includes(filter.toLowerCase()),
  );
  const positions = w.positions.filter((p) =>
    assets.some((a) => a.symbol === p.symbol),
  );
  const equity = w.positions.reduce((n, p) => n + BigInt(p.valueMinor), 0n),
    gain = w.positions.reduce((n, p) => n + BigInt(p.returnMinor), 0n);
  const allocationName = (symbol: string) => {
    const a = w.assets.find((a) => a.symbol === symbol)!;
    return allocation === "holding"
      ? a.symbol
      : allocation === "sector"
        ? a.sector
        : a.class;
  };
  const classes = [
    ...new Set(w.positions.map((p) => allocationName(p.symbol))),
  ].map((name) => ({
    name,
    value: w.positions
      .filter((p) => allocationName(p.symbol) === name)
      .reduce((n, p) => n + BigInt(p.valueMinor), 0n),
  }));
  const cutoff =
    range === "ALL"
      ? 0
      : Date.now() -
        ({ "1D": 1, "1W": 7, "1M": 30, "3M": 90, "1Y": 365 }[range] ?? 365) *
          86400000;
  const observations = w.observations
    .filter((p) => Date.parse(p.at) >= cutoff)
    .map((p) => ({
      at: p.at,
      value: (BigInt(p.cashMinor) + BigInt(p.equityMinor)).toString(),
    }));
  const act = async (action: string, input: Record<string, unknown>) => {
    setError("");
    try {
      return await c.command(action, input);
    } catch (e) {
      setError(message(e));
    }
  };
  if (part === "asset" && symbol) {
    const asset = w.assets.find((a) => a.symbol === symbol);
    return asset ? (
      <AssetDetail asset={asset} />
    ) : (
      <Alert>Asset unavailable.</Alert>
    );
  }
  if (part === "trade" && symbol) {
    const asset = w.assets.find((a) => a.symbol === symbol);
    return asset ? <Trade asset={asset} /> : <Alert>Asset unavailable.</Alert>;
  }
  if (part === "review" && symbol) return <ReviewAction id={symbol} />;
  if (part === "automate") return <Automation />;
  return (
    <>
      <Head
        title={
          part === "watchlist"
            ? "Watchlist & Income"
            : part === "analytics"
              ? "Portfolio Analytics"
              : "Investment Portfolio"
        }
        sub={
          w.account
            ? `Individual brokerage •••• ${w.account.id.slice(-4)} · Buying power: ${amount(w.account.buyingPowerMinor)}`
            : "A dedicated place for your cash and investments."
        }
      >
        <a className="ui-pill" href="#/invest/automate">
          Automated wealth →
        </a>
      </Head>
      <nav className="ui-tabs" aria-label="Investment navigation">
        {[
          ["", "Portfolio"],
          ["watchlist", "Watchlist & income"],
          ["analytics", "Allocation & performance"],
          ["automate", "Auto-invest"],
        ].map(([r, label]) => (
          <a
            key={r}
            href={"#/invest" + (r ? "/" + r : "")}
            aria-current={(part ?? "") === r ? "page" : undefined}
          >
            {label}
          </a>
        ))}
      </nav>
      <p className="ui-safe-notice">{w.notice}</p>
      {error && <Alert>{error}</Alert>}
      {!w.account && (
        <Box title="Start your investment journey">
          <p>
            Open a simulated individual brokerage, fund its cash balance from an
            account you own, then review and authorize your first order.
          </p>
          <label className="review-checkbox">
            <input
              type="checkbox"
              checked={accepted}
              onChange={(e) => setAccepted(e.target.checked)}
            />
            I accept the synthetic brokerage terms. Assets, prices and TST are
            simulated; no real securities or insurance are provided.
          </label>
          <button
            disabled={!accepted || c.busy}
            onClick={() => void act("BROKER_OPEN", { accepted: true })}
          >
            Open simulated brokerage
          </button>
        </Box>
      )}
      {w.account && (
        <div className="ui-invest-layout">
          <Box>
            <div className="spread">
              <p className="ui-kicker">TOTAL PORTFOLIO VALUE</p>
              <div className="ui-segment">
                {["1D", "1W", "1M", "3M", "1Y", "ALL"].map((r) => (
                  <button
                    key={r}
                    aria-pressed={range === r}
                    onClick={() => setRange(r)}
                  >
                    {r}
                  </button>
                ))}
              </div>
            </div>
            <div className="ui-number">
              {c.data.preferences.hideBalance
                ? "••••••"
                : amount(equity + BigInt(w.account.cashMinor))}
            </div>
            <p className={gain < 0n ? "ui-negative" : "ui-positive"}>
              {gain < 0n ? "▼" : "▲"} {amount(gain)} unrealized return · Cash{" "}
              {amount(w.account.cashMinor)}
            </p>
            <Chart points={observations} />
          </Box>
          <Box title="Asset Allocation">
            <div className="ui-tabs" aria-label="Allocation grouping">
              {[
                ["class", "Asset class"],
                ["sector", "Sector"],
                ["holding", "Holdings"],
              ].map(([key, label]) => (
                <button
                  className="secondary"
                  key={key}
                  aria-pressed={allocation === key}
                  onClick={() => setAllocation(key)}
                >
                  {label}
                </button>
              ))}
            </div>
            <Allocation items={classes} />
          </Box>
          <Box className="ui-wide" title="Your Holdings">
            <div className="spread">
              <p className="ui-caption">
                Native positions and synthetic valuations
              </p>
              <input
                className="ui-search"
                aria-label="Filter holdings"
                placeholder="Filter ticker or name…"
                value={filter}
                onChange={(e) => setFilter(e.target.value)}
              />
            </div>
            <div className="ui-mobile-holdings">
              {positions.map((p) => {
                const a = w.assets.find((a) => a.symbol === p.symbol)!;
                return (
                  <a
                    className="ui-click-row"
                    key={p.symbol}
                    href={"#/invest/asset/" + p.symbol}
                  >
                    <span className="ui-asset">
                      <span className="ui-asset-mark">{p.symbol}</span>
                      <span>
                        <strong>{a.name}</strong>
                        <small>{shares(p.units)} shares</small>
                      </span>
                    </span>
                    <span>
                      <strong>{amount(p.valueMinor)}</strong>
                      <small
                        className={
                          BigInt(p.returnMinor) < 0n
                            ? "ui-negative"
                            : "ui-positive"
                        }
                      >
                        {BigInt(p.returnMinor) < 0n ? "▼" : "▲"}{" "}
                        {amount(p.returnMinor)}
                      </small>
                    </span>
                  </a>
                );
              })}
            </div>
            <div
              className="ui-wide-scroll ui-desktop-holdings"
              role="region"
              aria-label="Financial records"
              tabIndex={0}
            >
              <table className="ui-table">
                <thead>
                  <tr>
                    <th>Asset</th>
                    <th>Shares</th>
                    <th>Avg cost</th>
                    <th>Current price</th>
                    <th>Total value</th>
                    <th>Total return</th>
                  </tr>
                </thead>
                <tbody>
                  {positions.map((p) => {
                    const a = w.assets.find((a) => a.symbol === p.symbol)!;
                    return (
                      <tr key={p.symbol}>
                        <td>
                          <AssetLink asset={a} />
                        </td>
                        <td>{shares(p.units)}</td>
                        <td>
                          {amount(
                            BigInt(p.units) === 0n
                              ? 0n
                              : (BigInt(p.costMinor) * 1000000n) /
                                  BigInt(p.units),
                          )}
                        </td>
                        <td>{amount(a.priceMinor)}</td>
                        <td>
                          <strong>{amount(p.valueMinor)}</strong>
                        </td>
                        <td
                          className={
                            BigInt(p.returnMinor) < 0n
                              ? "ui-negative"
                              : "ui-positive"
                          }
                        >
                          {BigInt(p.returnMinor) < 0n ? "▼" : "▲"}{" "}
                          {amount(p.returnMinor)} (
                          {ratio(
                            BigInt(p.returnMinor),
                            BigInt(p.costMinor),
                          ).toFixed(2)}
                          %)
                        </td>
                      </tr>
                    );
                  })}
                </tbody>
              </table>
            </div>
            {!positions.length && (
              <p className="ui-caption">
                No matching positions. Explore assets below to review a first
                investment.
              </p>
            )}
          </Box>
          <Box title="Move brokerage cash">
            <MoneyTransfer />
          </Box>
          <Box title="Income & dividends">
            <p className="ui-kicker">Recorded dividends</p>
            <div className="ui-number">
              {amount(
                w.dividends.reduce((n, d) => n + BigInt(d.amountMinor), 0n),
              )}
            </div>
            <p className="ui-caption">
              Payouts require a declared synthetic corporate action and actual
              holdings at its record time. Projected income is not cash.
            </p>
            {w.dividends.map((d) => (
              <p key={d.id}>
                {d.symbol} · {d.recordDate} · {amount(d.amountMinor)}
              </p>
            ))}
            <a href="#/invest/watchlist">Manage dividend reinvestment →</a>
          </Box>
        </div>
      )}
      {part === "analytics" && w.account && <PortfolioInsights />}
      <div className="ui-grid">
        <Box
          className="ui-wide"
          title={part === "watchlist" ? "Watched assets" : "Explore assets"}
        >
          <div className="spread">
            <p className="ui-caption">
              Synthetic catalogue · no live exchange feed
            </p>
            <input
              className="ui-search"
              aria-label="Search assets"
              placeholder="Search stocks, ETFs, bonds…"
              value={filter}
              onChange={(e) => setFilter(e.target.value)}
            />
          </div>
          {assets
            .filter((a) => part !== "watchlist" || a.watched)
            .map((a) => (
              <div className="ui-click-row" key={a.symbol}>
                <AssetLink asset={a} />
                <div>
                  <strong>{amount(a.priceMinor)}</strong>
                  <small className="ui-caption"> · {a.class}</small>
                </div>
                <button
                  className="secondary"
                  aria-label={(a.watched ? "Unwatch " : "Watch ") + a.symbol}
                  onClick={() =>
                    void act("WATCHLIST", {
                      symbol: a.symbol,
                      watch: !a.watched,
                    })
                  }
                >
                  {a.watched ? "★" : "☆"}
                </button>
              </div>
            ))}
          {part === "watchlist" && !assets.some((a) => a.watched) && (
            <p>No watchlist entries. Open an asset to add it.</p>
          )}
        </Box>
      </div>
      {w.account && <Orders />}
    </>
  );
}
function PortfolioInsights() {
  const { wealth: w } = useConsumer();
  const sorted = [...w.positions].sort((a, b) =>
      BigInt(a.returnMinor) < BigInt(b.returnMinor)
        ? -1
        : BigInt(a.returnMinor) > BigInt(b.returnMinor)
          ? 1
          : 0,
    ),
    projected = w.positions.reduce(
      (n, p) =>
        n +
        (BigInt(p.valueMinor) *
          BigInt(
            w.assets.find((a) => a.symbol === p.symbol)?.dividendBps ?? 0,
          )) /
          10000n,
      0n,
    ),
    next = (w.upcomingDividends ?? [])
      .filter((d) => w.positions.some((p) => p.symbol === d.symbol))
      .sort((a, b) => a.payAt.localeCompare(b.payAt))[0];
  return (
    <div className="ui-grid two">
      <Box title="Position performance">
        <dl className="ui-stats">
          <div>
            <dt>Realized return</dt>
            <dd>{amount(w.realizedMinor ?? "0")}</dd>
          </div>
          <div>
            <dt>Unrealized return</dt>
            <dd>
              {amount(
                w.positions.reduce((n, p) => n + BigInt(p.returnMinor), 0n),
              )}
            </dd>
          </div>
        </dl>
        {sorted.length > 0 && (
          <>
            <p>
              Highest current return:{" "}
              <a href={"#/invest/asset/" + sorted[sorted.length - 1].symbol}>
                {sorted[sorted.length - 1].symbol}
              </a>{" "}
              · {amount(sorted[sorted.length - 1].returnMinor)}
            </p>
            <p>
              Lowest current return:{" "}
              <a href={"#/invest/asset/" + sorted[0].symbol}>
                {sorted[0].symbol}
              </a>{" "}
              · {amount(sorted[0].returnMinor)}
            </p>
          </>
        )}
        <p className="ui-caption">
          Average-cost synthetic accounting. Recorded values include
          contributions; a market benchmark feed has not been supplied.
        </p>
      </Box>
      <Box title="Income outlook">
        <p className="ui-kicker">Indicative annual income</p>
        <div className="ui-number">{amount(projected)}</div>
        <p className="ui-caption">
          Uses current synthetic values and fixture yields. This estimate is not
          a promised return or a cash entitlement.
        </p>
        {next ? (
          <p>
            Next declared fixture payout: {next.symbol} ·{" "}
            {new Date(next.payAt).toLocaleDateString()}
            <br />
            <small>
              Actual entitlement depends on holdings at its record time.
            </small>
          </p>
        ) : (
          <p>No upcoming declared payout for current positions.</p>
        )}
      </Box>
    </div>
  );
}
function AssetLink({ asset: a }: { asset: Asset }) {
  return (
    <a className="ui-asset" href={"#/invest/asset/" + a.symbol}>
      <span className="ui-asset-mark">{a.symbol}</span>
      <span>
        {a.name}
        <small>
          {a.class} · {a.sector}
        </small>
      </span>
    </a>
  );
}
function AssetDetail({ asset: a }: { asset: Asset }) {
  const c = useConsumer(),
    p = c.wealth.positions.find((p) => p.symbol === a.symbol),
    [error, setError] = useState("");
  const estimated = p
    ? (BigInt(p.valueMinor) * BigInt(a.dividendBps)) / 10000n
    : 0n;
  return (
    <>
      <Head title={a.name} sub={a.symbol + " · " + a.class}>
        <button
          className="secondary"
          onClick={() =>
            void c
              .command("WATCHLIST", { symbol: a.symbol, watch: !a.watched })
              .catch((e) => setError(message(e)))
          }
        >
          {a.watched ? "★ Watching" : "☆ Add to watchlist"}
        </button>
      </Head>
      {error && <Alert>{error}</Alert>}
      <div className="ui-invest-layout">
        <Box>
          <span className="ui-pill">Synthetic price fixture</span>
          <div className="ui-price">{amount(a.priceMinor)}</div>
          <p className="ui-caption">
            Observed {new Date(c.wealth.quoteObservedAt).toLocaleString()} · not
            a live market price
          </p>
          <Chart
            points={(c.wealth.priceHistory ?? [])
              .filter((p) => p.symbol === a.symbol)
              .map((p) => ({ at: p.at, value: p.priceMinor }))}
          />
          <div className="ui-actions">
            <a className="text-link" href={"#/invest/trade/" + a.symbol}>
              Buy {a.symbol} →
            </a>
            {p && (
              <a className="text-link" href={"#/invest/trade/" + a.symbol}>
                Sell position →
              </a>
            )}
          </div>
        </Box>
        <Box title="Your position">
          <dl className="ui-stats">
            <div>
              <dt>Shares owned</dt>
              <dd>{p ? shares(p.units) : "0"}</dd>
            </div>
            <div>
              <dt>Market value</dt>
              <dd>{amount(p?.valueMinor ?? "0")}</dd>
            </div>
            <div>
              <dt>Average cost</dt>
              <dd>
                {p && BigInt(p.units) > 0n
                  ? amount((BigInt(p.costMinor) * 1000000n) / BigInt(p.units))
                  : "—"}
              </dd>
            </div>
            <div>
              <dt>Unrealized return</dt>
              <dd>{amount(p?.returnMinor ?? "0")}</dd>
            </div>
          </dl>
          <p>{a.description}</p>
          <p className="ui-caption">
            Fundamentals and analyst ratings are not supplied by this synthetic
            feed.
          </p>
        </Box>
        <Box title="Income & reinvestment">
          <p>
            Declared fixture yield: {(a.dividendBps / 100).toFixed(2)}%
            annualized. Indicative income at current synthetic value:{" "}
            {amount(estimated)} / year.
          </p>
          <p className="ui-caption">
            This estimate is not an entitlement, a promised return or a cash
            receipt.
          </p>
          <button
            className="secondary"
            disabled={!c.wealth.account || c.busy}
            onClick={() =>
              void c
                .review("DRIP_SET", { symbol: a.symbol, enabled: !a.drip })
                .then((r) => navigate("/invest/review/" + r.id))
                .catch((e) => setError(message(e)))
            }
          >
            {a.drip ? "Disable" : "Enable"} dividend reinvestment
          </button>
        </Box>
        <Box title="Price alerts">
          <form
            className="ui-form"
            onSubmit={(e) => {
              e.preventDefault();
              const f = new FormData(e.currentTarget);
              void c
                .command("WATCHLIST", {
                  symbol: a.symbol,
                  watch: true,
                  alertMinor: minor(text(f, "price")),
                })
                .then(() => c.notify("Price alert saved."))
                .catch((e) => setError(message(e)));
            }}
          >
            <label>
              Alert price (TST)
              <input name="price" inputMode="decimal" required />
            </label>
            <button disabled={c.busy}>Save price alert</button>
          </form>
        </Box>
      </div>
      <p className="ui-safe-notice">{c.wealth.notice}</p>
    </>
  );
}
function Trade({ asset: a }: { asset: Asset }) {
  const c = useConsumer(),
    [side, setSide] = useState("BUY"),
    [type, setType] = useState("MARKET"),
    [mode, setMode] = useState("AMOUNT"),
    [input, setInput] = useState("25.00"),
    [error, setError] = useState(""),
    [busy, setBusy] = useState(false);
  const p = c.wealth.positions.find((p) => p.symbol === a.symbol);
  async function submit(form: HTMLFormElement) {
    setBusy(true);
    setError("");
    try {
      const f = new FormData(form),
        q = await c.quote({
          symbol: a.symbol,
          side,
          orderType: type,
          mode,
          ...(mode === "AMOUNT"
            ? { amountMinor: minor(input) }
            : { quantityUnits: minor(input, 6) }),
          ...(type === "LIMIT" ? { limitMinor: minor(text(f, "limit")) } : {}),
          timeInForce: text(f, "tif"),
        });
      navigate("/invest/review/" + q.id);
    } catch (e) {
      setError(message(e));
    } finally {
      setBusy(false);
    }
  }
  return (
    <>
      <Head
        title={side + " " + a.symbol}
        sub="Configure an order, then review its exact native terms before authorization."
      />
      <div className="ui-invest-layout">
        <Box>
          <form
            className="ui-form"
            onSubmit={(e) => {
              e.preventDefault();
              void submit(e.currentTarget);
            }}
          >
            <div className="ui-two">
              <label>
                Action
                <select value={side} onChange={(e) => setSide(e.target.value)}>
                  <option>BUY</option>
                  <option>SELL</option>
                </select>
              </label>
              <label>
                Order type
                <select value={type} onChange={(e) => setType(e.target.value)}>
                  <option value="MARKET">Market</option>
                  <option value="LIMIT">Limit</option>
                </select>
              </label>
            </div>
            <label>
              Investment mode
              <select value={mode} onChange={(e) => setMode(e.target.value)}>
                <option value="AMOUNT">Amount (TST)</option>
                <option value="UNITS">Shares</option>
              </select>
            </label>
            <label>
              {mode === "AMOUNT" ? "Trade amount (TST)" : "Share quantity"}
              <input
                value={input}
                onChange={(e) => setInput(e.target.value)}
                inputMode="decimal"
                required
              />
            </label>
            <div className="ui-actions">
              {["25", "100", "500"].map((n) => (
                <button
                  key={n}
                  type="button"
                  className="secondary"
                  onClick={() => {
                    setMode("AMOUNT");
                    setInput(n);
                  }}
                >
                  {n} TST
                </button>
              ))}
              <button
                type="button"
                className="secondary"
                onClick={() => {
                  if (side === "BUY") {
                    const n = BigInt(c.wealth.account?.buyingPowerMinor ?? "0");
                    setMode("AMOUNT");
                    setInput(
                      (n / 100n).toString() +
                        "." +
                        (n % 100n).toString().padStart(2, "0"),
                    );
                  } else {
                    const n =
                      BigInt(p?.units ?? "0") - BigInt(p?.reservedUnits ?? "0");
                    setMode("UNITS");
                    setInput(
                      (n / 1000000n).toString() +
                        "." +
                        (n % 1000000n).toString().padStart(6, "0"),
                    );
                  }
                }}
              >
                Max
              </button>
            </div>
            {type === "LIMIT" && (
              <label>
                Limit price (TST)
                <input name="limit" inputMode="decimal" required />
              </label>
            )}
            <label>
              Time in force
              <select name="tif">
                <option value="DAY">Good for day (UTC)</option>
                <option value="GTC">Good for 30 days</option>
              </select>
            </label>
            {error && <Alert>{error}</Alert>}
            <button disabled={busy || !c.wealth.account}>Review order</button>
          </form>
        </Box>
        <Box title="Order context">
          <AssetLink asset={a} />
          <dl className="ui-stats">
            <div>
              <dt>Fixture price</dt>
              <dd>{amount(a.priceMinor)}</dd>
            </div>
            <div>
              <dt>Buying power</dt>
              <dd>{amount(c.wealth.account?.buyingPowerMinor ?? "0")}</dd>
            </div>
            <div>
              <dt>Shares available</dt>
              <dd>
                {p
                  ? shares(
                      (BigInt(p.units) - BigInt(p.reservedUnits)).toString(),
                    )
                  : "0"}
              </dd>
            </div>
            <div>
              <dt>Commission</dt>
              <dd>TST 0.00</dd>
            </div>
          </dl>
          <p className="ui-safe-notice">
            Cash-only simulation. No margin or short selling. Real-market PDT
            and good-faith eligibility are not qualified. Orders require
            action-bound issuer reauthentication.
          </p>
          <a href="#/invest/automate">Set up recurring investment →</a>
        </Box>
      </div>
    </>
  );
}
export function ReviewAction({ id }: { id: string }) {
  const { client } = useSession(),
    c = useConsumer(),
    [review, setReview] = useState<Review>(),
    [result, setResult] = useState<Record<string, unknown>>(),
    [error, setError] = useState(""),
    generation = useRef(0);
  async function load() {
    const n = ++generation.current;
    try {
      const r = (await client
        .GET("/api/v1/consumer/authorizations/{id}", {
          params: { path: { id } },
        })
        .then(value)) as unknown as Review;
      if (r.id !== id) throw new Error("Authorization scope mismatch");
      if (n === generation.current) {
        setReview(r);
        if (r.receipt) setResult(r.receipt);
        setError("");
      }
    } catch (e) {
      if (n === generation.current) {
        setReview(undefined);
        setError(message(e));
      }
    }
  }
  useEffect(() => {
    void load();
    return () => {
      generation.current++;
    };
  }, [id]);
  async function execute() {
    if (!review) return;
    setError("");
    try {
      const action =
        review.action === "TRADE" ? "TRADE_EXECUTE" : review.action;
      const r = await c.command(action, { authorizationId: id });
      setResult(r);
    } catch (e) {
      await load();
      setError(message(e));
    }
  }
  return (
    <>
      <Head
        title={result ? "Instruction receipt" : "Review & authorize"}
        sub="Exact terms and actual native status are retained for this instruction."
      />
      <Box>
        {error && <Alert>{error}</Alert>}
        {review && (
          <>
            <span className="ui-pill">
              {review.action.replaceAll("_", " ")}
            </span>
            <dl className="ui-receipt">
              {Object.entries(review.details).map(([k, v]) => (
                <div key={k}>
                  <dt>{k.replace(/([A-Z])/g, " $1")}</dt>
                  <dd>
                    <ReviewedValue name={k} value={v} />
                  </dd>
                </div>
              ))}
              <dt>Reference</dt>
              <dd>{review.id}</dd>
              <dt>Material digest</dt>
              <dd>{review.digest}</dd>
              <dt>Approval expires</dt>
              <dd>{new Date(review.expiresAt).toLocaleString()}</dd>
            </dl>
            {!result && (
              <>
                <p className="ui-caption">
                  Issuer reauthentication authorizes only these exact terms. A
                  placed limit order is not a filled trade.
                </p>
                {!review.approvalReady ? (
                  <a
                    className="sign-in"
                    href={review.approvalUrl}
                    onClick={() =>
                      sessionStorage.setItem(
                        "pb:return",
                        "/invest/review/" + id,
                      )
                    }
                  >
                    Confirm identity to authorize →
                  </a>
                ) : (
                  <button
                    disabled={
                      c.busy || Date.parse(review.expiresAt) <= Date.now()
                    }
                    onClick={() => void execute()}
                  >
                    Submit approved instruction
                  </button>
                )}
                <button className="secondary" onClick={() => void load()}>
                  Refresh authorization
                </button>
              </>
            )}
            {result && (
              <div role="status">
                <h2>{String(result.state ?? "Saved")}</h2>
                <dl className="ui-receipt">
                  {Object.entries(result)
                    .filter(([k]) => !["token", "cardToken"].includes(k))
                    .map(([k, v]) => (
                      <div key={k}>
                        <dt>{k}</dt>
                        <dd>
                          {typeof v === "object"
                            ? JSON.stringify(v)
                            : String(v)}
                        </dd>
                      </div>
                    ))}
                </dl>
                {Boolean(result.cardToken) && (
                  <p className="ui-code">
                    Synthetic card token: {String(result.cardToken)}
                  </p>
                )}
                {Boolean(result.token) && (
                  <p className="ui-code">Copy once: {String(result.token)}</p>
                )}
                <a
                  href={
                    review.action.startsWith("SCHEDULE_")
                      ? "#/invest/automate"
                      : "#/invest"
                  }
                >
                  Done →
                </a>
              </div>
            )}
          </>
        )}
      </Box>
    </>
  );
}
function MoneyTransfer() {
  const c = useConsumer(),
    [error, setError] = useState("");
  async function submit(form: HTMLFormElement) {
    try {
      const f = new FormData(form),
        input = {
          accountId: text(f, "account"),
          amountMinor: minor(text(f, "amount")),
        };
      if (text(f, "direction") === "WITHDRAW") {
        const r = await c.review("BROKER_WITHDRAW", input);
        navigate("/invest/review/" + r.id);
      } else {
        await c.command("BROKER_DEPOSIT", input);
        c.notify("Brokerage deposit posted.");
      }
    } catch (e) {
      setError(message(e));
    }
  }
  return (
    <form
      className="ui-form"
      onSubmit={(e) => {
        e.preventDefault();
        void submit(e.currentTarget);
      }}
    >
      <label>
        Bank account
        <select name="account" required>
          {c.data.accounts
            .filter((a) => a.state === "ACTIVE")
            .map((a) => (
              <option key={a.id} value={a.id}>
                •••• {a.id.slice(-4)} · available {amount(a.availableMinor)}
              </option>
            ))}
        </select>
      </label>
      <div className="ui-two">
        <label>
          Direction
          <select name="direction">
            <option value="DEPOSIT">Deposit into brokerage</option>
            <option value="WITHDRAW">Withdraw to bank</option>
          </select>
        </label>
        <label>
          Transfer amount (TST)
          <input name="amount" inputMode="decimal" required />
        </label>
      </div>
      {error && <Alert>{error}</Alert>}
      <button disabled={c.busy}>Continue transfer</button>
    </form>
  );
}
function Orders() {
  const c = useConsumer();
  return (
    <Box title="Recent & open orders">
      {!c.wealth.orders.length && (
        <p className="ui-caption">No orders placed.</p>
      )}
      <div
        className="ui-wide-scroll"
        role="region"
        aria-label="Financial records"
        tabIndex={0}
      >
        <table className="ui-table">
          <thead>
            <tr>
              <th>Asset / side</th>
              <th>Quantity</th>
              <th>Status</th>
              <th>Native receipt</th>
              <th>Action</th>
            </tr>
          </thead>
          <tbody>
            {c.wealth.orders.map((o) => (
              <tr key={o.id}>
                <td>
                  {o.symbol} · {o.side}
                  <small className="ui-caption"> · {o.orderType}</small>
                </td>
                <td>{shares(o.units)}</td>
                <td>{o.state}</td>
                <td>
                  {o.fill ? (
                    <>
                      <strong>{amount(o.fill.amountMinor)}</strong>
                      <small className="ui-caption"> · {o.fill.batchId}</small>
                    </>
                  ) : (
                    <span className="ui-caption">No fill receipt</span>
                  )}
                </td>
                <td>
                  {o.state === "OPEN" && (
                    <button
                      className="secondary"
                      onClick={() =>
                        void c
                          .command("ORDER_CANCEL", { id: o.id })
                          .catch(() => {})
                      }
                    >
                      Cancel order
                    </button>
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      <button className="secondary" onClick={() => void c.refresh()}>
        Refresh investment status
      </button>
    </Box>
  );
}
function ReviewedValue({ name, value: v }: { name: string; value: unknown }) {
  if (v === null || v === undefined) return <>—</>;
  if (typeof v === "object")
    return (
      <dl className="ui-review-nested">
        {Object.entries(v).map(([k, x]) => (
          <div key={k}>
            <dt>{k.replace(/([A-Z])/g, " $1")}</dt>
            <dd>
              <ReviewedValue name={k} value={x} />
            </dd>
          </div>
        ))}
      </dl>
    );
  return (
    <>
      {name.endsWith("Minor")
        ? amount(String(v))
        : name === "quantityUnits"
          ? shares(String(v))
          : String(v)}
    </>
  );
}
export function Automation() {
  const c = useConsumer(),
    [error, setError] = useState(""),
    [kind, setKind] = useState("INVEST"),
    [risk, setRisk] = useState(Number(c.data.preferences.riskLevel ?? 3)),
    [editing, setEditing] = useState<Schedule>(),
    [history, setHistory] = useState(false);
  const today = new Date().toISOString().slice(0, 10),
    summary = c.wealth.automationSummary;
  const visible = c.wealth.schedules.filter(
    (s) => history || (!s.replacedBy && s.state !== "CANCELLED"),
  );
  function edit(s: Schedule) {
    setEditing(s);
    setKind(s.kind);
    setError("");
    requestAnimationFrame(() => {
      document
        .getElementById("recurring-editor")
        ?.scrollIntoView({ behavior: "smooth", block: "start" });
      document
        .querySelector<HTMLInputElement>("#recurring-editor input[name=amount]")
        ?.focus();
    });
  }
  async function submit(form: HTMLFormElement) {
    setError("");
    try {
      const f = new FormData(form),
        input = {
          kind,
          accountId: text(f, "account"),
          amountMinor: minor(text(f, "amount")),
          frequency: text(f, "frequency"),
          nextRun: text(f, "date"),
          ...(editing ? { id: editing.id } : {}),
          ...(kind === "INVEST"
            ? { symbol: text(f, "symbol") }
            : { vaultId: text(f, "vault") }),
        };
      const r = await c.review(
        editing ? "SCHEDULE_REPLACE" : "SCHEDULE_CREATE",
        input,
      );
      navigate("/invest/review/" + r.id);
    } catch (e) {
      setError(message(e));
    }
  }
  return (
    <>
      <Head
        title="Automated Wealth Autopilot"
        sub="Explicit recurring instructions, retained consent and native execution receipts."
      />
      {summary && (
        <div className="ui-grid">
          <Box className="ui-hero">
            <p className="ui-kicker">APPROXIMATE MONTHLY COMMITMENT</p>
            <div className="ui-number">
              {amount(
                BigInt(summary.saveMonthlyMinor) +
                  BigInt(summary.investMonthlyMinor),
              )}
            </div>
            <p>
              {summary.activeCount} active instruction(s) · Next due:{" "}
              {summary.nextDueDate ?? "None"}
            </p>
            <p className="ui-caption">
              Daily × 365/12, weekly × 52/12, biweekly × 26/12; estimates round
              up to a minor unit. Actual calendar executions vary.
            </p>
          </Box>
          <Box title="Completed instructions">
            <dl className="ui-stats">
              <div>
                <dt>Saved to date</dt>
                <dd>{amount(summary.totalSavedMinor)}</dd>
              </div>
              <div>
                <dt>Invested to date</dt>
                <dd>{amount(summary.totalInvestedMinor)}</dd>
              </div>
            </dl>
            <p className="ui-caption">
              Posted financial receipts across every retained version.
              Investment totals reflect actual executed costs.
            </p>
          </Box>
        </div>
      )}
      <div className="ui-grid two">
        <Box title="Your preferences">
          <label className="ui-form">
            Risk preference: {risk} / 5
            <input
              type="range"
              min="1"
              max="5"
              value={risk}
              onChange={(e) => setRisk(Number(e.target.value))}
            />
          </label>
          <button
            className="secondary"
            onClick={() =>
              void c.command("PREFERENCES", { riskLevel: risk }).catch(() => {})
            }
          >
            Save preference
          </button>
          <p className="ui-caption">
            Preference only; changing it does not trade or rebalance your
            holdings.
          </p>
          <h3>Explore thematic funds</h3>
          {c.wealth.assets
            .filter((a) => a.class === "ETFs")
            .map((a) => (
              <p key={a.symbol}>
                <a href={"#/invest/asset/" + a.symbol}>{a.name} →</a>
              </p>
            ))}
        </Box>
        <div id="recurring-editor">
          <Box
            title={
              editing
                ? "Edit recurring instruction"
                : "Create recurring instruction"
            }
          >
            {editing && (
              <p className="ui-safe-notice">
                A new issuer approval replaces these terms. Previous consent and
                completed runs remain in history. This instruction will remain{" "}
                {editing.state.toLowerCase()}.
              </p>
            )}
            <form
              key={editing?.id ?? "new"}
              className="ui-form"
              onSubmit={(e) => {
                e.preventDefault();
                void submit(e.currentTarget);
              }}
            >
              <label>
                Instruction kind
                <select value={kind} onChange={(e) => setKind(e.target.value)}>
                  <option value="INVEST">Recurring investment</option>
                  <option value="SAVE">Recurring vault saving</option>
                </select>
              </label>
              <label>
                Funding account
                <select
                  name="account"
                  defaultValue={editing?.accountId}
                  required
                >
                  {c.data.accounts
                    .filter((a) => a.state === "ACTIVE")
                    .map((a) => (
                      <option key={a.id} value={a.id}>
                        •••• {a.id.slice(-4)}
                      </option>
                    ))}
                </select>
              </label>
              {kind === "INVEST" ? (
                <label>
                  Asset
                  <select
                    name="symbol"
                    defaultValue={editing?.symbol ?? undefined}
                  >
                    {c.wealth.assets.map((a) => (
                      <option key={a.symbol}>{a.symbol}</option>
                    ))}
                  </select>
                </label>
              ) : (
                <label>
                  Destination vault
                  <select
                    name="vault"
                    defaultValue={editing?.vaultId ?? undefined}
                    required
                  >
                    {c.data.vaults
                      .filter((v) => v.isOwner)
                      .map((v) => (
                        <option key={v.id} value={v.id}>
                          {v.name}
                        </option>
                      ))}
                  </select>
                </label>
              )}
              <label>
                Amount (TST, minimum 5.00)
                <input
                  name="amount"
                  inputMode="decimal"
                  defaultValue={editing ? decimal(editing.amountMinor) : ""}
                  required
                />
              </label>
              <div className="ui-two">
                <label>
                  Frequency
                  <select
                    name="frequency"
                    defaultValue={editing?.frequency ?? "DAILY"}
                  >
                    {["DAILY", "WEEKLY", "BIWEEKLY", "MONTHLY"].map((x) => (
                      <option key={x}>{x}</option>
                    ))}
                  </select>
                </label>
                <label>
                  {editing
                    ? "Next execution date (UTC)"
                    : "First execution date (UTC)"}
                  <input
                    name="date"
                    type="date"
                    min={today}
                    defaultValue={
                      editing && editing.nextRun > today
                        ? editing.nextRun
                        : today
                    }
                    required
                  />
                </label>
              </div>
              <p className="ui-caption">
                Monthly instructions keep the selected day after shorter months.
                A completed family/date cannot execute again after an edit.
              </p>
              {!c.wealth.account && kind === "INVEST" && (
                <p>
                  <a href="#/invest">
                    Open your simulated brokerage before creating recurring buys
                    →
                  </a>
                </p>
              )}
              {error && <Alert>{error}</Alert>}
              <div className="ui-actions">
                <button
                  disabled={c.busy || (kind === "INVEST" && !c.wealth.account)}
                >
                  {editing
                    ? "Review updated instruction"
                    : "Review recurring instruction"}
                </button>
                {editing && (
                  <button
                    type="button"
                    className="secondary"
                    onClick={() => {
                      setEditing(undefined);
                      setError("");
                    }}
                  >
                    Cancel editing
                  </button>
                )}
              </div>
            </form>
          </Box>
        </div>
      </div>
      <Box title="Your recurring instructions">
        <label className="review-checkbox">
          <input
            type="checkbox"
            checked={history}
            onChange={(e) => setHistory(e.target.checked)}
          />
          Show previous versions & cancelled schedules
        </label>
        {!visible.length && <p>No current recurring instructions.</p>}
        {visible.map((s) => (
          <article className="ui-mini-card" key={s.id}>
            <div className="spread">
              <strong>
                {s.symbol ??
                  c.data.vaults.find((v) => v.id === s.vaultId)?.name ??
                  "Savings"}{" "}
                · {amount(s.amountMinor)} / {s.frequency.toLowerCase()}
              </strong>
              <span className="ui-pill">
                {s.replacedBy ? "REPLACED" : s.state}
              </span>
            </div>
            <p>
              {s.state === "CANCELLED" ? "Last retained due date" : "Next due"}:{" "}
              {s.nextRun} UTC · Funding •••• {s.accountId.slice(-4)}
            </p>
            {s.familyId && (
              <p>
                {s.completedRuns ?? 0} execution(s) in this version ·{" "}
                {amount(s.familyExecutedMinor ?? "0")} executed across{" "}
                {s.familyCompletedRuns ?? 0} completed family run(s)
              </p>
            )}
            {s.lastError && (
              <p role="status">
                {s.lastError} · Execution awaits review; no completion is
                assumed.
              </p>
            )}
            {s.recentRuns && s.recentRuns.length > 0 && (
              <details>
                <summary>Execution receipts ({s.completedRuns})</summary>
                {s.recentRuns.map((r) => (
                  <p key={r.batchId}>
                    {r.dueDate} · {amount(r.executedMinor)}
                    <br />
                    <span className="ui-code">{r.batchId}</span>
                  </p>
                ))}
                <small className="ui-caption">
                  Most recent five receipts for this version.
                </small>
              </details>
            )}
            <div className="ui-actions">
              {s.state !== "CANCELLED" && (
                <>
                  {s.familyId && (
                    <button className="secondary" onClick={() => edit(s)}>
                      Edit instruction
                    </button>
                  )}
                  <button
                    className="secondary"
                    onClick={() =>
                      void c
                        .command("SCHEDULE_UPDATE", {
                          id: s.id,
                          state: s.state === "ACTIVE" ? "PAUSED" : "ACTIVE",
                        })
                        .catch(() => {})
                    }
                  >
                    {s.state === "ACTIVE" ? "Pause" : "Resume"} schedule
                  </button>
                  <button
                    className="quiet"
                    onClick={() =>
                      void c
                        .command("SCHEDULE_UPDATE", {
                          id: s.id,
                          state: "CANCELLED",
                        })
                        .catch(() => {})
                    }
                  >
                    Cancel schedule
                  </button>
                </>
              )}
            </div>
          </article>
        ))}
      </Box>
    </>
  );
}
