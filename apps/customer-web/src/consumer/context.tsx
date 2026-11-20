import {
  createContext,
  useContext,
  useCallback,
  useEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { useSession, value, message, Alert, Busy } from "@piggybank/web-core";
import { stableKey } from "../lib/navigation";
import type { Workspace, Wealth, Review } from "./model";
export type Consumer = {
  data: Workspace;
  wealth: Wealth;
  busy: boolean;
  error: string;
  refresh: () => Promise<void>;
  command: (
    action: string,
    input: Record<string, unknown>,
  ) => Promise<Record<string, unknown>>;
  review: (action: string, input: Record<string, unknown>) => Promise<Review>;
  quote: (input: Record<string, unknown>) => Promise<Review>;
  notify: (s: string) => void;
};
const Context = createContext<Consumer | null>(null);
export function ConsumerProvider({ children }: { children: ReactNode }) {
  const { client, identity } = useSession();
  const [data, setData] = useState<Workspace>(),
    [wealth, setWealth] = useState<Wealth>(),
    [error, setError] = useState(""),
    [notice, setNotice] = useState(""),
    [busy, setBusy] = useState(false);
  const generation = useRef(0);
  const refresh = useCallback(async () => {
    const n = ++generation.current;
    try {
      const [d, w] = await Promise.all([
        client.GET("/api/v1/consumer").then(value),
        client.GET("/api/v1/investments").then(value),
      ]);
      if (n !== generation.current) return;
      setData(d as unknown as Workspace);
      setWealth(w as unknown as Wealth);
      setError("");
    } catch (e) {
      if (n === generation.current) setError(message(e));
    }
  }, [client]);
  useEffect(() => {
    void refresh();
    return () => {
      generation.current++;
    };
  }, [refresh]);
  useEffect(() => {
    let stopped = false;
    let timer: ReturnType<typeof setTimeout>;
    const tick = async () => {
      if (document.visibilityState === "visible") await refresh();
      if (!stopped) timer = setTimeout(() => void tick(), 2000);
    };
    timer = setTimeout(() => void tick(), 2000);
    return () => {
      stopped = true;
      clearTimeout(timer);
    };
  }, [refresh]);
  useEffect(() => {
    if (!data) return;
    const theme = data.preferences.theme ?? "light";
    document.documentElement.dataset.theme =
      theme === "system"
        ? matchMedia("(prefers-color-scheme:dark)").matches
          ? "dark"
          : "light"
        : theme;
  }, [data]);
  async function command(action: string, input: Record<string, unknown>) {
    setBusy(true);
    setError("");
    try {
      const scope =
        "pb:consumer:" +
        identity?.actor.subject +
        ":" +
        action +
        ":" +
        JSON.stringify(input);
      const body = await client
        .POST("/api/v1/consumer/commands", {
          params: { header: { "Idempotency-Key": stableKey(scope) } },
          body: { action, input },
        })
        .then(value);
      sessionStorage.removeItem(scope);
      await refresh();
      return body as Record<string, unknown>;
    } catch (e) {
      setError(message(e));
      throw e;
    } finally {
      setBusy(false);
    }
  }
  async function review(action: string, input: Record<string, unknown>) {
    return (await client
      .POST("/api/v1/consumer/authorizations", { body: { action, input } })
      .then(value)) as unknown as Review;
  }
  async function quote(input: Record<string, unknown>) {
    return (await client
      .POST("/api/v1/investments/quotes", { body: input })
      .then(value)) as unknown as Review;
  }
  if (!data || !wealth)
    return (
      <>
        {error ? (
          <Alert>
            {error}
            <button onClick={() => void refresh()}>Retry workspace</button>
          </Alert>
        ) : (
          <Busy label="Opening your financial workspace…" />
        )}
      </>
    );
  return (
    <Context.Provider
      value={{
        data,
        wealth,
        error,
        busy,
        refresh,
        command,
        review,
        quote,
        notify: setNotice,
      }}
    >
      {notice && (
        <p role="status" className="notice">
          {notice}
        </p>
      )}
      {error && <Alert>{error}</Alert>}
      {children}
    </Context.Provider>
  );
}
export function useConsumer() {
  const c = useContext(Context);
  if (!c) throw new Error("Consumer workspace required");
  return c;
}
