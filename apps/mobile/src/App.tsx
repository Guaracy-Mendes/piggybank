import NetInfo from "@react-native-community/netinfo";
import { StatusBar } from "expo-status-bar";
import { useCallback, useEffect, useRef, useState } from "react";
import { AppState, Pressable, ScrollView, StyleSheet, Text, View, type AppStateStatus } from "react-native";
import { SafeAreaProvider, SafeAreaView } from "react-native-safe-area-context";
import { accessToken, hasDeviceSession, SignInRequired, signIn, signOut } from "./auth";
import { config } from "./config";
import { clearPending } from "./pending";
import { Accounts } from "./screens/Accounts";
import { Documents } from "./screens/Documents";
import { Payments } from "./screens/Payments";
import { Settings } from "./screens/Settings";
import { Support } from "./screens/Support";
import { Banner, Busy, Button, Line, Title, useTheme } from "./ui";

type Tab = "accounts" | "payments" | "documents" | "support" | "settings";
const tabs: { id: Tab; label: string }[] = [
  { id: "accounts", label: "Accounts" },
  { id: "payments", label: "Payments" },
  { id: "documents", label: "Statements" },
  { id: "support", label: "Support" },
  { id: "settings", label: "Profile" },
];

export default function App() {
  return (
    <SafeAreaProvider>
      <Root />
    </SafeAreaProvider>
  );
}
function Root() {
  const t = useTheme();
  const [state, setState] = useState<"checking" | "signedOut" | "signedIn">("checking");
  const [tab, setTab] = useState<Tab>("accounts");
  const [online, setOnline] = useState(true);
  const [covered, setCovered] = useState(false);
  const [refreshKey, setRefreshKey] = useState(0);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const last = useRef<AppStateStatus>(AppState.currentState);

  /* Resume: re-establish the session from secure storage and re-read every screen from the server. */
  const resume = useCallback(async () => {
    if (!(await hasDeviceSession())) return setState("signedOut");
    try {
      await accessToken();
      setState("signedIn");
      setRefreshKey((k) => k + 1);
    } catch (e) {
      if (e instanceof SignInRequired) setState("signedOut");
      else setError("Can't reach Piggybank. Showing nothing rather than stale balances.");
    }
  }, []);
  useEffect(() => {
    void resume();
    const app = AppState.addEventListener("change", (next) => {
      // Hide balances in the app switcher; refresh truthfully when the user returns.
      setCovered(next !== "active");
      if (last.current !== "active" && next === "active") void resume();
      last.current = next;
    });
    const net = NetInfo.addEventListener((s) => setOnline(s.isConnected !== false));
    return () => {
      app.remove();
      net();
    };
  }, [resume]);
  async function start() {
    setBusy(true);
    setError("");
    try {
      await signIn();
      setState("signedIn");
      setRefreshKey((k) => k + 1);
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  }
  async function leave(remove: boolean) {
    await signOut(remove ? clearPending : undefined);
    setTab("accounts");
    setState("signedOut");
  }
  return (
    <SafeAreaView edges={["top", "bottom", "left", "right"]} style={[styles.root, { backgroundColor: t.ground }]}>
      <StatusBar style="auto" />
      {!online ? (
        <View accessibilityRole="alert" style={[styles.offline, { backgroundColor: t.negative }]}>
          <Text style={{ color: t.paper, fontWeight: "700" }}>Offline — showing the last loaded screen. Commands are paused.</Text>
        </View>
      ) : null}
      {state === "checking" ? (
        <Busy label="Starting" />
      ) : state === "signedOut" ? (
        <ScrollView contentContainerStyle={styles.content}>
          <Title sub="Synthetic demonstration. TST has no monetary value.">Piggybank</Title>
          {error ? <Banner tone="error" text={error} /> : null}
          <Line>Sign in with your Piggybank password. Your session is kept only in this device's secure storage.</Line>
          <Button label="Sign in" disabled={busy || !online} onPress={() => void start()} />
          <Line muted>{config.label}</Line>
        </ScrollView>
      ) : (
        <>
          <ScrollView contentContainerStyle={styles.content}>
            {error ? <Banner tone="error" text={error} /> : null}
            {tab === "accounts" ? <Accounts refreshKey={refreshKey} /> : null}
            {tab === "payments" ? <Payments online={online} refreshKey={refreshKey} /> : null}
            {tab === "documents" ? <Documents refreshKey={refreshKey} /> : null}
            {tab === "support" ? <Support refreshKey={refreshKey} /> : null}
            {tab === "settings" ? <Settings signOut={() => void leave(false)} removeDevice={() => void leave(true)} /> : null}
          </ScrollView>
          <View accessibilityRole="tablist" style={[styles.tabs, { borderColor: t.line, backgroundColor: t.paper }]}>
            {tabs.map((x) => (
              <Pressable
                key={x.id}
                accessibilityRole="tab"
                accessibilityLabel={x.label}
                accessibilityState={{ selected: tab === x.id }}
                onPress={() => setTab(x.id)}
                style={styles.tab}
              >
                <Text style={{ color: tab === x.id ? t.accent : t.muted, fontWeight: tab === x.id ? "700" : "500", fontSize: 13 }}>{x.label}</Text>
              </Pressable>
            ))}
          </View>
        </>
      )}
      {covered && state === "signedIn" ? (
        <View style={[StyleSheet.absoluteFill, styles.cover, { backgroundColor: t.ground }]} accessibilityElementsHidden importantForAccessibility="no-hide-descendants">
          <Text style={{ color: t.ink, fontSize: 22, fontWeight: "700" }}>Piggybank</Text>
        </View>
      ) : null}
    </SafeAreaView>
  );
}
const styles = StyleSheet.create({
  root: { flex: 1 },
  content: { padding: 16, paddingBottom: 32 },
  tabs: { flexDirection: "row", borderTopWidth: 1 },
  tab: { flex: 1, minHeight: 56, alignItems: "center", justifyContent: "center" },
  offline: { padding: 10 },
  cover: { alignItems: "center", justifyContent: "center" },
});
