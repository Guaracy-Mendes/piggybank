import { useEffect, useState } from "react";
import { api, value } from "../api";
import { Banner, Button, Card, Line, Title } from "../ui";

/* Profile and device control. Removing the device revokes the session and wipes every stored credential. */
export function Settings({ signOut, removeDevice }: { signOut: () => void; removeDevice: () => void }) {
  const [profile, setProfile] = useState<{ fullName?: string; email?: string }>();
  const [error, setError] = useState("");
  useEffect(() => {
    api
      .GET("/api/v1/profile")
      .then(value)
      .then((p) => setProfile(p as unknown as { fullName?: string; email?: string }))
      .catch((e: Error) => setError(e.message));
  }, []);
  return (
    <>
      <Title sub="Synthetic demonstration · TST has no monetary value.">Profile & device</Title>
      {error ? <Banner tone="error" text={error} /> : null}
      <Card label="Profile">
        <Line strong>{profile?.fullName ?? "Profile"}</Line>
        <Line muted>{profile?.email ?? "No verified contact yet"}</Line>
      </Card>
      <Card label="This device">
        <Line>Your sign-in is kept in this device's secure storage only.</Line>
        <Button quiet label="Sign out" onPress={signOut} />
        <Button label="Remove this device" hint="Revokes the session and deletes stored credentials and pending requests" onPress={removeDevice} />
      </Card>
    </>
  );
}
