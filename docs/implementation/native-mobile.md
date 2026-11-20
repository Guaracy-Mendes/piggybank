# WP-27 dedicated native mobile banking

WP-27 was implemented on 2026-11-17. It covers:
- requirements FR-CH-01 (mobile channel parity) and NFR-11 (accessible native screens)
- acceptance scenarios AT-01, AT-08, AT-14 and AT-26, for the native branches implemented here

It adds:
- the native app in `apps/mobile`
- native action-bound reauthentication through migration `0064_native_step_up.sql` (schema 64)

The device evidence is local Android emulator evidence only. iOS build and device evidence remain **pending** because they need macOS with Xcode. This is not a store, device-farm or production qualification.

## App

| Area | Detail |
| --- | --- |
| Stack | Expo SDK 54 (`expo@54.0.38`), React Native 0.81.5, React 19.1, new architecture, strict TypeScript. Dependencies are pinned exactly in an isolated `apps/mobile/package-lock.json`. The root workspace list is now explicit, so the web lockfile is unaffected. |
| Contract | `openapi-fetch` against the generated `packages/api-client` types, the same contract as the web portals. There is no mobile-only financial API. |
| Screens | Accounts and authoritative balances with freshness. Onboarding: application against the same published product and exact terms, plus status. Payments: persisted intent, recipients, native approval, status. Statements: single-use retrieval. Support and safe messages. Profile and device. |
| Build | `scripts/mobile.sh build`: Expo prebuild, then a Gradle release APK with the bundled JS (x86_64 for the emulator). Uses a pinned, checksummed Temurin 17 toolchain; GraalVM is refused. The Android project is generated and ignored by git. |

## Native authentication (no embedded web credentials)

**Sign-in.** Authorization code with PKCE (S256) for the existing public issuer client `bank-mobile`. The login runs in the system browser tab and returns through `piggybank://auth/callback`. State and nonce are checked, and `prompt=login` always asks for credentials, so a shared browser session never signs in silently.

**Tokens.** Access tokens last 30 seconds and stay in memory; the API verifier accepts `azp=bank-mobile` only for the customer trust. The refresh token is stored in Keystore-backed `expo-secure-store` with `WHEN_UNLOCKED_THIS_DEVICE_ONLY`, and application backup is disabled. Refresh tokens are single use. Reusing a rotated token makes the issuer revoke the whole chain, and the test verifies this.

**Sign-out and device removal.** Sign-out revokes the refresh token at the issuer and wipes local credentials. **Remove this device** also deletes saved pending requests.

**Native action-bound step-up (new):**
1. `POST /api/v1/approvals/{challengeId}/native-step-up` runs as the bearer `bank-mobile` actor. It returns the pending challenge's server-issued nonce and records when the attempt began (`identity.native_step_ups`, at most 3 minutes).
2. The app performs a fresh login with `prompt=login`, `max_age=0` and that nonce. It revokes the step-up session's own refresh token immediately.
3. `POST /api/v1/approvals/{challengeId}/native-proof {idToken}` verifies the ID token:
   - RS256 signature from the configured issuer
   - audience and `azp` equal to `bank-mobile`
   - constant-time nonce match
   - same issuer and subject as the bearer actor
   - `auth_time` at or after the attempt began
4. It then records the same single-use `identity.step_up_proofs` row the browser callback records. The attempt is deleted, so a proof cannot be replayed, and the action is audited.

Browser sessions (`customer-web`) are refused on this path.

**Native-only customers.** The browser callback is what registers a subject. A customer who has only ever used the app has no subject row, so the native step-up now registers it inside the same transaction. The HTTP test was reordered to cover this case; see the evidence file.

**Network.** In the synthetic profile, cleartext HTTP is allowed only for `127.0.0.1` and `localhost` through a network security config; every other host keeps Android's default. `adb reverse` maps the API (4180) and issuer (18280) ports, so issuer URLs match the tokens exactly.

## Lifecycle, interruption and accessibility

**Interrupted payments.** A payment request saves its idempotency key and quote before it is sent. After process death, a timeout or a lost response, the app either re-sends the same key (the server returns the same payment) or reads the saved payment. It never creates a replacement; the device test confirms a single payment after a forced stop.

**Offline.** Offline (no connectivity) shows a banner and disables commands. Nothing is queued for later automatic execution.

**Background.** When the app leaves the foreground, a privacy cover hides balances in the app switcher. On return, the session is re-established from secure storage and every screen re-reads the server, so status after resume is truthful.

**Accessibility.**
- Every interactive element has an accessibility label, a role (button, tab, radio) and state.
- Targets are at least 48 dp; the device test checks every clickable node.
- Body text is 16 sp and scales with system font size, using the official palette in light and dark.
- Live regions announce alerts.
- Safe-area insets keep content clear of the Android 15 edge-to-edge system bars.

## Verification

| Check | What it covers |
| --- | --- |
| `make test-mobile-api` (`tests/mobile/native_api.py`) | Native contract against the real issuer, API, worker and provider: PKCE; AT-01 cross-channel identities and cross-customer denial; interrupted payment returns the same operation; native step-up through to settlement for a native-only customer; stale, wrong-nonce and foreign attempts refused; browser session refused; refresh rotation, reuse revocation and sign-out. |
| `make test-mobile` (`tests/mobile/device.py`) | Release APK on the `Pixel_9_Pro_XL` emulator: system-browser sign-in; matching account; resume after access-token expiry; payment survives process death without duplication; offline blocks commands; native step-up settles once; statements and support; labelled 48 dp targets; device removal. |

## Models

| View | Source |
| --- | --- |
| C4 components | [current-mobile-components.puml](current-mobile-components.puml) |
| Payment and step-up sequence | [current-mobile-step-up.puml](current-mobile-step-up.puml) |

## Deliberate boundaries

- **iOS.** Not built or tested, because there is no macOS or Xcode here. The code and config include a bundle identifier and use Keychain-backed secure storage, but iOS evidence is pending.
- **Real devices and store builds.** Real-device matrices, store signing, push notifications, biometrics and app attestation are WP-28 and WP-31 inputs (DEC-10).
- **Localization.** English only. Translations and the supported device and language matrix are DEC-10.
- **Feature scope.** The consumer wealth workspace (vaults, investments, cards) is not in the R1 native scope; it remains available on the web.
