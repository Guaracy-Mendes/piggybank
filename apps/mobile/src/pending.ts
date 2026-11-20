import * as SecureStore from "expo-secure-store";

/* A financial request is persisted before it is sent. After an interruption the app resends the same
   idempotency key or reads the persisted payment; it never queues or creates a replacement payment. */
export type Pending = { key: string; quoteId: string; paymentId?: string; createdAt: string };
const KEY = "pb.pending-payment";
const store = { keychainAccessible: SecureStore.WHEN_UNLOCKED_THIS_DEVICE_ONLY };

export async function loadPending(): Promise<Pending | undefined> {
  const raw = await SecureStore.getItemAsync(KEY, store);
  return raw ? (JSON.parse(raw) as Pending) : undefined;
}
export async function savePending(p: Pending) {
  await SecureStore.setItemAsync(KEY, JSON.stringify(p), store);
}
export async function clearPending() {
  await SecureStore.deleteItemAsync(KEY, store);
}
