import { useCallback, useEffect, useState } from "react";
import {
  Alert,
  Badge,
  Busy,
  Icon,
  message,
  short,
  useSession,
  value,
  type Schema,
} from "@piggybank/web-core";
import { RailsView } from "./Rails";
import { PaymentsView } from "./Payments";
import { CasesView } from "./Cases";
import { ConfigurationView } from "./Configuration";
import { RolesView } from "./Roles";
import { FinanceView } from "./Finance";
import { FinancePolicies } from "./FinancePolicies";
import { FinanceCorrections } from "./FinanceCorrections";
export type Access = Schema["StaffAccess"];
export function Root() {
  const { identity, loading, problem, refresh } = useSession();
  if (loading)
    return (
      <main className="center">
        <Busy label="Opening workforce session…" />
      </main>
    );
  if (problem)
    return (
      <main className="center">
        <Alert>{problem}</Alert>
        <button onClick={() => void refresh()}>Retry session</button>
      </main>
    );
  if (!identity)
    return (
      <main className="staff-welcome">
        <div className="wordmark">
          p<span>piggybank</span>
          <small>OPERATIONS</small>
        </div>
        <Badge>Workforce access</Badge>
        <h1>
          Clear decisions.
          <br />
          Accountable actions.
        </h1>
        <p>
          Review the evidence, preserve the context and keep every decision
          traceable.
        </p>
        <a className="sign-in" href="/auth/workforce/login">
          Sign in to operations <Icon name="arrow" />
        </a>
        <footer>Synthetic local environment · TST has no monetary value</footer>
      </main>
    );
  return <Workbench key={identity.actor.issuer + identity.actor.subject} />;
}
function Workbench() {
  const { client, logout } = useSession();
  const [access, setAccess] = useState<Access>();
  const [route, setRoute] = useState(location.hash.slice(1) || "/cases");
  const [error, setError] = useState("");
  const refresh = useCallback(async () => {
    try {
      setAccess(await client.GET("/api/v1/staff/me").then(value));
      setError("");
    } catch (e) {
      setError(message(e));
    }
  }, [client]);
  useEffect(() => {
    void refresh();
    const change = () => {
      setRoute(location.hash.slice(1) || "/cases");
      document
        .querySelector<HTMLElement>("main")
        ?.focus({ preventScroll: true });
    };
    window.addEventListener("hashchange", change);
    return () => window.removeEventListener("hashchange", change);
  }, [refresh]);
  const can = (prefix: string) =>
    access?.capabilities.some((c) => c.startsWith(prefix));
  return (
    <div className="workbench">
      <a
        className="skip-link"
        href="#main"
        onClick={(e) => {
          e.preventDefault();
          document.getElementById("main")?.focus();
        }}
      >
        Skip to main content
      </a>
      <aside className="staff-sidebar">
        <a className="wordmark" href="#/cases">
          p
          <span>
            piggybank<small>OPERATIONS</small>
          </span>
        </a>
        <p className="eyebrow">WORKSPACE</p>
        <nav aria-label="Operations navigation">
          {[
            ["/cases", "Review queues", "applications", can("case.")],
            [
              "/configuration",
              "Products & pricing",
              "accounts",
              can("product."),
            ],
            ["/roles", "Access register", "shield", can("roles.")],
            ["/rails", "Rail controls", "accounts", can("rail.")],
            ["/finance", "Finance & GL", "accounts", can("finance.inquire")],
            [
              "/finance/corrections",
              "Financial corrections",
              "accounts",
              can("finance.inquire"),
            ],
            [
              "/finance/policies",
              "Periodic policies",
              "accounts",
              can("finance.inquire"),
            ],
            [
              "/payments",
              "Payment inquiry",
              "accounts",
              can("payment.inquire"),
            ],
          ]
            .filter((n) => n[3])
            .map(([url, label, icon]) => (
              <a
                key={String(url)}
                aria-current={
                  route.startsWith(String(url)) &&
                  !(
                    url === "/finance" &&
                    (route.startsWith("/finance/policies") ||
                      route.startsWith("/finance/corrections"))
                  )
                    ? "page"
                    : undefined
                }
                href={"#" + url}
              >
                <Icon name={icon as "accounts"} />
                {label}
              </a>
            ))}
        </nav>
        <div className="sidebar-note">
          <Icon name="shield" />
          <strong>Every action has a source.</strong>
          <p>
            Use current evidence and an independent approver for controlled
            decisions.
          </p>
        </div>
        <span className="synthetic">SYNTHETIC · LOCAL</span>
      </aside>
      <div className="staff-content">
        <header>
          <span>
            Operations <span aria-hidden="true">/</span>{" "}
            <strong>
              {route.startsWith("/configuration")
                ? "Products & pricing"
                : route === "/roles"
                  ? "Access register"
                  : route.startsWith("/rails")
                    ? "Rail controls"
                    : route.startsWith("/finance")
                      ? "Finance & GL"
                      : route.startsWith("/payments")
                        ? "Payment inquiry"
                        : "Review queues"}
            </strong>
          </span>
          <div>
            <Badge>Workforce</Badge>
            <button
              className="secondary"
              onClick={() => void logout().catch((e) => setError(message(e)))}
            >
              Sign out
            </button>
          </div>
        </header>
        <main id="main" tabIndex={-1}>
          {error && <Alert>{error}</Alert>}
          {!access ? (
            <Busy />
          ) : (
            <>
              {route.startsWith("/configuration") ? (
                <ConfigurationView access={access} />
              ) : route === "/rails" ? (
                <RailsView access={access} />
              ) : route.startsWith("/finance/corrections") ? (
                <FinanceCorrections
                  access={access}
                  selected={
                    /^\/finance\/corrections\/[a-f0-9-]{36}$/.test(route)
                      ? route.split("/")[3]
                      : undefined
                  }
                />
              ) : route.startsWith("/finance/policies") ? (
                <FinancePolicies
                  access={access}
                  selected={
                    /^\/finance\/policies\/[a-f0-9-]{36}(?:\/bases\/[a-f0-9-]{36})?$/.test(
                      route,
                    )
                      ? route.split("/")[3]
                      : undefined
                  }
                  basisId={
                    /^\/finance\/policies\/[a-f0-9-]{36}\/bases\/[a-f0-9-]{36}$/.test(
                      route,
                    )
                      ? route.split("/")[5]
                      : undefined
                  }
                />
              ) : route.startsWith("/finance") ? (
                <FinanceView
                  access={access}
                  reportId={
                    /^\/finance\/reports\/[a-f0-9-]{36}(?:\/(?:reconciliations|credit-reconciliations|groups)\/[a-f0-9-]{36})?$/.test(
                      route,
                    )
                      ? route.split("/")[3]
                      : undefined
                  }
                  reconciliationId={
                    /^\/finance\/reports\/[a-f0-9-]{36}\/reconciliations\/[a-f0-9-]{36}$/.test(
                      route,
                    )
                      ? route.split("/")[5]
                      : undefined
                  }
                  creditReconciliationId={
                    /^\/finance\/reports\/[a-f0-9-]{36}\/credit-reconciliations\/[a-f0-9-]{36}$/.test(
                      route,
                    )
                      ? route.split("/")[5]
                      : undefined
                  }
                  groupId={
                    /^\/finance\/reports\/[a-f0-9-]{36}\/groups\/[a-f0-9-]{36}$/.test(
                      route,
                    )
                      ? route.split("/")[5]
                      : undefined
                  }
                  selected={
                    /^\/finance\/[a-f0-9-]{36}$/.test(route)
                      ? route.split("/")[2]
                      : undefined
                  }
                />
              ) : route.startsWith("/payments") ? (
                <PaymentsView
                  access={access}
                  selected={
                    /^\/payments\/[a-f0-9-]{36}$/.test(route)
                      ? route.split("/")[2]
                      : undefined
                  }
                />
              ) : route === "/roles" ? (
                <RolesView access={access} />
              ) : (
                <CasesView
                  access={access}
                  selected={
                    /^\/cases\/[a-f0-9-]{36}$/.test(route)
                      ? route.split("/")[2]
                      : undefined
                  }
                />
              )}
            </>
          )}
        </main>
        <footer>
          Current server authority applies to every action.
          <span>{access ? "Person " + short(access.personId) : ""}</span>
        </footer>
      </div>
    </div>
  );
}
