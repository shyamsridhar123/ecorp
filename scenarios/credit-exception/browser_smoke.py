#!/usr/bin/env python3
"""Real-browser smoke test for the credit-exception workflow.

Starts its own server on port 9313, drives Chromium through the full
governed workflow at desktop and mobile viewports, and fails on any console
error, page error, or document-level horizontal overflow.

Run:  python browser_smoke.py
Exit: 0 on success, 1 on any failure.
"""

from __future__ import annotations

import json
import subprocess
import sys
import time
import urllib.error
import urllib.request
from datetime import datetime, timedelta, timezone
from pathlib import Path

try:
    from playwright.sync_api import sync_playwright
except ImportError:  # pragma: no cover
    print("FAIL: playwright is not installed on this host.", file=sys.stderr)
    raise SystemExit(1)

ROOT = Path(__file__).resolve().parent
PORT = 9313
BASE_URL = f"http://127.0.0.1:{PORT}"
EVIDENCE_DIR = ROOT / "evidence"

DESKTOP = {"width": 1440, "height": 900}
MOBILE = {"width": 390, "height": 844}


class SmokeFailure(Exception):
    pass


# --------------------------------------------------------------------------
# server lifecycle
# --------------------------------------------------------------------------


def start_server() -> subprocess.Popen:
    proc = subprocess.Popen(
        [sys.executable, str(ROOT / "server.py"), "--port", str(PORT)],
        cwd=str(ROOT),
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
    )
    deadline = time.time() + 20
    while time.time() < deadline:
        if proc.poll() is not None:
            raise SmokeFailure(f"server exited early: {proc.stdout.read()}")
        try:
            with urllib.request.urlopen(f"{BASE_URL}/api/health", timeout=1) as res:
                if res.status == 200:
                    return proc
        except (urllib.error.URLError, OSError, TimeoutError):
            time.sleep(0.2)
    proc.kill()
    raise SmokeFailure(f"server did not become healthy on port {PORT}")


def stop_server(proc: subprocess.Popen) -> int:
    proc.terminate()
    try:
        proc.wait(timeout=10)
    except subprocess.TimeoutExpired:
        proc.kill()
        proc.wait(timeout=5)
    return proc.returncode


# --------------------------------------------------------------------------
# local browser discovery
# --------------------------------------------------------------------------

# Playwright pins one exact browser build. When the host has a different
# local build installed we reuse it rather than downloading, because this
# scenario forbids network access and package installation.
_BROWSER_GLOBS = (
    "chromium_headless_shell-*/chrome-headless-shell-*/chrome-headless-shell.exe",
    "chromium_headless_shell-*/chrome-headless-shell-*/chrome-headless-shell",
    "chromium-*/chrome-win/chrome.exe",
    "chromium-*/chrome-linux/chrome",
    "chromium-*/chrome-mac/Chromium.app/Contents/MacOS/Chromium",
)


def find_local_chromium() -> str | None:
    """Return a locally installed Chromium path, or None to use the default."""
    import os

    roots = []
    env_root = os.environ.get("PLAYWRIGHT_BROWSERS_PATH")
    if env_root:
        roots.append(Path(env_root))
    local = os.environ.get("LOCALAPPDATA")
    if local:
        roots.append(Path(local) / "ms-playwright")
    roots.append(Path.home() / ".cache" / "ms-playwright")
    roots.append(Path.home() / "Library" / "Caches" / "ms-playwright")

    candidates: list[Path] = []
    for root in roots:
        if not root.is_dir():
            continue
        for pattern in _BROWSER_GLOBS:
            candidates.extend(p for p in root.glob(pattern) if p.is_file())

    if not candidates:
        return None

    def build_number(path: Path) -> int:
        for part in path.parts:
            if "-" in part and part.rsplit("-", 1)[-1].isdigit():
                return int(part.rsplit("-", 1)[-1])
        return 0

    return str(max(candidates, key=build_number))


