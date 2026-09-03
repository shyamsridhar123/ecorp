"""HTTP API: routing, JSON envelopes, and idempotent command dispatch.

Built on :mod:`http.server` from the standard library. Mutating routes
accept an ``X-Command-Id`` header; replaying the same id returns the stored
response verbatim without re-executing the command.
"""

from __future__ import annotations

import json
import mimetypes
import re
from datetime import timezone
from http.server import BaseHTTPRequestHandler
from pathlib import Path
from urllib.parse import parse_qs, urlparse

from . import rbac
from .domain import ALL_STATES, APPROVAL_COMPLIANCE, APPROVAL_RISK
from .errors import AppError, MethodNotAllowed, NotFound, ValidationError
from .policy import catalog
from .service import ExceptionService

STATIC_ROOT = Path(__file__).resolve().parent.parent / "static"
MAX_BODY_BYTES = 256 * 1024

EXC_PATH = re.compile(r"^/api/exceptions/([A-Za-z0-9\-]{1,64})$")
EXC_ACTION = re.compile(r"^/api/exceptions/([A-Za-z0-9\-]{1,64})/([a-z\-]{1,32})$")


def _json_bytes(payload: object) -> bytes:
    return json.dumps(payload, ensure_ascii=False, indent=2).encode("utf-8")


