import { useState, useCallback, useEffect } from "react";
import {
  Alert,
  Badge,
  Busy,
  Icon,
  message,
  short,
  useSession,
  value,
} from "@piggybank/web-core";
import type { Application, Product, Account, Cases } from "./types";
import {
  navigate,
  useRoute,
  stableKey,
  doneKey,
  validRoute,
} from "./lib/navigation";
import { StateBadge, PageTitle } from "./lib/display";
import { Wordmark, Welcome, ProductCard } from "./components/Catalog";
import { ApplicationList, ApplicationView } from "./components/Applications";
import { BalanceCard, AccountsView } from "./components/Accounts";
import { ProfileView } from "./components/Profile";
import { PaymentsView } from "./components/Payments";
import { SupportView } from "./components/Support";

export function Root() {
  const session = useSession();
  useEffect(() => {
    if (session.identity) {
      const saved = sessionStorage.getItem("pb:return");
      if (saved) {
        sessionStorage.removeItem("pb:return");
        if (
          (!window.location.hash || window.location.hash === "#/home") &&
          validRoute(saved)
        )
          navigate(saved);
      }
    }
  }, [session.identity]);
  if (session.loading)
    return (
      <div className="center">
        <Wordmark />
        <Busy label="Opening your secure session…" />
      </div>
    );
  if (session.problem)
    return (
      <div className="center">
        <Wordmark />
        <Alert>{session.problem}</Alert>
        <button onClick={() => void session.refresh()}>Try again</button>
      </div>
    );
  return session.identity ? (
    <Bank
      key={session.identity.actor.issuer + session.identity.actor.subject}
    />
  ) : (
    <Welcome />
  );
}

