import Constants from "expo-constants";

type Extra = { apiBase: string; issuer: string; clientId: string; redirectUri: string; label: string };
const extra = Constants.expoConfig?.extra as Extra;

/** Synthetic local profile. On an emulator, `adb reverse` maps these loopback ports to the developer machine. */
export const config = {
  apiBase: extra.apiBase,
  issuer: extra.issuer,
  clientId: extra.clientId,
  redirectUri: extra.redirectUri,
  label: extra.label,
};
