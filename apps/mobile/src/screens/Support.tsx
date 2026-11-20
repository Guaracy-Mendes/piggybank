import { useCallback, useEffect, useState } from "react";
import { api, value } from "../api";
import { Banner, Busy, Card, Line, Title } from "../ui";

type Message = { id: string; subject: string; body: string; createdAt: string; state: string };
type Case = { id: string; state: string; message: string; createdAt: string };

/* Support requests and safe messages (no amounts, balances or links that carry secrets). */
export function Support({ refreshKey }: { refreshKey: number }) {
  const [cases, setCases] = useState<Case[]>();
  const [messages, setMessages] = useState<Message[]>([]);
  const [error, setError] = useState("");
  const load = useCallback(async () => {
    try {
      const [c, m] = await Promise.all([api.GET("/api/v1/cases").then(value), api.GET("/api/v1/notifications").then(value)]);
      setCases(((c as unknown as { items: Case[] }).items ?? []) as Case[]);
      setMessages(((m as unknown as { messages: Message[] }).messages ?? []) as Message[]);
    } catch (e) {
      setError((e as Error).message);
    }
  }, []);
  useEffect(() => {
    void load();
  }, [load, refreshKey]);
  if (!cases) return error ? <Banner tone="error" text={error} /> : <Busy label="Loading support" />;
  return (
    <>
      <Title sub="Your requests and our messages to you.">Support</Title>
      {error ? <Banner tone="error" text={error} /> : null}
      <Card label="Support requests">
        <Line strong>Support requests</Line>
        {cases.map((c) => (
          <Line key={c.id}>{c.message}</Line>
        ))}
        {!cases.length ? <Line muted>No open requests.</Line> : null}
      </Card>
      {messages.map((m) => (
        <Card key={m.id} label={m.subject}>
          <Line strong>{m.subject}</Line>
          <Line>{m.body}</Line>
          <Line muted>{new Date(m.createdAt).toLocaleString()}</Line>
        </Card>
      ))}
    </>
  );
}