def launch_chromium(pw):
    """Launch Chromium, falling back to any local build already on the host."""
    try:
        return pw.chromium.launch(), "playwright-default"
    except Exception:
        local = find_local_chromium()
        if not local:
            raise
        return pw.chromium.launch(executable_path=local), local


# --------------------------------------------------------------------------
# page helpers
# --------------------------------------------------------------------------


def attach_guards(page, errors: list[str]) -> None:
    """Fail on any console error or uncaught page error.

    The negative-path probes below deliberately provoke 4xx responses. The
    browser itself logs those as "Failed to load resource" console errors
    even though the application handled them correctly, so those specific
    network notices are filtered while every other console error still
    fails the run.
    """

    def on_console(message):
        if message.type != "error":
            return
        text = message.text
        if "Failed to load resource" in text and _expected_status(text):
            return
        errors.append(f"console.{message.type}: {text}")

    page.on("console", on_console)
    page.on("pageerror", lambda e: errors.append(f"pageerror: {e}"))


# Status codes the smoke test intentionally triggers to prove denial paths.
_INTENTIONAL_STATUSES = ("403", "404", "409", "422")


def _expected_status(text: str) -> bool:
    return any(f"status of {code}" in text for code in _INTENTIONAL_STATUSES)


def check_no_overflow(page, label: str, errors: list[str]) -> None:
    overflow = page.evaluate(
        "() => ({ scroll: document.documentElement.scrollWidth,"
        " client: document.documentElement.clientWidth })"
    )
    if overflow["scroll"] > overflow["client"] + 1:
        errors.append(
            f"horizontal overflow at {label}: scrollWidth={overflow['scroll']} "
            f"clientWidth={overflow['client']}"
        )


def select_actor(page, actor_id: str) -> None:
    page.select_option('[data-testid="actor-select"]', actor_id)
    page.wait_for_function(
        "id => document.querySelector('[data-testid=\"actor-caps\"]')"
        ".textContent.startsWith(id)",
        arg=actor_id,
    )


def banner_text(page) -> str:
    return page.locator('[data-testid="banner"]').inner_text().strip()


def wait_for_state(page, expected: str) -> None:
    page.wait_for_function(
        "exp => { const el = document.querySelector('[data-testid=\"detail-state\"]');"
        " return el && el.textContent.trim().toLowerCase() === exp; }",
        arg=expected.replace("_", " "),
        timeout=10000,
    )


def click_and_settle(page, testid: str) -> None:
    page.locator(f'[data-testid="{testid}"]').click()
    page.wait_for_function(
        "() => !document.querySelector('[data-testid=\"banner\"]').hidden",
        timeout=10000,
    )


# --------------------------------------------------------------------------
# the workflow
# --------------------------------------------------------------------------


