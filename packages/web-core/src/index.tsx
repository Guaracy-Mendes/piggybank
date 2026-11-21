import {
  Component,
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { createBankClient, type components } from "@piggybank/api-client";
export type Schema = components["schemas"];
export type Trust = "customer" | "workforce";
export class ApiFailure extends Error {
  constructor(public status: number) {
    super(
      status === 401
        ? "Please sign in again. Your saved work remains available."
        : status === 409
          ? "This request needs a current version or a completed prerequisite. Refresh its status before trying again."
          : status === 403
            ? "Your current access does not permit this action."
            : status === 404
              ? "This item is unavailable in your current session."
              : "We could not confirm this request. Refresh its status before trying again.",
    );
  }
}
export function message(error: unknown): string {
  return error instanceof ApiFailure
    ? error.message
    : "We could not reach the service. Your saved work can be resumed when the connection returns.";
}
export function sessionClient(trust: Trust) {
  const client = createBankClient();
  client.use({
    onRequest({ request }) {
      request.headers.set("Accept", "application/json");
      if (!["GET", "HEAD", "OPTIONS"].includes(request.method)) {
        const name = `pb_csrf_${trust}=`;
        const value = document.cookie
          .split(";")
          .map((c) => c.trim())
          .find((c) => c.startsWith(name))
          ?.slice(name.length);
        if (value) request.headers.set("X-CSRF-Token", value);
      }
      return request;
    },
    onResponse({ response }) {
      if (response.status === 401)
        window.dispatchEvent(new Event(`bank:expired:${trust}`));
      return response;
    },
  });
  return client;
}
export async function value<T>(result: {
  data?: T;
  response: Response;
}): Promise<T> {
  if (!result.response.ok) throw new ApiFailure(result.response.status);
  return result.data as T;
}
export function money(value: Schema["Money"]): string {
  const minor = BigInt(value.minorUnits);
  const absolute = minor < 0n ? -minor : minor;
  return `${minor < 0n ? "−" : ""}${value.currency.code} ${(absolute / 100n).toLocaleString("en-US")}.${(absolute % 100n).toString().padStart(2, "0")}`;
}
export function time(value: string): string {
  return new Intl.DateTimeFormat(undefined, {
    dateStyle: "medium",
    timeStyle: "short",
  }).format(new Date(value));
}
export function short(value: string): string {
  return value.slice(-8).toUpperCase();
}
const Session = createContext<{
  identity: Schema["Identity"] | null;
  loading: boolean;
  problem: string | null;
  refresh: () => Promise<void>;
  logout: () => Promise<void>;
  client: ReturnType<typeof sessionClient>;
} | null>(null);
export function SessionProvider({
  trust,
  children,
}: {
  trust: Trust;
  children: ReactNode;
}) {
  const client = useMemo(() => sessionClient(trust), [trust]);
  const [identity, setIdentity] = useState<Schema["Identity"] | null>(null);
  const [loading, setLoading] = useState(true);
  const [problem, setProblem] = useState<string | null>(null);
  const sequence = useRef(0);
  const known = useRef<Schema["Identity"] | null>(null);
  const refresh = useCallback(async () => {
    const run = ++sequence.current;
    // With a signed-in identity this is a background revalidation (e.g. on window focus): keep the workspace
    // mounted so page state survives. A 401 still signs out at once; any other failure surfaces on the next call.
    const background = known.current !== null;
    if (!background) {
      setLoading(true);
      setProblem(null);
    }
    try {
      const result = await client.GET("/auth/{trust}/me", {
        params: { path: { trust } },
      });
      if (run !== sequence.current) return;
      if (result.response.status === 401) {
        known.current = null;
        setIdentity(null);
      } else {
        const current = await value(result);
        if (current.actor.trust !== trust.toUpperCase())
          throw new ApiFailure(403);
        if (run !== sequence.current) return;
        known.current = current;
        setIdentity(current);
      }
    } catch (error) {
      if (run === sequence.current && !background) setProblem(message(error));
    } finally {
      if (run === sequence.current) setLoading(false);
    }
  }, [client, trust]);
  useEffect(() => {
    void refresh();
    const expired = () => {
      ++sequence.current;
      known.current = null;
      setIdentity(null);
      setLoading(false);
    };
    const focus = () => {
      void refresh();
    };
    window.addEventListener(`bank:expired:${trust}`, expired);
    window.addEventListener("focus", focus);
    return () => {
      ++sequence.current;
      window.removeEventListener(`bank:expired:${trust}`, expired);
      window.removeEventListener("focus", focus);
    };
  }, [refresh, trust]);
  const logout = useCallback(async () => {
    const name = `pb_csrf_${trust}=`;
    const csrf =
      document.cookie
        .split(";")
        .map((c) => c.trim())
        .find((c) => c.startsWith(name))
        ?.slice(name.length) ?? "";
    const result = await client.POST("/auth/{trust}/logout", {
      params: {
        path: { trust },
        header: { "X-CSRF-Token": csrf, Origin: window.location.origin },
      },
    });
    const navigation = await value(result);
    if (
      !navigation ||
      !/^\/auth\/(customer|workforce)\/end-session\/[A-Za-z0-9_-]+$/.test(
        navigation.continueTo,
      )
    )
      throw new ApiFailure(503);
    known.current = null;
    setIdentity(null);
    window.location.assign(navigation.continueTo);
  }, [client, trust]);
  return (
    <Session.Provider
      value={{ identity, loading, problem, refresh, logout, client }}
    >
      {children}
    </Session.Provider>
  );
}
export function useSession() {
  const session = useContext(Session);
  if (!session) throw new Error("Session provider required");
  return session;
}
export function Alert({ children }: { children: ReactNode }) {
  return (
    <div className="alert" role="alert">
      {children}
    </div>
  );
}
export function Busy({
  label = "Loading your saved information…",
}: {
  label?: string;
}) {
  return (
    <div className="busy" role="status">
      <span className="spinner" aria-hidden="true" />
      {label}
    </div>
  );
}
export function Badge({
  children,
  tone = "neutral",
}: {
  children: ReactNode;
  tone?: "neutral" | "good" | "attention" | "danger";
}) {
  return <span className={`badge ${tone}`}>{children}</span>;
}
export function Icon({
  name,
}: {
  name:
    | "home"
    | "accounts"
    | "applications"
    | "profile"
    | "support"
    | "arrow"
    | "check"
    | "shield"
    | "logout";
}) {
  const paths = {
    home: "M3 10 12 3l9 7v10a1 1 0 0 1-1 1h-5v-7H9v7H4a1 1 0 0 1-1-1Z",
    accounts:
      "M3 8h18M5 8v12m5-12v12m4-12v12m5-12v12M3 21h18M2 5l10-3 10 3v3H2Z",
    applications: "M6 3h9l4 4v14H6ZM14 3v5h5M9 12h7m-7 4h7",
    profile: "M16 7a4 4 0 1 1-8 0 4 4 0 0 1 8 0ZM4 21v-2a8 8 0 0 1 16 0v2",
    support:
      "M4 14v-3a8 8 0 1 1 16 0v7a3 3 0 0 1-3 3h-4M4 11H2v6h4v-6ZM20 11h2v6h-4v-6Z",
    arrow: "M5 12h14m-6-6 6 6-6 6",
    check: "m5 12 4 4L19 6",
    shield: "m12 2 8 4v6c0 5-8 10-8 10S4 17 4 12V6Zm-4 9 3 3 5-6",
    logout: "M9 4H4v16h5m5-4 4-4-4-4m-6 4h13",
  };
  return (
    <svg
      viewBox="0 0 24 24"
      aria-hidden="true"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.6"
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      <path d={paths[name]} />
    </svg>
  );
}
export class ErrorBoundary extends Component<
  {
    children: ReactNode;
  },
  {
    failed: boolean;
  }
> {
  state = { failed: false };
  static getDerivedStateFromError() {
    return { failed: true };
  }
  render() {
    return this.state.failed ? (
      <main className="center">
        <Alert>This view was interrupted. Reload your saved information.</Alert>
        <button onClick={() => window.location.reload()}>Reload</button>
      </main>
    ) : (
      this.props.children
    );
  }
}
