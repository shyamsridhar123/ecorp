"""Append-only, tamper-evident audit chain.

Each entry commits to its predecessor via SHA-256 over a canonical JSON
encoding. Any mutation, reordering, insertion, or deletion breaks
verification at the first affected index.
"""

from __future__ import annotations

import hashlib
import json
from dataclasses import dataclass, field
from datetime import datetime
from typing import Any

GENESIS_HASH = "0" * 64


def canonical_json(value: Any) -> str:
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False)


def compute_entry_hash(
    *,
    sequence: int,
    prev_hash: str,
    tenant_id: str,
    subject_id: str,
    action: str,
    actor_id: str,
    recorded_at: str,
    payload: dict,
) -> str:
    material = canonical_json(
        {
            "sequence": sequence,
            "prev_hash": prev_hash,
            "tenant_id": tenant_id,
            "subject_id": subject_id,
            "action": action,
            "actor_id": actor_id,
            "recorded_at": recorded_at,
            "payload": payload,
        }
    )
    return hashlib.sha256(material.encode("utf-8")).hexdigest()


@dataclass
class AuditEntry:
    sequence: int
    prev_hash: str
    entry_hash: str
    tenant_id: str
    subject_id: str
    action: str
    actor_id: str
    recorded_at: str
    payload: dict = field(default_factory=dict)

    def to_dict(self) -> dict:
        return {
            "sequence": self.sequence,
            "prev_hash": self.prev_hash,
            "entry_hash": self.entry_hash,
            "tenant_id": self.tenant_id,
            "subject_id": self.subject_id,
            "action": self.action,
            "actor_id": self.actor_id,
            "recorded_at": self.recorded_at,
            "payload": self.payload,
        }

    def recompute(self) -> str:
        return compute_entry_hash(
            sequence=self.sequence,
            prev_hash=self.prev_hash,
            tenant_id=self.tenant_id,
            subject_id=self.subject_id,
            action=self.action,
            actor_id=self.actor_id,
            recorded_at=self.recorded_at,
            payload=self.payload,
        )


class AuditChain:
    """Single global append-only chain covering all tenants.

    A global chain (rather than per-tenant) makes cross-tenant deletion
    detectable too. Reads are always tenant-scoped by the caller.
    """

    def __init__(self) -> None:
        self._entries: list[AuditEntry] = []

    def append(
        self,
        *,
        tenant_id: str,
        subject_id: str,
        action: str,
        actor_id: str,
        recorded_at: datetime,
        payload: dict | None = None,
    ) -> AuditEntry:
        payload = payload or {}
        sequence = len(self._entries) + 1
        prev_hash = self._entries[-1].entry_hash if self._entries else GENESIS_HASH
        stamp = recorded_at.isoformat()
        entry_hash = compute_entry_hash(
            sequence=sequence,
            prev_hash=prev_hash,
            tenant_id=tenant_id,
            subject_id=subject_id,
            action=action,
            actor_id=actor_id,
            recorded_at=stamp,
            payload=payload,
        )
        entry = AuditEntry(
            sequence=sequence,
            prev_hash=prev_hash,
            entry_hash=entry_hash,
            tenant_id=tenant_id,
            subject_id=subject_id,
            action=action,
            actor_id=actor_id,
            recorded_at=stamp,
            payload=payload,
        )
        self._entries.append(entry)
        return entry

    def all_entries(self) -> list[AuditEntry]:
        return list(self._entries)

    def for_subject(self, tenant_id: str, subject_id: str) -> list[AuditEntry]:
        return [
            e
            for e in self._entries
            if e.tenant_id == tenant_id and e.subject_id == subject_id
        ]

    def for_tenant(self, tenant_id: str) -> list[AuditEntry]:
        return [e for e in self._entries if e.tenant_id == tenant_id]

    def head_hash(self) -> str:
        return self._entries[-1].entry_hash if self._entries else GENESIS_HASH

    def verify(self) -> dict:
        """Walk the chain and report the first structural break, if any."""
        prev = GENESIS_HASH
        for index, entry in enumerate(self._entries, start=1):
            if entry.sequence != index:
                return _broken(index, "sequence_gap", entry)
            if entry.prev_hash != prev:
                return _broken(index, "prev_hash_mismatch", entry)
            if entry.recompute() != entry.entry_hash:
                return _broken(index, "entry_hash_mismatch", entry)
            prev = entry.entry_hash
        return {
            "valid": True,
            "entry_count": len(self._entries),
            "head_hash": prev,
            "broken_at": None,
            "reason": None,
        }


def _broken(index: int, reason: str, entry: AuditEntry) -> dict:
    return {
        "valid": False,
        "entry_count": index,
        "head_hash": None,
        "broken_at": index,
        "reason": reason,
        "subject_id": entry.subject_id,
    }
