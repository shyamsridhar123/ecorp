"""Structured error taxonomy for the credit-policy exception service.

Every failure surfaced by the API is an :class:`AppError`. The HTTP layer
renders it as a stable JSON envelope so clients can branch on ``code``
rather than parsing prose.
"""

from __future__ import annotations

from typing import Any


class AppError(Exception):
    """Base class for every deterministic, client-visible failure."""

    status = 400
    code = "bad_request"

    def __init__(self, message: str, *, details: Any = None, code: str | None = None):
        super().__init__(message)
        self.message = message
        self.details = details
        if code is not None:
            self.code = code

    def to_payload(self) -> dict:
        body: dict[str, Any] = {"error": {"code": self.code, "message": self.message}}
        if self.details is not None:
            body["error"]["details"] = self.details
        return body


class ValidationError(AppError):
    status = 422
    code = "validation_failed"


class PolicyViolation(AppError):
    """Request is well-formed but forbidden by deterministic credit policy."""

    status = 422
    code = "policy_violation"


class NotFound(AppError):
    status = 404
    code = "not_found"


class Unauthenticated(AppError):
    status = 401
    code = "unauthenticated"


class Forbidden(AppError):
    status = 403
    code = "forbidden"


class TenantMismatch(Forbidden):
    code = "tenant_mismatch"


class MakerCheckerViolation(Forbidden):
    code = "maker_checker_violation"


class Conflict(AppError):
    status = 409
    code = "conflict"


class VersionConflict(Conflict):
    code = "version_conflict"


class InvalidState(Conflict):
    code = "invalid_state"


class ApprovalExpired(Conflict):
    code = "approval_expired"


class IdempotencyMismatch(Conflict):
    """Same command id replayed with a different payload fingerprint."""

    code = "idempotency_mismatch"


class MethodNotAllowed(AppError):
    status = 405
    code = "method_not_allowed"
