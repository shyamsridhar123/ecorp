#!/usr/bin/env python3
"""Validate that the scenario's evidence artifacts are present and coherent.

This is the persisted verifier's last gate: it refuses to pass on provider
claims alone, and instead re-reads the artifacts the test runs actually
produced.

Run:  python verify_evidence.py
Exit: 0 when every check passes, 1 otherwise.
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent
EVIDENCE = ROOT / "evidence"

REQUIRED_SOURCES = (
    "server.py",
    "browser_smoke.py",
    "verify_evidence.py",
    "README.md",
    "EVIDENCE.md",
    ".gitignore",
    "creditexc/__init__.py",
    "creditexc/api.py",
    "creditexc/audit.py",
    "creditexc/domain.py",
    "creditexc/errors.py",
    "creditexc/policy.py",
    "creditexc/rbac.py",
    "creditexc/service.py",
    "static/index.html",
    "static/app.js",
    "static/styles.css",
    "tests/test_credit_exception.py",
)

REQUIRED_EVIDENCE = (
    "unit-tests.txt",
    "browser-smoke.json",
    "browser-smoke.txt",
    "desktop-approved.png",
    "mobile-detail.png",
)

REQUIRED_SMOKE_STEPS = (
    "tenant isolation",
    "role denial",
    "version conflict",
    "maker-checker denial",
    "final authority approved",
    "audit chain verified",
    "mobile 390x844",
)


class Checks:
    def __init__(self) -> None:
        self.failures: list[str] = []
        self.passes: list[str] = []

    def ok(self, label: str) -> None:
        self.passes.append(label)

    def fail(self, label: str) -> None:
        self.failures.append(label)

    def expect(self, condition: bool, label: str) -> bool:
        (self.ok if condition else self.fail)(label)
        return condition


def check_sources(c: Checks) -> None:
    for rel in REQUIRED_SOURCES:
        path = ROOT / rel
        c.expect(path.is_file() and path.stat().st_size > 0,
                 f"source present and non-empty: {rel}")


def check_stdlib_only(c: Checks) -> None:
    """The runtime package must not import third-party modules."""
    banned = ("playwright", "flask", "django", "fastapi", "requests",
              "sqlalchemy", "pydantic", "numpy")
    offenders = []
    for path in sorted((ROOT / "creditexc").glob("*.py")):
        text = path.read_text(encoding="utf-8")
        for name in banned:
            if f"import {name}" in text or f"from {name}" in text:
                offenders.append(f"{path.name}:{name}")
    c.expect(not offenders,
             f"runtime package is standard-library only ({offenders or 'clean'})")


def check_evidence_files(c: Checks) -> None:
    for rel in REQUIRED_EVIDENCE:
        path = EVIDENCE / rel
        c.expect(path.is_file() and path.stat().st_size > 0,
                 f"evidence present and non-empty: evidence/{rel}")


def check_unit_test_log(c: Checks) -> None:
    path = EVIDENCE / "unit-tests.txt"
    if not path.is_file():
        c.fail("unit test log readable")
        return
    text = path.read_text(encoding="utf-8", errors="replace")
    c.expect("OK" in text, "unit test log reports OK")
    c.expect("FAILED" not in text, "unit test log reports no failures")
    ran = [ln for ln in text.splitlines() if ln.startswith("Ran ")]
    if c.expect(bool(ran), "unit test log records a test count"):
        count = int(ran[-1].split()[1])
        c.expect(count >= 80, f"unit test count is substantial ({count} tests)")


def check_browser_report(c: Checks) -> None:
    path = EVIDENCE / "browser-smoke.json"
    if not path.is_file():
        c.fail("browser smoke report readable")
        return
    try:
        report = json.loads(path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as exc:
        c.fail(f"browser smoke report is valid JSON ({exc})")
        return

    c.expect(report.get("passed") is True, "browser smoke reports passed=true")
    c.expect(report.get("errors") == [], "browser smoke recorded zero errors")
    c.expect(report.get("port") == 9313, "browser smoke used port 9313")
    c.expect(report.get("viewports", {}).get("desktop") == {"width": 1440, "height": 900},
             "desktop viewport was 1440x900")
    c.expect(report.get("viewports", {}).get("mobile") == {"width": 390, "height": 844},
             "mobile viewport was 390x844")
    c.expect(bool(report.get("exception_id")),
             f"browser smoke drove a real exception ({report.get('exception_id')})")

    steps = " | ".join(report.get("steps", []))
    for needle in REQUIRED_SMOKE_STEPS:
        c.expect(needle in steps, f"browser smoke covered: {needle}")
    c.expect("server started" in steps and "server stopped" in steps,
             "browser smoke owned its server lifecycle")


def main() -> int:
    c = Checks()
    check_sources(c)
    check_stdlib_only(c)
    check_evidence_files(c)
    check_unit_test_log(c)
    check_browser_report(c)

    for label in c.passes:
        print(f"  ok   {label}")
    for label in c.failures:
        print(f"  FAIL {label}", file=sys.stderr)

    total = len(c.passes) + len(c.failures)
    print(f"\nevidence verification: {'PASS' if not c.failures else 'FAIL'} "
          f"({len(c.passes)}/{total} checks passed)")
    return 0 if not c.failures else 1


if __name__ == "__main__":
    raise SystemExit(main())
