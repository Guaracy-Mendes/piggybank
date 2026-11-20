#!/usr/bin/env bash
# Mobile toolchain wrapper: pinned Node (.node-version), local Android SDK, isolated apps/mobile lockfile.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
node_version="$(cat "$root/.node-version")"
node_path="${NVM_DIR:-$HOME/.nvm}/versions/node/v$node_version/bin"
[ -x "$node_path/node" ] && export PATH="$node_path:$PATH"
export ANDROID_HOME="${ANDROID_HOME:-$HOME/Android/Sdk}"
# Android's JDK-image transform fails with GraalVM jlink; use a standard OpenJDK/Temurin 17+ (BANK_JAVA_HOME).
# Default: project-local Temurin 17 in .cache/jdk-17 (see the mobile runbook for the pinned, checksummed download).
[ -z "${BANK_JAVA_HOME:-}" ] && [ -x "$root/.cache/jdk-17/bin/java" ] && BANK_JAVA_HOME="$root/.cache/jdk-17"
if [ -n "${BANK_JAVA_HOME:-}" ]; then export JAVA_HOME="$BANK_JAVA_HOME"; export PATH="$JAVA_HOME/bin:$PATH"; fi
if [ "${1:-}" = build ] && java -version 2>&1 | grep -qi graalvm; then
  echo "GraalVM detected; set BANK_JAVA_HOME to a standard JDK 17+ (for example OpenJDK 21)." >&2; exit 1
fi
cd "$root/apps/mobile"
case "${1:-}" in
  install) npm ci --no-audit --no-fund ;;
  check) npx tsc --noEmit ;;
  build)
    npx expo prebuild --platform android --clean --no-install
    (cd android && NODE_ENV=production ./gradlew assembleRelease --no-daemon -PreactNativeArchitectures="${BANK_ANDROID_ABI:-x86_64}" \
      -Porg.gradle.java.installations.auto-detect=false -Porg.gradle.java.installations.auto-download=false \
      -Porg.gradle.java.installations.paths="$JAVA_HOME")
    ls -l android/app/build/outputs/apk/release/app-release.apk ;;
  *) echo "usage: scripts/mobile.sh install|check|build" >&2; exit 2 ;;
esac
