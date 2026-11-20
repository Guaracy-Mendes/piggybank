import * as Crypto from "expo-crypto";
import createClient, { type Middleware } from "openapi-fetch";
import type { components, paths } from "../../../packages/api-client/src/schema";
import { accessToken } from "./auth";
import { config } from "./config";

export type Schema = components["schemas"];
/* The same generated contract as the web portals. Every request carries a fresh bearer token; no cookies, no CSRF. */
const bearer: Middleware = {
  async onRequest({ request }) {
    request.headers.set("Authorization", "Bearer " + (await accessToken()));
    request.headers.set("Accept", "application/json");
    return request;
  },
};
export const api = createClient<paths>({ baseUrl: config.apiBase });
api.use(bearer);

export class ApiError extends Error {
  constructor(public status: number) {
    super(
      status === 401
        ? "Please sign in again."
        : status === 403 || status === 404
          ? "This isn't available."
          : status === 409
            ? "This changed or isn't allowed right now. Refresh and try again."
            : status === 410
              ? "That link has expired."
              : `The service is unavailable (HTTP ${status}). Your request was not repeated.`,
    );
  }
}
export async function value<T>(result: { data?: T; error?: unknown; response: Response }): Promise<T> {
  if (!result.response.ok) throw new ApiError(result.response.status);
  return result.data as T;
}
export function tst(minor: string | number | bigint | undefined) {
  if (minor === undefined) return "—";
  const n = BigInt(minor);
  const a = n < 0n ? -n : n;
  return (n < 0n ? "−" : "") + "TST " + (a / 100n).toString() + "." + (a % 100n).toString().padStart(2, "0");
}
export function uuid() {
  return Crypto.randomUUID();
}
