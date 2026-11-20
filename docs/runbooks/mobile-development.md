# Native mobile development runbook

This runbook covers WP-27 [native mobile banking](../implementation/native-mobile.md). The app lives in `apps/mobile`: Expo SDK 54, React Native 0.81 and React 19.1. It has its own lockfile, isolated from the web workspaces, and uses the same generated API contract (`packages/api-client`). TST has no monetary value; use the synthetic identities only.

## 1. Prerequisites

| Item | Requirement |
| --- | --- |
| Node | `.node-version` (24.15.0). `scripts/mobile.sh` selects it from nvm. |
| JDK | Expo modules require a Java 17 toolchain, and GraalVM is refused because its `jlink` breaks the Android JDK-image transform. Use a standard JDK 17 via `BANK_JAVA_HOME`. By default `scripts/mobile.sh` uses `.cache/jdk-17`: Eclipse Temurin `jdk-17.0.16+8` (`OpenJDK17U-jdk_x64_linux_hotspot_17.0.16_8.tar.gz`, SHA-256 `166774efcf0f722f2ee18eba0039de2d685b350ee14d7b69e6f83437dafd2af1`), unpacked with `--strip-components=1`. Gradle auto-detection and auto-download are disabled, so only that JDK is used. |
| Android SDK | Platform 36 and build-tools 36.1 at `$ANDROID_HOME` (default `~/Android/Sdk`). Gradle downloads the NDK and CMake that React Native needs on the first build. |
| Emulator | An x86_64 AVD with KVM. Default `Pixel_9_Pro_XL`; override with `BANK_AVD`. |
| iOS | Not buildable on Linux. An iOS build and device evidence need macOS with Xcode and remain **pending**. |
| Local services | Issuer, database and broker running (`make dev-up`, `make identity-setup`). The issuer must have the public PKCE client `bank-mobile` with redirect `piggybank://auth/callback`. `scripts/identity.py` provisions it. |

## 2. Install, type-check and build

```sh
scripts/mobile.sh install      # npm ci in apps/mobile only
scripts/mobile.sh check        # strict TypeScript against the generated OpenAPI types
scripts/mobile.sh build        # expo prebuild + Gradle release APK (x86_64; set BANK_ANDROID_ABI for others)
```

The release APK is written to `apps/mobile/android/app/build/outputs/apk/release/app-release.apk`. It bundles the JavaScript and is signed with the local debug key, so it is not a store build. `android/` is generated and ignored by git; `scripts/mobile.sh build` regenerates it.

## 3. Run on an emulator

1. Start the AVD (`~/Android/Sdk/emulator/emulator -avd Pixel_9_Pro_XL`) or let `tests/mobile/device.py` start it headless.
2. Start the API, worker and provider: the local preview (`python3 scripts/local_cx.py start`) or `make dev`.
3. Map the loopback ports into the device. The issuer URL then matches the issuer recorded in tokens, and cleartext stays limited to loopback:
   ```sh
   adb reverse tcp:4180 tcp:4180
   adb reverse tcp:18280 tcp:18280
   adb install -r apps/mobile/android/app/build/outputs/apk/release/app-release.apk
   ```
4. Open **Piggybank** and choose **Sign in**. The issuer login opens in the system browser tab. After the password, Android returns to the app through `piggybank://auth/callback`.

## 4. Security behavior to verify

| Area | Expected behavior |
| --- | --- |
| Tokens | Access tokens (30 s) stay in memory. The rotating refresh token is stored only in Keystore-backed secure storage (`WHEN_UNLOCKED_THIS_DEVICE_ONLY`), and backup is disabled. Reusing an old refresh token makes the issuer revoke the chain. |
| Payments | Every payment needs a fresh issuer login. The app requests the server's challenge nonce (`POST /api/v1/approvals/{challengeId}/native-step-up`), signs in with `prompt=login`, `max_age=0` and that nonce, then sends the ID token to `.../native-proof`. Browser sessions cannot use this path. |
| Interruptions | A payment request is saved (idempotency key and quote) before it is sent. After a crash, kill or timeout, the app re-sends the same key or reads the saved payment. It never creates a replacement. Offline mode disables commands and nothing is queued. |
| Background | When the app leaves the foreground, a cover hides balances in the app switcher. On return, the session is refreshed and every screen re-reads the server. |
| Sign out | **Sign out** revokes the refresh token at the issuer. **Remove this device** also deletes saved pending requests. Both wipe local credentials. |

## 5. Automated checks

```sh
make test-mobile-api   # native PKCE, bearer API, step-up, interruption recovery, refresh rotation, revocation
make test-mobile       # plus release build and the Android emulator journey (tests/mobile/device.py)
```

`tests/mobile/device.py` saves screenshots in `.local/mobile/`. Set `BANK_KEEP_EMULATOR=1` to leave the emulator running.

The suites start the isolated browser fixture on ports 4180–4301, so stop the preview first (`python3 scripts/local_cx.py stop`). Shared database suites run one at a time.