class ApiHandler(BaseHTTPRequestHandler):
    server_version = "CreditExceptionService/1.0"
    protocol_version = "HTTP/1.1"

    # Injected by the server factory.
    service: ExceptionService

    # --- plumbing ---------------------------------------------------------

    def log_message(self, fmt: str, *args) -> None:  # pragma: no cover - noise
        if getattr(self.server, "verbose", False):
            super().log_message(fmt, *args)

    def _send(self, status: int, payload: object, extra_headers: dict | None = None) -> None:
        body = _json_bytes(payload)
        self.send_response(status)
        self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Cache-Control", "no-store")
        self.send_header("X-Content-Type-Options", "nosniff")
        for key, value in (extra_headers or {}).items():
            self.send_header(key, value)
        self.end_headers()
        self.wfile.write(body)

    def _send_error_obj(self, exc: AppError) -> None:
        self._send(exc.status, exc.to_payload())

    def _read_json(self) -> dict:
        raw_len = self.headers.get("Content-Length")
        if raw_len is None:
            return {}
        try:
            length = int(raw_len)
        except ValueError as exc:
            raise ValidationError("Invalid Content-Length header.") from exc
        if length < 0 or length > MAX_BODY_BYTES:
            raise ValidationError(
                f"Request body must be between 0 and {MAX_BODY_BYTES} bytes.",
                details={"max_bytes": MAX_BODY_BYTES},
            )
        if length == 0:
            return {}
        raw = self.rfile.read(length)
        try:
            parsed = json.loads(raw.decode("utf-8"))
        except (UnicodeDecodeError, json.JSONDecodeError) as exc:
            raise ValidationError("Request body must be valid UTF-8 JSON.") from exc
        if not isinstance(parsed, dict):
            raise ValidationError("Request body must be a JSON object.")
        return parsed

    def _actor(self) -> rbac.Actor:
        return rbac.authenticate(self.headers.get("X-Actor"))

    def _command_id(self) -> str | None:
        value = self.headers.get("X-Command-Id")
        if value is None:
            return None
        value = value.strip()
        if not value:
            return None
        if len(value) > 128 or not re.match(r"^[A-Za-z0-9_.\-]+$", value):
            raise ValidationError(
                "X-Command-Id must be 1-128 chars of [A-Za-z0-9_.-].",
                details={"header": "X-Command-Id"},
            )
        return value

    # --- static -----------------------------------------------------------

    def _serve_static(self, path: str) -> None:
        if path == "/favicon.ico":
            # Answer explicitly so the browser console stays clean.
            self.send_response(204)
            self.send_header("Content-Length", "0")
            self.send_header("Cache-Control", "no-store")
            self.end_headers()
            return
        rel = "index.html" if path in ("/", "") else path.lstrip("/")
        target = (STATIC_ROOT / rel).resolve()
        try:
            target.relative_to(STATIC_ROOT.resolve())
        except ValueError:
            self._send_error_obj(NotFound("Not found."))
            return
        if not target.is_file():
            self._send_error_obj(NotFound(f"No such resource: {path}"))
            return
        body = target.read_bytes()
        ctype = mimetypes.guess_type(str(target))[0] or "application/octet-stream"
        if ctype.startswith("text/") or ctype in ("application/javascript",):
            ctype += "; charset=utf-8"
        self.send_response(200)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Cache-Control", "no-store")
        self.end_headers()
        self.wfile.write(body)

    # --- verbs ------------------------------------------------------------

    def do_GET(self) -> None:
        parsed = urlparse(self.path)
        path = parsed.path
        try:
            if not path.startswith("/api/"):
                self._serve_static(path)
                return
            self._route_get(path, parse_qs(parsed.query))
        except AppError as exc:
            self._send_error_obj(exc)
        except Exception as exc:  # pragma: no cover - defensive
            self._send(500, {"error": {"code": "internal_error", "message": str(exc)}})

    def do_POST(self) -> None:
        try:
            self._route_post(urlparse(self.path).path)
        except AppError as exc:
            self._send_error_obj(exc)
        except Exception as exc:  # pragma: no cover - defensive
            self._send(500, {"error": {"code": "internal_error", "message": str(exc)}})

    def do_PUT(self) -> None:
        self._send_error_obj(MethodNotAllowed("PUT is not supported."))

    def do_DELETE(self) -> None:
        self._send_error_obj(MethodNotAllowed("DELETE is not supported."))

    # --- routing ----------------------------------------------------------

    def _route_get(self, path: str, query: dict) -> None:
        if path == "/api/health":
            self._send(200, {"status": "ok", "service": "credit-exception"})
            return
        if path == "/api/policy-rules":
            self._send(200, {"rules": catalog()})
            return
        if path == "/api/actors":
            self._send(200, {"actors": [a.public() for a in rbac.ACTORS.values()]})
            return
        if path == "/api/session":
            self._send(200, {"actor": self._actor().public()})
            return
        if path == "/api/exceptions":
            actor = self._actor()
            state = (query.get("state") or [None])[0]
            if state is not None and state not in ALL_STATES:
                raise ValidationError(
                    f"Unknown state filter '{state}'.",
                    details={"allowed": list(ALL_STATES)},
                )
            self._send(200, {"exceptions": self.service.list_exceptions(actor, state=state)})
            return
        if path == "/api/audit/verify":
            self._send(200, self.service.verify_audit(self._actor()))
            return

        match = EXC_PATH.match(path)
        if match:
            self._send(200, self.service.get_exception(self._actor(), match.group(1)))
            return
        match = EXC_ACTION.match(path)
        if match and match.group(2) == "audit":
            actor = self._actor()
            self._send(200, {"entries": self.service.get_audit(actor, match.group(1))})
            return
        raise NotFound(f"No such API route: {path}")

    def _route_post(self, path: str) -> None:
        actor = self._actor()
        body = self._read_json()
        command_id = self._command_id()

        operation, handler = self._resolve_command(path, actor, body)

        if command_id is not None:
            prior = self.service.store.lookup_command(
                actor.tenant_id, command_id, operation, body
            )
            if prior is not None:
                self._send(prior.status, prior.response,
                           {"X-Idempotent-Replay": "true"})
                return

        payload, status = handler()

        if command_id is not None:
            self.service.store.record_command(
                tenant_id=actor.tenant_id,
                command_id=command_id,
                actor_id=actor.actor_id,
                operation=operation,
                payload=body,
                response=payload,
                status=status,
                recorded_at=self.service.clock(),
            )
        self._send(status, payload)

    def _resolve_command(self, path: str, actor: rbac.Actor, body: dict):
        """Map a POST route to (operation-name, zero-arg handler)."""
        if path == "/api/exceptions":
            return "exception.create", lambda: self.service.create_exception(actor, body)

        match = EXC_ACTION.match(path)
        if not match:
            raise NotFound(f"No such API route: {path}")
        exception_id, action = match.group(1), match.group(2)
        version = body.get("expected_version")

        if action == "submit":
            return "exception.submit", lambda: self.service.submit_exception(
                actor, exception_id, version
            )
        if action == "analyze":
            return "exception.analyze", lambda: self.service.analyze_exception(
                actor, exception_id, version
            )
        if action == "withdraw":
            return "exception.withdraw", lambda: self.service.withdraw_exception(
                actor, exception_id, version, body.get("reason", "")
            )
        if action in ("risk-review", "compliance-review"):
            kind = APPROVAL_RISK if action == "risk-review" else APPROVAL_COMPLIANCE
            return f"exception.review.{kind}", lambda: self.service.record_review(
                actor,
                exception_id,
                kind=kind,
                decision=body.get("decision", ""),
                rationale=body.get("rationale", ""),
                expected_version=version,
            )
        if action == "decide":
            return "exception.decide", lambda: self.service.decide_exception(
                actor,
                exception_id,
                decision=body.get("decision", ""),
                rationale=body.get("rationale", ""),
                expected_version=version,
            )
        raise NotFound(f"No such action: {action}")
