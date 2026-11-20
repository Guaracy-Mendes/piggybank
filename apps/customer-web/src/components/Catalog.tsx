import { useState, useCallback, useEffect } from "react";
import {
  Alert,
  Badge,
  Busy,
  Icon,
  message,
  money,
  useSession,
  value,
} from "@piggybank/web-core";
import type { Product } from "../types";
import { login, register } from "../lib/navigation";

export function Wordmark() {
  return (
    <a className="wordmark" href="#/home" aria-label="Piggybank home">
      <span className="mark" aria-hidden="true">
        <svg
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.8"
          aria-hidden="true"
        >
          <path d="M19 5c-1.5 0-2.8 1.4-3 2-3.5-1.5-11-.3-11 5 0 1.8 0 3 2 4.5V20h4v-2h3v2h4v-4c1-.5 1.7-1 2-2h2v-4h-2c0-1-.5-1.5-1-2V5zM2 9v1c0 1.1.9 2 2 2h1M16 11h.01" />
        </svg>
      </span>
      piggybank<span className="wordmark-dot">.</span>
    </a>
  );
}

export function Welcome() {
  const { client } = useSession();
  const [products, setProducts] = useState<Product[] | null>(null);
  const [error, setError] = useState("");
  const load = useCallback(() => {
    void client
      .GET("/api/v1/products")
      .then(value)
      .then(setProducts)
      .catch((e) => setError(message(e)));
  }, [client]);
  useEffect(load, [load]);
  return (
    <div className="welcome">
      <header className="welcome-header">
        <Wordmark />
        <div className="row">
          <Badge>Synthetic demo</Badge>
          <button className="secondary" onClick={register}>
            Create account
          </button>
          <button className="secondary" onClick={login}>
            Sign in <Icon name="arrow" />
          </button>
        </div>
      </header>
      <main id="main" tabIndex={-1}>
        <div className="welcome-hero">
          <div>
            <span className="eyebrow">A little more possibility</span>
            <h1>
              Your everyday.
              <br />
              <span>Growing with you.</span>
            </h1>
            <p>
              A clear view of your money, an account that feels simple, and a
              place to pick up where you left off.
            </p>
            <WelcomeSlides />
            <button onClick={login}>
              Start your application <Icon name="arrow" />
            </button>
            <div className="welcome-note">
              <Icon name="shield" />
              <span>
                A synthetic banking demonstration. TST has no monetary value.
              </span>
            </div>
          </div>
          <div className="welcome-art" aria-hidden="true">
            <div className="art-orbit" />
            <div className="art-card">
              <span>piggybank.</span>
              <div className="art-emblem">p</div>
              <small>A little room to grow.</small>
            </div>
            <div className="art-leaf" />
          </div>
        </div>
        <section className="welcome-products">
          <div className="section-title">
            <div>
              <span className="eyebrow">Make space for your plans</span>
              <h2>Your next chapter starts here</h2>
            </div>
          </div>
          {error ? (
            <>
              <Alert>{error}</Alert>
              <button className="secondary" onClick={load}>
                Refresh products
              </button>
            </>
          ) : products ? (
            <div className="product-grid">
              {products.map((product) => (
                <ProductCard
                  key={product.id}
                  product={product}
                  onApply={login}
                />
              ))}
            </div>
          ) : (
            <Busy />
          )}
        </section>
      </main>
      <footer>
        Simple by design. Synthetic by purpose.
        <span>© Piggybank · Local demonstration</span>
      </footer>
    </div>
  );
}

export function ProductCard({
  product,
  onApply,
  busy = false,
}: {
  product: Product;
  onApply: () => void;
  busy?: boolean;
}) {
  return (
    <article className="card product">
      <div className="spread">
        <span className="product-symbol">
          <Icon name="accounts" />
        </span>
        <Badge>
          Synthetic · {product.label === "SYNTHETIC" ? "TST" : "Account"}
        </Badge>
      </div>
      <h3>{product.name.replace("Piggybank synthetic ", "")}</h3>
      <p className="muted">
        An everyday account with clear balances, saved applications and support
        when you need it.
      </p>
      <dl className="product-facts">
        <div>
          <dt>Eligibility</dt>
          <dd>Ages {product.minimumAge} and over</dd>
        </div>
        <div>
          <dt>Payment fee</dt>
          <dd>
            {money(product.flatFee)}
            {product.feeBasisPoints
              ? ` + ${product.feeBasisPoints / 100}%`
              : ""}
          </dd>
        </div>
        <div>
          <dt>Offer</dt>
          <dd>Version {product.version}</dd>
        </div>
      </dl>
      <button className="secondary" disabled={busy} onClick={onApply}>
        Open an account <Icon name="arrow" />
      </button>
    </article>
  );
}

function WelcomeSlides() {
  const [index, setIndex] = useState(0);
  const slides = [
    [
      "Save Automatically",
      "Turn spare change and recurring deposits into meaningful goals.",
    ],
    [
      "Spend Smarter",
      "See your spending clearly, set budgets and control your virtual cards.",
    ],
    [
      "Invest Effortlessly",
      "Explore fractional synthetic shares with a clear review before every order.",
    ],
  ];
  return (
    <section
      className="ui-welcome-slides"
      aria-label="Piggybank features"
      aria-roledescription="carousel"
    >
      <div aria-live="polite">
        <strong>{slides[index][0]}</strong>
        <p>{slides[index][1]}</p>
      </div>
      <div className="ui-actions">
        {slides.map(([title], i) => (
          <button
            key={title}
            className="secondary"
            aria-label={"Show " + title}
            aria-pressed={index === i}
            onClick={() => setIndex(i)}
          >
            {i + 1}
          </button>
        ))}
      </div>
    </section>
  );
}
