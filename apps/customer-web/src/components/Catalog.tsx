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
import { login } from "../lib/navigation";

export function Wordmark() {
  return (
    <a className="wordmark" href="#/home" aria-label="Piggybank home">
      <span className="mark" aria-hidden="true">
        p
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
