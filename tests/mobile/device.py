#!/usr/bin/env python3
"""WP-27 Android device journey on a local emulator, driven through adb and UIAutomator.

Real release APK, real issuer login in the system browser tab (PKCE, custom-scheme return), real API, worker
and provider on an isolated database. Covers sign-in, matching identities, suspension/resume with token refresh,
a payment interrupted by process death and recovered from the persisted operation, native step-up, offline
command blocking, statements, accessibility labels/target sizes and device removal. Prints no credentials.
"""
import json
import os
import re
import signal
import subprocess
import sys
import time
import uuid
import xml.etree.ElementTree as ET
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SDK = Path(os.environ.get("ANDROID_HOME", Path.home() / "Android/Sdk"))
ADB = str(SDK / "platform-tools/adb")
EMULATOR = str(SDK / "emulator/emulator")
AVD = os.environ.get("BANK_AVD", "Pixel_9_Pro_XL")
APK = ROOT / "apps/mobile/android/app/build/outputs/apk/release/app-release.apk"
PACKAGE = "com.piggybank.synthetic"
PASSWORD = "Synthetic-Local-123!"
OUT = ROOT / ".local/mobile"
sys.path.insert(0, str(Path(__file__).parent))
import native_api  # noqa: E402


def adb(*args, check=True, capture=True):
    r = subprocess.run([ADB, *args], check=check, capture_output=capture, text=True, timeout=120)
    return r.stdout.strip() if capture else ""


def shell(*args, check=True):
    return adb("shell", *args, check=check)


