import type { ReactNode } from "react";
import { ActivityIndicator, Pressable, StyleSheet, Text, TextInput, View, useColorScheme, type TextInputProps } from "react-native";

/* Piggybank tokens (official palette), light and dark. Contrast meets WCAG AA for body text. */
export const palette = {
  light: { ink: "#13241c", muted: "#4a5c53", paper: "#ffffff", soft: "#eef4f0", line: "#d6e2db", accent: "#1d6e4e", onAccent: "#ffffff", negative: "#a3241c", ground: "#f6f8f6" },
  dark: { ink: "#e8f1ec", muted: "#a9bdb2", paper: "#16211c", soft: "#1f2d26", line: "#2f4038", accent: "#5fcf9a", onAccent: "#0b1a12", negative: "#ff9a91", ground: "#0e1512" },
};
export function useTheme() {
  return palette[useColorScheme() === "dark" ? "dark" : "light"];
}
export function Title({ children, sub }: { children: ReactNode; sub?: string }) {
  const t = useTheme();
  return (
    <View style={s.titleBlock}>
      <Text accessibilityRole="header" style={[s.title, { color: t.ink }]}>
        {children}
      </Text>
      {sub ? <Text style={[s.body, { color: t.muted }]}>{sub}</Text> : null}
    </View>
  );
}
export function Card({ children, label }: { children: ReactNode; label?: string }) {
  const t = useTheme();
  return (
    <View accessibilityLabel={label} style={[s.card, { backgroundColor: t.paper, borderColor: t.line }]}>
      {children}
    </View>
  );
}
export function Line({ children, muted, strong }: { children: ReactNode; muted?: boolean; strong?: boolean }) {
  const t = useTheme();
  return <Text style={[strong ? s.strong : s.body, { color: muted ? t.muted : t.ink }]}>{children}</Text>;
}
export function Button({ label, onPress, disabled, quiet, hint }: { label: string; onPress: () => void; disabled?: boolean; quiet?: boolean; hint?: string }) {
  const t = useTheme();
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={label}
      accessibilityHint={hint}
      accessibilityState={{ disabled: !!disabled }}
      disabled={disabled}
      onPress={onPress}
      style={({ pressed }) => [
        s.button,
        quiet ? { backgroundColor: "transparent", borderColor: t.accent, borderWidth: 1 } : { backgroundColor: t.accent },
        (disabled || pressed) && { opacity: disabled ? 0.45 : 0.8 },
      ]}
    >
      <Text style={[s.buttonText, { color: quiet ? t.accent : t.onAccent }]}>{label}</Text>
    </Pressable>
  );
}
export function Field({ label, ...props }: { label: string } & TextInputProps) {
  const t = useTheme();
  return (
    <View style={s.field}>
      <Text style={[s.label, { color: t.ink }]}>{label}</Text>
      <TextInput
        accessibilityLabel={label}
        placeholderTextColor={t.muted}
        style={[s.input, { color: t.ink, borderColor: t.line, backgroundColor: t.paper }]}
        {...props}
      />
    </View>
  );
}
export function Choice({ label, options, value, onChange }: { label: string; options: { id: string; text: string }[]; value?: string; onChange: (id: string) => void }) {
  const t = useTheme();
  return (
    <View style={s.field} accessibilityRole="radiogroup" accessibilityLabel={label}>
      <Text style={[s.label, { color: t.ink }]}>{label}</Text>
      {options.map((o) => (
        <Pressable
          key={o.id}
          accessibilityRole="radio"
          accessibilityLabel={o.text}
          accessibilityState={{ checked: value === o.id }}
          onPress={() => onChange(o.id)}
          style={[s.choice, { borderColor: value === o.id ? t.accent : t.line, backgroundColor: value === o.id ? t.soft : t.paper }]}
        >
          <Text style={[s.body, { color: t.ink }]}>{(value === o.id ? "● " : "○ ") + o.text}</Text>
        </Pressable>
      ))}
    </View>
  );
}
export function Banner({ text, tone = "info" }: { text: string; tone?: "info" | "error" }) {
  const t = useTheme();
  return (
    <View accessibilityRole="alert" accessibilityLiveRegion="polite" style={[s.banner, { backgroundColor: t.soft, borderColor: tone === "error" ? t.negative : t.accent }]}>
      <Text style={[s.body, { color: tone === "error" ? t.negative : t.ink }]}>{text}</Text>
    </View>
  );
}
export function Busy({ label = "Loading" }: { label?: string }) {
  const t = useTheme();
  return <ActivityIndicator accessibilityLabel={label} color={t.accent} style={{ margin: 24 }} />;
}
export const s = StyleSheet.create({
  titleBlock: { marginBottom: 12 },
  title: { fontSize: 26, fontWeight: "700", marginBottom: 4 },
  body: { fontSize: 16, lineHeight: 22 },
  strong: { fontSize: 18, fontWeight: "700", lineHeight: 24 },
  label: { fontSize: 15, fontWeight: "600", marginBottom: 6 },
  card: { borderWidth: 1, borderRadius: 14, padding: 16, marginBottom: 12, gap: 6 },
  button: { minHeight: 48, borderRadius: 12, alignItems: "center", justifyContent: "center", paddingHorizontal: 16, marginTop: 8 },
  buttonText: { fontSize: 16, fontWeight: "700" },
  field: { marginBottom: 12 },
  input: { minHeight: 48, borderWidth: 1, borderRadius: 10, paddingHorizontal: 12, fontSize: 16 },
  choice: { minHeight: 48, borderWidth: 1, borderRadius: 10, padding: 12, marginBottom: 6, justifyContent: "center" },
  banner: { borderWidth: 1, borderRadius: 12, padding: 12, marginBottom: 12 },
});