def run_workflow(page, steps: list[str], errors: list[str]) -> str:
    page.goto(BASE_URL, wait_until="domcontentloaded")
    page.wait_for_function("() => document.body.dataset.ready === 'true'", timeout=15000)
    steps.append("app booted")
    check_no_overflow(page, "desktop:boot", errors)

    # --- create -----------------------------------------------------------
    select_actor(page, "nw-requester-1")
    expiry = (datetime.now(timezone.utc) + timedelta(days=45)).strftime("%Y-%m-%d")
    page.fill('[data-testid="input-applicant"]', "APP-BR7712")
    page.select_option('[data-testid="input-rule"]', "CP-101")
    page.fill('[data-testid="input-deviation"]', "1200")
    page.fill('[data-testid="input-justification"]',
              "Seasonal revenue timing depresses the ratio for a single quarter only.")
    page.fill('[data-testid="input-controls"]',
              "Quarterly covenant monitoring by the risk team\n"
              "Monthly exposure reporting to the credit committee")
    page.fill('[data-testid="input-expires"]', expiry)
    click_and_settle(page, "btn-create")

    if "Created" not in banner_text(page):
        errors.append(f"create failed: {banner_text(page)}")
        raise SmokeFailure("create step failed")
    exception_id = page.locator('[data-testid="detail-id"]').inner_text().strip()
    steps.append(f"created {exception_id}")
    wait_for_state(page, "draft")
    check_no_overflow(page, "desktop:draft", errors)

    # --- tenant denial ----------------------------------------------------
    select_actor(page, "cs-requester-1")
    page.wait_for_timeout(300)
    visible = page.locator('[data-testid="exception-item"]').count()
    if visible != 0:
        errors.append(f"tenant leak: cascadia actor sees {visible} northwind item(s)")
    else:
        steps.append("tenant isolation: cascadia sees no northwind records")

    # --- role denial ------------------------------------------------------
    select_actor(page, "nw-auditor-1")
    page.locator('[data-testid="exception-item"]').first.click()
    wait_for_state(page, "draft")
    if page.locator('[data-testid="btn-submit"]').count() != 0:
        errors.append("role leak: auditor was offered the submit action")
    else:
        steps.append("role denial: auditor has no submit action")

    # --- submit -----------------------------------------------------------
    select_actor(page, "nw-requester-1")
    page.locator('[data-testid="exception-item"]').first.click()
    wait_for_state(page, "draft")
    click_and_settle(page, "btn-submit")
    wait_for_state(page, "submitted")
    steps.append("submitted for review")

    # --- version conflict -------------------------------------------------
    # Force the server ahead of the version the page is holding, then act.
    stale_version = int(page.locator('[data-testid="detail-version"]').inner_text())
    conflict = page.evaluate(
        """async ({ id, version }) => {
             const res = await fetch(`/api/exceptions/${id}/analyze`, {
               method: 'POST',
               headers: { 'Content-Type': 'application/json', 'X-Actor': 'nw-risk-1' },
               body: JSON.stringify({ expected_version: version - 1 }),
             });
             const body = await res.json();
             return { status: res.status, code: body.error && body.error.code };
           }""",
        {"id": exception_id, "version": stale_version},
    )
    if conflict["status"] != 409 or conflict["code"] != "version_conflict":
        errors.append(f"expected 409 version_conflict, got {conflict}")
    else:
        steps.append("version conflict returned 409 version_conflict")

    # --- analyze ----------------------------------------------------------
    select_actor(page, "nw-risk-1")
    page.locator('[data-testid="exception-item"]').first.click()
    wait_for_state(page, "submitted")
    click_and_settle(page, "btn-analyze")
    wait_for_state(page, "risk review")
    steps.append("eligibility analysis routed to risk review")

    # --- risk approval ----------------------------------------------------
    click_and_settle(page, "btn-risk-approve")
    wait_for_state(page, "compliance review")
    steps.append("risk review approved")

    # --- compliance approval ---------------------------------------------
    select_actor(page, "nw-compliance-1")
    page.locator('[data-testid="exception-item"]').first.click()
    wait_for_state(page, "compliance review")
    click_and_settle(page, "btn-compliance-approve")
    wait_for_state(page, "pending decision")
    steps.append("compliance review approved")

    # --- maker-checker denial --------------------------------------------
    denial = page.evaluate(
        """async ({ id }) => {
             const cur = await (await fetch(`/api/exceptions/${id}`, {
               headers: { 'X-Actor': 'nw-requester-1' } })).json();
             const res = await fetch(`/api/exceptions/${id}/decide`, {
               method: 'POST',
               headers: { 'Content-Type': 'application/json', 'X-Actor': 'nw-compliance-1' },
               body: JSON.stringify({ expected_version: cur.version, decision: 'approve',
                                      rationale: 'reviewer trying to self-authorize' }),
             });
             const body = await res.json();
             return { status: res.status, code: body.error && body.error.code };
           }""",
        {"id": exception_id},
    )
    if denial["status"] != 403:
        errors.append(f"expected 403 for reviewer-as-authority, got {denial}")
    else:
        steps.append(f"maker-checker denial: {denial['code']}")

    # --- final decision ---------------------------------------------------
    select_actor(page, "nw-authority-1")
    page.locator('[data-testid="exception-item"]').first.click()
    wait_for_state(page, "pending decision")
    click_and_settle(page, "btn-decide-approve")
    wait_for_state(page, "approved")
    steps.append("final authority approved the exception")
    check_no_overflow(page, "desktop:approved", errors)

    # --- audit verification ----------------------------------------------
    select_actor(page, "nw-auditor-1")
    page.locator('[data-testid="exception-item"]').first.click()
    wait_for_state(page, "approved")
    click_and_settle(page, "btn-verify")
    verify_text = page.locator('[data-testid="verify-result"]').inner_text()
    if "VALID" not in verify_text:
        errors.append(f"audit verification failed: {verify_text}")
    else:
        steps.append(f"audit chain verified: {verify_text.strip()}")

    entries = page.locator('[data-testid="audit-entry"]').count()
    if entries < 6:
        errors.append(f"expected at least 6 audit entries, saw {entries}")
    else:
        steps.append(f"audit entries rendered: {entries}")

    return exception_id


