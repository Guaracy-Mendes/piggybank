import * as AuthSession from "expo-auth-session";
import * as Crypto from "expo-crypto";
import * as SecureStore from "expo-secure-store";
import * as WebBrowser from "expo-web-browser";
import { config } from "./config";

WebBrowser.maybeCompleteAuthSession();

/* Native OIDC: authorization code + PKCE (S256) for the public `bank-mobile` client. The refresh token lives only
   in OS-protected storage (Android Keystore / iOS Keychain, this device only); access tokens stay in memory. */
const REFRESH = "pb.refresh";
const store = { keychainAccessible: SecureStore.WHEN_UNLOCKED_THIS_DEVICE_ONLY };
let discovery: AuthSession.DiscoveryDocument | undefined;
let access: { token: string; expires: number } | undefined;
let refreshing: Promise<string> | undefined;

export class SignInRequired extends Error {
  constructor() {
    super("Please sign in again.");
  }
}
async function issuer() {
  discovery ??= await AuthSession.fetchDiscoveryAsync(config.issuer);
  return discovery;
}
function claims(jwt: string): Record<string, unknown> {
  const part = jwt.split(".")[1] ?? "";
  const json = atob(part.replace(/-/g, "+").replace(/_/g, "/").padEnd(Math.ceil(part.length / 4) * 4, "="));
  return JSON.parse(json) as Record<string, unknown>;
}
async function randomNonce() {
  const bytes = await Crypto.getRandomBytesAsync(16);
  return Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("");
}
/** A fresh interactive login. `prompt=login` always asks for credentials, so a shared browser session never signs in silently. */
async function authorize(nonce: string, fresh: boolean) {
  const d = await issuer();
  const request = new AuthSession.AuthRequest({
    clientId: config.clientId,
    redirectUri: config.redirectUri,
    scopes: ["openid"],
    usePKCE: true,
    responseType: AuthSession.ResponseType.Code,
    prompt: AuthSession.Prompt.Login,
    extraParams: fresh ? { nonce, max_age: "0" } : { nonce },
  });
  const result = await request.promptAsync(d);
  if (result.type !== "success") throw new Error(result.type === "cancel" || result.type === "dismiss" ? "Sign-in was cancelled." : "Sign-in failed.");
  const tokens = await AuthSession.exchangeCodeAsync(
    { clientId: config.clientId, code: result.params.code ?? "", redirectUri: config.redirectUri, extraParams: { code_verifier: request.codeVerifier ?? "" } },
    d,
  );
  if (!tokens.idToken || claims(tokens.idToken).nonce !== nonce) throw new Error("Sign-in response did not match this request.");
  return tokens;
}
function keep(tokens: AuthSession.TokenResponse) {
  access = { token: tokens.accessToken, expires: (tokens.issuedAt + (tokens.expiresIn ?? 30)) * 1000 };
}
export async function signIn() {
  const tokens = await authorize(await randomNonce(), false);
  if (!tokens.refreshToken) throw new Error("The issuer did not return a refresh token.");
  await SecureStore.setItemAsync(REFRESH, tokens.refreshToken, store);
  keep(tokens);
}
export async function hasDeviceSession() {
  return (await SecureStore.getItemAsync(REFRESH, store)) !== null;
}
/** Rotating refresh: each refresh token is single use; reuse makes the issuer revoke the chain. */
async function refresh(): Promise<string> {
  const token = await SecureStore.getItemAsync(REFRESH, store);
  if (!token) throw new SignInRequired();
  try {
    const tokens = await AuthSession.refreshAsync({ clientId: config.clientId, refreshToken: token }, await issuer());
    if (tokens.refreshToken) await SecureStore.setItemAsync(REFRESH, tokens.refreshToken, store);
    keep(tokens);
    return tokens.accessToken;
  } catch {
    await SecureStore.deleteItemAsync(REFRESH, store);
    access = undefined;
    throw new SignInRequired();
  }
}
export async function accessToken(): Promise<string> {
  if (access && access.expires - Date.now() > 5000) return access.token;
  refreshing ??= refresh().finally(() => (refreshing = undefined));
  return refreshing;
}
/** Native action-bound reauthentication: the ID token must carry the server-issued challenge nonce. */
export async function reauthenticate(nonce: string) {
  const tokens = await authorize(nonce, true);
  if (tokens.refreshToken) {
    // The step-up login is only evidence; its own session is revoked immediately.
    await AuthSession.revokeAsync({ clientId: config.clientId, token: tokens.refreshToken, tokenTypeHint: AuthSession.TokenTypeHint.RefreshToken }, await issuer()).catch(() => undefined);
  }
  return tokens.idToken as string;
}
/** Sign out revokes this device's refresh token at the issuer, then wipes local credentials. */
export async function signOut(removeDevice: (() => Promise<void>) | undefined = undefined) {
  const token = await SecureStore.getItemAsync(REFRESH, store);
  access = undefined;
  if (token) {
    await AuthSession.revokeAsync({ clientId: config.clientId, token, tokenTypeHint: AuthSession.TokenTypeHint.RefreshToken }, await issuer()).catch(() => undefined);
  }
  await SecureStore.deleteItemAsync(REFRESH, store);
  if (removeDevice) await removeDevice();
}