def boot():
    if "emulator-" in adb("devices"):
        return None
    process = subprocess.Popen([EMULATOR, "-avd", AVD, "-no-window", "-no-audio", "-no-boot-anim", "-no-snapshot-save", "-gpu", "swiftshader_indirect"],
                               stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
    adb("wait-for-device")
    for _ in range(240):
        if shell("getprop", "sys.boot_completed", check=False) == "1":
            break
        time.sleep(1)
    else:
        raise AssertionError("emulator boot timeout")
    shell("settings", "put", "global", "window_animation_scale", "0")
    shell("settings", "put", "global", "transition_animation_scale", "0")
    shell("settings", "put", "global", "animator_duration_scale", "0")
    return process


class Node:
    def __init__(self, e):
        self.text = e.get("text", "")
        self.desc = e.get("content-desc", "")
        self.rid = e.get("resource-id", "")
        self.cls = e.get("class", "")
        self.clickable = e.get("clickable") == "true"
        self.enabled = e.get("enabled") == "true"
        self.package = e.get("package", "")
        x1, y1, x2, y2 = map(int, re.findall(r"\d+", e.get("bounds", "[0,0][0,0]")))
        self.box = (x1, y1, x2, y2)

    @property
    def label(self):
        return self.text or self.desc

    def center(self):
        return (self.box[0] + self.box[2]) // 2, (self.box[1] + self.box[3]) // 2


def dump():
    for _ in range(5):
        shell("uiautomator", "dump", "/sdcard/ui.xml", check=False)
        raw = adb("exec-out", "cat", "/sdcard/ui.xml", check=False)
        if raw.startswith("<?xml"):
            return [Node(e) for e in ET.fromstring(raw).iter("node")]
        time.sleep(0.5)
    raise AssertionError("UI dump failed")


def find(nodes, label=None, pattern=None, rid=None, cls=None):
    for n in nodes:
        if label is not None and n.label != label:
            continue
        if pattern is not None and not re.search(pattern, n.label):
            continue
        if rid is not None and not n.rid.endswith(rid):
            continue
        if cls is not None and n.cls != cls:
            continue
        return n
    return None


def wait(label=None, pattern=None, rid=None, timeout=45, gone=False):
    until = time.time() + timeout
    while time.time() < until:
        n = find(dump(), label, pattern, rid)
        if (n is None) == gone:
            return n
        time.sleep(0.7)
    raise AssertionError(f"timeout waiting for {'absence of ' if gone else ''}{label or pattern or rid}")


def field(label, timeout=30):
    until = time.time() + timeout
    while time.time() < until:
        n = find(dump(), label, cls="android.widget.EditText")
        if n:
            return n
        time.sleep(0.7)
    raise AssertionError("field not found: " + label)


def tap(node):
    x, y = node.center()
    shell("input", "tap", str(x), str(y))
    time.sleep(0.6)


def tap_label(label=None, pattern=None, timeout=45):
    tap(wait(label, pattern, timeout=timeout))


def scroll_to(label=None, pattern=None, attempts=8):
    for _ in range(attempts):
        n = find(dump(), label, pattern)
        if n:
            return n
        shell("input", "swipe", "540", "1800", "540", "900", "300")
        time.sleep(0.6)
    raise AssertionError(f"not found after scrolling: {label or pattern}")


def type_into(node, text):
    tap(node)
    shell("input", "text", text.replace(" ", "%s"))
    time.sleep(0.3)


def screenshot(name):
    OUT.mkdir(parents=True, exist_ok=True)
    with open(OUT / f"{name}.png", "wb") as f:
        f.write(subprocess.run([ADB, "exec-out", "screencap", "-p"], capture_output=True, timeout=60, check=True).stdout)


def issuer_login(username):
    """Completes the Keycloak form in the system browser tab, dismissing first-run browser prompts."""
    until = time.time() + 90
    while time.time() < until:
        nodes = dump()
        for prompt in ["Use without an account", "Accept & continue", "No thanks", "No, thanks", "Got it", "Continue as guest"]:
            n = find(nodes, prompt)
            if n:
                tap(n)
                break
        else:
            user = find(nodes, rid="username") or find(nodes, cls="android.widget.EditText")
            password = find(nodes, rid="password")
            if user and password:
                if not user.text or user.text != username:
                    type_into(user, username)
                type_into(find(dump(), rid="password") or password, PASSWORD)
                shell("input", "keyevent", "KEYCODE_ENTER")
                return
            time.sleep(1)
    raise AssertionError("issuer login form not reached")


def check_accessibility(screen):
    """Every interactive app element has a label and at least a 48dp target."""
    density = int(shell("wm", "density").split()[-1]) / 160
    problems = []
    for n in dump():
        if n.package != PACKAGE or not n.clickable:
            continue
        height = n.box[3] - n.box[1]
        if not n.label:
            problems.append(("unlabelled", n.cls, n.box))
        elif height < 48 * density - 2:
            problems.append(("small", n.label, height))
    assert not problems, (screen, problems)


def main():
    try:
        journey()
    except Exception:
        try:
            screenshot("failure")
        except Exception:
            pass
        raise


def journey():
    assert APK.exists(), "Build the release APK first (see docs/runbooks/mobile-development.md)"
    emulator = boot()
    env = dict(os.environ, BANK_BROWSER_RATE_LIMIT="5000")
    fixture = subprocess.Popen(["python3", str(ROOT / "tests/browser-customer/server.py")], cwd=ROOT, env=env, start_new_session=True, stdout=subprocess.DEVNULL)
    results = {}
    try:
        for _ in range(240):
            try:
                if native_api.http("GET", native_api.API + "/health/ready")[0] == 200:
                    break
            except OSError:
                pass
            time.sleep(0.5)
        database = json.loads((ROOT / ".local/customer-browser-fixture.json").read_text())["database"]
        # Server-side state through the same native contract: Alice's active funded account and a verified recipient.
        alice, bob = native_api.Session("alice"), native_api.Session("bob")
        source, target = native_api.open_account(alice, "Device"), native_api.open_account(bob, "Recipient")
        subprocess.run([str(ROOT / "target/debug/bank-fund"), str(uuid.uuid4()), source, "100000"], check=True, stdout=subprocess.DEVNULL,
                       env=dict(os.environ, BANK_PROFILE="local", BANK_CORE_DATABASE_URL=f"postgres://bank_core:synthetic-core-only@127.0.0.1:15432/{database}"))
        reference = bob.api("POST", f"/api/v1/accounts/{target}/receiving-reference", expect=200)[1]["reference"]
        alice.api("POST", "/api/v1/beneficiaries", {"label": "Bob Device", "receivingReference": reference}, key=str(uuid.uuid4()), expect=201)
        for port in ["4180", "18280"]:
            adb("reverse", f"tcp:{port}", f"tcp:{port}")
        adb("install", "-r", str(APK))
        shell("pm", "clear", PACKAGE)
        shell("am", "start", "-W", "-n", f"{PACKAGE}/.MainActivity")
        # Sign in through the system browser tab; the custom scheme returns to the app.
        # A cold-booted emulator can be slow to draw the first frame or show a system "not responding" dialog.
        until = time.time() + 120
        while time.time() < until:
            nodes = dump()
            for dialog in ["Wait", "Close app"]:
                n = find(nodes, dialog)
                if n and n.package == "android":
                    tap(n)
            if find(nodes, "Sign in"):
                break
            if not any(n.package == PACKAGE for n in nodes):
                shell("am", "start", "-W", "-n", f"{PACKAGE}/.MainActivity")
            time.sleep(2)
        tap_label("Sign in")
        issuer_login("alice")
        wait(pattern=r"^Account •••• " + source[-4:] + "$", timeout=60)
        wait(label="Available TST 1000.00")
        results["sign_in_and_matching_identity"] = True
        check_accessibility("accounts")
        screenshot("accounts")
        # Suspension and resume after the 30 s access token expires: the app refreshes and re-reads.
        shell("input", "keyevent", "KEYCODE_HOME")
        time.sleep(35)
        shell("am", "start", "-W", "-n", f"{PACKAGE}/.MainActivity")
        wait(label="Available TST 1000.00", timeout=60)
        results["resume_after_access_token_expiry"] = True
        # Payment: save the intent, then kill the process before approval.
        tap_label("Payments")
        scroll_to("Account •••• " + source[-4:])
        tap_label("Account •••• " + source[-4:])
        tap_label("Bob Device")
        type_into(field("Amount (TST)"), "12.50")
        shell("input", "keyevent", "KEYCODE_BACK")
        tap(scroll_to("Review payment"))
        wait(pattern=r"^Payment saved")
        payments = [p for p in alice.api("GET", "/api/v1/payments", expect=200)[1] if p["total"]["minorUnits"] == "1250"]
        assert len(payments) == 1 and payments[0]["state"] == "DRAFT"
        payment = payments[0]["id"]
        shell("am", "force-stop", PACKAGE)
        shell("am", "start", "-W", "-n", f"{PACKAGE}/.MainActivity")
        tap_label("Payments", timeout=60)
        wait(label="Reference " + payment[:8], timeout=60)
        assert len([p for p in alice.api("GET", "/api/v1/payments", expect=200)[1] if p["total"]["minorUnits"] == "1250"]) == 1
        results["process_death_recovers_persisted_payment"] = True
        screenshot("payment-recovered")
        # Offline: commands are disabled and nothing is queued.
        shell("cmd", "connectivity", "airplane-mode", "enable")
        wait(pattern=r"^Offline")
        approve = find(dump(), "Approve and send")
        assert approve is not None and not approve.enabled
        screenshot("offline")
        shell("cmd", "connectivity", "airplane-mode", "disable")
        wait(pattern=r"^Offline", gone=True, timeout=60)
        for port in ["4180", "18280"]:
            adb("reverse", f"tcp:{port}", f"tcp:{port}")
        results["offline_blocks_commands"] = True
        # Native action-bound approval: a fresh issuer login carrying the server's challenge nonce.
        tap(wait("Approve and send", timeout=60))
        issuer_login("alice")
        wait(pattern=r"^Payment submitted", timeout=90)
        for _ in range(60):
            state = alice.api("GET", "/api/v1/payments/" + payment, expect=200)[1]["state"]
            if state == "SETTLED":
                break
            time.sleep(1)
        assert state == "SETTLED", state
        results["native_step_up_settles_once"] = True
        tap_label("Accounts")
        wait(label="Available TST 987.50", timeout=60)
        # Statements and support render through the same authorized APIs.
        tap_label("Statements")
        wait(label="Monthly statements, every version kept.")
        check_accessibility("statements")
        tap_label("Support")
        wait(label="Support requests")
        # Device removal revokes the session and wipes stored credentials.
        tap_label("Profile")
        tap(scroll_to("Remove this device"))
        wait(label="Sign in", timeout=30)
        shell("am", "force-stop", PACKAGE)
        shell("am", "start", "-W", "-n", f"{PACKAGE}/.MainActivity")
        wait(label="Sign in", timeout=30)
        results["device_removal"] = True
        print(json.dumps({"result": "passed", "platform": "android-emulator", "avd": AVD, **results}))
    finally:
        try:
            shell("cmd", "connectivity", "airplane-mode", "disable", check=False)
        except Exception:
            pass
        os.killpg(fixture.pid, signal.SIGTERM)
        fixture.wait(timeout=60)
        if emulator and not os.environ.get("BANK_KEEP_EMULATOR"):
            adb("emu", "kill", check=False)
            emulator.wait(timeout=60)


if __name__ == "__main__":
    main()
