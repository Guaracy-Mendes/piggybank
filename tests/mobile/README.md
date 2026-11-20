# WP-27 mobile acceptance manifest

| Scenario branch | Executable evidence |
| --- | --- |
| AT-01: mobile and desktop show the same own-account and payment identities; another customer substituting the account ID is denied without leakage | `tests/mobile/native_api.py` (native bearer and BFF session for the same person) and `tests/mobile/device.py` (account shown on device equals the API account) |
| AT-14 (native): a changed or stale approval is invalid; only a fresh login carrying the challenge nonce authorizes, once; a browser session cannot use the native path | `native_api.py` |
| FR-CH-06 / AT-26: after an interruption the persisted operation is shown, not a replacement | `native_api.py` (same idempotency key gives the same payment); `device.py` (forced stop, then relaunch shows the same payment, one payment server-side) |
| Lost network: commands disabled, nothing queued | `device.py` (airplane mode) |
| Suspension and resume with truthful status after access-token expiry | `device.py` |
| OS-protected credentials, rotation, reuse revocation, sign-out and device removal | `native_api.py`, `device.py` |
| NFR-11 native accessibility: labels, roles and 48 dp targets | `device.py` `check_accessibility` |
| AT-08 (payments on mobile): an approved internal payment settles once | `native_api.py`, `device.py` |

Pending, and recorded explicitly:
- iOS build and device runs (needs macOS with Xcode).
- Real devices, screen-reader walkthroughs (TalkBack, VoiceOver), store builds.
- AT-19 sensitive-log inspection on device.
