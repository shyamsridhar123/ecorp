"""In-memory, thread-safe persistence with tenant scoping and idempotency.

Storage is process-local by design: the scenario forbids external
dependencies, and a dict behind a re-entrant lock is enough to demonstrate
tenant isolation, optimistic concurrency, and command idempotency
faithfully.
"""

from __future__ import annotations

import hashlib
import threading
from dataclasses import dataclass
from datetime import datetime

from .audit import AuditChain, canonical_json
from .domain import ExceptionRequest
from .errors import IdempotencyMismatch, NotFound


def fingerprint(value: object) -> str:
    return hashlib.sha256(canonical_json(value).encode("utf-8")).hexdigest()


@dataclass
class CommandRecord:
    """A completed command, keyed by (tenant, command_id)."""

    command_id: str
    tenant_id: str
    actor_id: str
    operation: str
    payload_fingerprint: str
    response: dict
    status: int
    recorded_at: str


class Store:
    def __init__(self) -> None:
        self._lock = threading.RLock()
        self._exceptions: dict[tuple[str, str], ExceptionRequest] = {}
        self._commands: dict[tuple[str, str], CommandRecord] = {}
        self._counter = 0
        self.audit = AuditChain()

    # --- identity ---------------------------------------------------------

    def next_exception_id(self, tenant_id: str) -> str:
        with self._lock:
            self._counter += 1
            return f"EXC-{tenant_id.split('-')[-1][:4].upper()}-{self._counter:05d}"

    # --- exceptions -------------------------------------------------------

    def put(self, record: ExceptionRequest) -> None:
        with self._lock:
            self._exceptions[(record.tenant_id, record.exception_id)] = record

    def get(self, tenant_id: str, exception_id: str) -> ExceptionRequest:
        """Tenant-scoped lookup.

        A record belonging to another tenant is reported as *not found*,
        never as forbidden — cross-tenant existence must not leak.
        """
        with self._lock:
            record = self._exceptions.get((tenant_id, exception_id))
        if record is None:
            raise NotFound(
                f"Exception '{exception_id}' was not found.",
                details={"exception_id": exception_id},
            )
        return record

    def list_for_tenant(
        self, tenant_id: str, *, state: str | None = None
    ) -> list[ExceptionRequest]:
        with self._lock:
            records = [
                r for (t, _), r in self._exceptions.items() if t == tenant_id
            ]
        if state:
            records = [r for r in records if r.state == state]
        return sorted(records, key=lambda r: r.exception_id)

    def count(self) -> int:
        with self._lock:
            return len(self._exceptions)

    # --- idempotent commands ---------------------------------------------

    def lookup_command(
        self, tenant_id: str, command_id: str, operation: str, payload: object
    ) -> CommandRecord | None:
        """Return a prior result for this command, or ``None`` if new.

        Replaying a command id with a different operation or payload is a
        client bug and raises :class:`IdempotencyMismatch` rather than
        silently returning the old response.
        """
        with self._lock:
            record = self._commands.get((tenant_id, command_id))
        if record is None:
            return None
        expected = fingerprint({"operation": operation, "payload": payload})
        if record.payload_fingerprint != expected:
            raise IdempotencyMismatch(
                f"Command '{command_id}' was already used with a different payload.",
                details={
                    "command_id": command_id,
                    "original_operation": record.operation,
                    "attempted_operation": operation,
                },
            )
        return record

    def record_command(
        self,
        *,
        tenant_id: str,
        command_id: str,
        actor_id: str,
        operation: str,
        payload: object,
        response: dict,
        status: int,
        recorded_at: datetime,
    ) -> CommandRecord:
        record = CommandRecord(
            command_id=command_id,
            tenant_id=tenant_id,
            actor_id=actor_id,
            operation=operation,
            payload_fingerprint=fingerprint(
                {"operation": operation, "payload": payload}
            ),
            response=response,
            status=status,
            recorded_at=recorded_at.isoformat(),
        )
        with self._lock:
            self._commands[(tenant_id, command_id)] = record
        return record

    def command_count(self) -> int:
        with self._lock:
            return len(self._commands)

    # --- test / operational support --------------------------------------

    @property
    def lock(self) -> threading.RLock:
        return self._lock

    def reset(self) -> None:
        with self._lock:
            self._exceptions.clear()
            self._commands.clear()
            self._counter = 0
            self.audit = AuditChain()