def run_mobile_pass(page, steps: list[str], errors: list[str]) -> None:
    page.goto(BASE_URL, wait_until="domcontentloaded")
    page.wait_for_function("() => document.body.dataset.ready === 'true'", timeout=15000)
    check_no_overflow(page, "mobile:boot", errors)

    select_actor(page, "nw-auditor-1")
    page.locator('[data-testid="exception-item"]').first.click()
    page.wait_for_selector('[data-testid="detail-id"]', timeout=10000)
    check_no_overflow(page, "mobile:detail", errors)

    click_and_settle(page, "btn-verify")
    check_no_overflow(page, "mobile:verified", errors)
    steps.append("mobile 390x844 rendered detail and verification without overflow")


# --------------------------------------------------------------------------
# main
# --------------------------------------------------------------------------


def main() -> int:
    EVIDENCE_DIR.mkdir(exist_ok=True)
    errors: list[str] = []
    steps: list[str] = []
    started = datetime.now(timezone.utc)
    server = None
    exception_id = None
    browser_path = None

    try:
        server = start_server()
        steps.append(f"server started on port {PORT}")

        with sync_playwright() as pw:
            browser, browser_path = launch_chromium(pw)
            steps.append(f"chromium launched ({browser_path})")
            try:
                desktop = browser.new_context(viewport=DESKTOP)
                page = desktop.new_page()
                attach_guards(page, errors)
                exception_id = run_workflow(page, steps, errors)
                page.screenshot(path=str(EVIDENCE_DIR / "desktop-approved.png"),
                                full_page=True)
                desktop.close()

                mobile = browser.new_context(viewport=MOBILE, is_mobile=False)
                mpage = mobile.new_page()
                attach_guards(mpage, errors)
                run_mobile_pass(mpage, steps, errors)
                mpage.screenshot(path=str(EVIDENCE_DIR / "mobile-detail.png"),
                                 full_page=True)
                mobile.close()
            finally:
                browser.close()
    except SmokeFailure as exc:
        errors.append(str(exc))
    except Exception as exc:  # noqa: BLE001 - report any harness failure
        errors.append(f"{type(exc).__name__}: {exc}")
    finally:
        if server is not None:
            code = stop_server(server)
            steps.append(f"server stopped (exit {code})")

    report = {
        "started_at": started.isoformat(),
        "finished_at": datetime.now(timezone.utc).isoformat(),
        "port": PORT,
        "viewports": {"desktop": DESKTOP, "mobile": MOBILE},
        "browser_executable": browser_path,
        "exception_id": exception_id,
        "steps": steps,
        "errors": errors,
        "passed": not errors,
    }
    (EVIDENCE_DIR / "browser-smoke.json").write_text(
        json.dumps(report, indent=2), encoding="utf-8"
    )

    for step in steps:
        print(f"  ok  {step}")
    for err in errors:
        print(f"  FAIL {err}", file=sys.stderr)
    print(f"\nbrowser smoke: {'PASS' if not errors else 'FAIL'} "
          f"({len(steps)} steps, {len(errors)} errors)")
    return 0 if not errors else 1


if __name__ == "__main__":
    raise SystemExit(main())