export function Bank() {
  const { client, logout } = useSession();
  const current = useRoute();
  const [products, setProducts] = useState<Product[]>([]);
  const [applications, setApplications] = useState<Application[]>([]);
  const [accounts, setAccounts] = useState<Account[]>([]);
  const [cases, setCases] = useState<Cases>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const [creating, setCreating] = useState(false);
  const [notice, setNotice] = useState("");
  useEffect(() => {
    setNotice("");
  }, [current]);
  const refresh = useCallback(async () => {
    try {
      const [p, a, b, c] = await Promise.all([
        client.GET("/api/v1/products").then(value),
        client.GET("/api/v1/applications").then(value),
        client.GET("/api/v1/accounts").then(value),
        client.GET("/api/v1/cases").then(value),
      ]);
      setProducts(p);
      setApplications(a);
      setAccounts(b);
      setCases(c.items);
      setError("");
    } catch (e) {
      setError(message(e));
    } finally {
      setLoading(false);
    }
  }, [client]);
  useEffect(() => {
    void refresh();
  }, [refresh]);
  const apply = async (product: Product) => {
    setCreating(true);
    setError("");
    const scope = `pb:create:${product.id}`;
    try {
      const app = await value(
        await client.POST("/api/v1/applications", {
          params: { header: { "Idempotency-Key": stableKey(scope) } },
          body: {
            productVersionId: product.id,
            marketing: false,
            attribution: null,
          },
        }),
      );
      doneKey(scope);
      navigate("/applications/" + app.id);
      await refresh();
    } catch (e) {
      setError(message(e));
    } finally {
      setCreating(false);
    }
  };
  const section = current.split("/")[1] || "home";
  const selected = current.split("/")[2];
  const open = applications.filter(
    (a) => !["ACTIVE", "DECLINED", "EXPIRED"].includes(a.state),
  );
  const name = applications
    .find((a) => a.profile)
    ?.profile?.fullName.split(" ")[0];
  const nav = [
    ["home", "Overview", "home"],
    ["accounts", "Accounts", "accounts"],
    ["payments", "Payments", "arrow"],
    ["applications", "Applications", "applications"],
    ["profile", "Profile & security", "profile"],
    ["support", "Help & support", "support"],
  ] as const;
  return (
    <div className="shell">
      <a
        className="skip"
        href="#main"
        onClick={(e) => {
          e.preventDefault();
          document.getElementById("main")?.focus();
        }}
      >
        Skip to main content
      </a>
      <aside className="sidebar" aria-label="Personal banking menu">
        <Wordmark />
        <div className="workspace-label">PERSONAL SPACE</div>
        <nav aria-label="Main navigation">
          {nav.map(([id, label, icon]) => (
            <a
              key={id}
              className={`nav-item ${section === id ? "active" : ""}`}
              aria-current={section === id ? "page" : undefined}
              href={"#/" + id}
            >
              <Icon name={icon} />
              <span>{label}</span>
              {id === "applications" && open.length > 0 ? (
                <span className="nav-count">{open.length}</span>
              ) : null}
            </a>
          ))}
        </nav>
        <div className="sidebar-bottom">
          <div className="sidebar-help">
            <span className="help-orbit">
              <Icon name="support" />
            </span>
            <strong>A little help?</strong>
            <p>Your saved requests are always here.</p>
            <a href="#/support">
              Visit support <span aria-hidden="true">↗</span>
            </a>
          </div>
          <div className="sidebar-note">
            <span className="status-dot" /> Synthetic demonstration
          </div>
        </div>
      </aside>
      <div className="main-wrap">
        <header className="topbar">
          <div className="breadcrumb">
            Personal banking <span>/</span>{" "}
            <strong>
              {nav.find((n) => n[0] === section)?.[1] ?? "Overview"}
            </strong>
          </div>
          <div className="row">
            <Badge>Synthetic demo</Badge>
            <span className="avatar" aria-hidden="true">
              {name?.[0] ?? "P"}
            </span>
            <button
              className="quiet"
              aria-label="Sign out"
              onClick={() => void logout().catch((e) => setError(message(e)))}
            >
              <Icon name="logout" />
            </button>
          </div>
        </header>
        <main id="main" tabIndex={-1} className="content">
          {error ? (
            <Alert>
              {error}{" "}
              <button className="quiet" onClick={() => void refresh()}>
                Refresh
              </button>
            </Alert>
          ) : null}
          {notice ? (
            <div className="toast" role="status">
              {notice}
            </div>
          ) : null}
          {loading ? (
            <Busy />
          ) : section === "applications" && selected ? (
            <ApplicationView
              key={selected}
              id={selected}
              products={products}
              changed={refresh}
            />
          ) : section === "accounts" ? (
            <AccountsView
              key={selected ?? "all"}
              accounts={accounts}
              selected={selected}
              disclosureId={
                current.split("/")[3] === "periodic"
                  ? current.split("/")[4]
                  : undefined
              }
              refresh={refresh}
              notify={setNotice}
            />
          ) : section === "payments" ? (
            <PaymentsView accounts={accounts} selected={selected} />
          ) : section === "profile" ? (
            <ProfileView hasAccount={accounts.length > 0} />
          ) : section === "support" ? (
            <SupportView cases={cases} changed={refresh} />
          ) : section === "applications" ? (
            <>
              <PageTitle
                eyebrow="Pick up where you left off"
                title="Your applications"
                description="Your progress is saved to your account, so you can continue across sessions."
              />
              <ApplicationList applications={applications} />
              <section className="section-space">
                <h2>Make room for something new</h2>
                <div className="product-grid">
                  {products.map((product) => (
                    <ProductCard
                      key={product.id}
                      product={product}
                      busy={creating}
                      onApply={() => void apply(product)}
                    />
                  ))}
                </div>
              </section>
            </>
          ) : (
            <>
              <div className="page-heading spread">
                <div>
                  <span className="eyebrow">Your everyday, in one place</span>
                  <h1>{name ? `Hello, ${name}.` : "Welcome to your space."}</h1>
                  <p className="muted">
                    A little clarity for the things that matter.
                  </p>
                </div>
                <button className="secondary" onClick={() => void refresh()}>
                  Refresh overview
                </button>
              </div>
              <div className="overview-grid">
                <section>
                  {accounts.length ? (
                    <>
                      <BalanceCard
                        account={
                          accounts.find((a) => a.state === "ACTIVE") ??
                          accounts[0]
                        }
                      />
                      <div className="quick-actions">
                        <button
                          className="secondary"
                          onClick={() =>
                            navigate("/accounts/" + accounts[0].id)
                          }
                        >
                          <Icon name="accounts" /> View account
                        </button>
                        <button
                          className="secondary"
                          onClick={() => navigate("/support")}
                        >
                          <Icon name="support" /> Get help
                        </button>
                        <a
                          href="/auth/customer/security"
                          className="quick-link"
                        >
                          <Icon name="shield" /> Security settings
                        </a>
                      </div>
                    </>
                  ) : (
                    <div className="intro-card">
                      <span className="eyebrow">A fresh start</span>
                      <h2>
                        Make a little room
                        <br />
                        for your everyday.
                      </h2>
                      <p>Your first account begins with a saved application.</p>
                      <button
                        onClick={() => products[0] && void apply(products[0])}
                        disabled={creating || !products.length}
                      >
                        Open an account <Icon name="arrow" />
                      </button>
                      <span className="intro-decoration" aria-hidden="true">
                        p
                      </span>
                    </div>
                  )}
                  <div className="section-space">
                    <div className="section-title">
                      <h2>
                        {open.length
                          ? "Continue your journey"
                          : "Your accounts"}
                      </h2>
                      <a
                        className="text-link"
                        href={open.length ? "#/applications" : "#/accounts"}
                      >
                        View all <span aria-hidden="true">↗</span>
                      </a>
                    </div>
                    {open.length ? (
                      <ApplicationList applications={open.slice(0, 3)} />
                    ) : accounts.length ? (
                      <div className="card account-mini-list">
                        {accounts.map((account) => (
                          <button
                            className="account-mini"
                            key={account.id}
                            onClick={() => navigate("/accounts/" + account.id)}
                          >
                            <span className="account-mini-icon">
                              <Icon name="accounts" />
                            </span>
                            <span>
                              <strong>Everyday account</strong>
                              <small className="reference">
                                •••• {short(account.id)}
                              </small>
                            </span>
                            <Badge
                              tone={account.debitBlocked ? "attention" : "good"}
                            >
                              {account.state === "CLOSED"
                                ? "Closed"
                                : account.debitBlocked
                                  ? "Outgoing frozen"
                                  : "Active"}
                            </Badge>
                            <Icon name="arrow" />
                          </button>
                        ))}
                      </div>
                    ) : (
                      <div className="card">
                        <p className="muted">
                          Your accounts will appear here after your application
                          is approved.
                        </p>
                        <a className="text-link" href="#/applications">
                          Explore the account
                        </a>
                      </div>
                    )}
                  </div>
                </section>
                <aside className="right-column" aria-label="Banking guidance">
                  <div className="card calm-card">
                    <span className="feature-icon">
                      <Icon name="shield" />
                    </span>
                    <h2>
                      A clear path.
                      <br /> At every step.
                    </h2>
                    <p>
                      Your application, account and support requests keep their
                      references when you sign out or come back later.
                    </p>
                    <div className="divider" />
                    <a className="text-link" href="#/applications">
                      View saved progress <span aria-hidden="true">↗</span>
                    </a>
                  </div>
                  <div className="card small-tip">
                    <span className="eyebrow">Good to know</span>
                    <h3>Available means ready.</h3>
                    <p>
                      Your available balance accounts for active holds. Posted
                      and held amounts are shown separately.
                    </p>
                  </div>
                </aside>
              </div>
            </>
          )}
        </main>
        <footer className="app-footer">
          <span>A little more clarity. Every day.</span>
          <span>TST · Synthetic demonstration · No monetary value</span>
        </footer>
      </div>
    </div>
  );
}
