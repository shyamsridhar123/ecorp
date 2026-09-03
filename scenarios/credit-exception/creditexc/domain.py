"""Durable domain model: exception aggregate, states, and approvals."""

from __future__ import annotations

from dataclasses import dataclass, field
from datetime import datetime, timedelta, timezone

from .errors import InvalidState

# --- Durable lifecycle states ---------------------------------------------

STATE_DRAFT = "draft"
STATE_SUBMITTED = "submitted"
STATE_RISK_REVIEW = "risk_review"
STATE_COMPLIANCE_REVIEW = "compliance_review"
STATE_PENDING_DECISION = "pending_decision"
STATE_APPROVED = "approved"
STATE_REJECTED = "rejected"
STATE_WITHDRAWN = "withdrawn"
STATE_EXPIRED = "expired"

TERMINAL_STATES = frozenset(
    {STATE_APPROVED, STATE_REJECTED, STATE_WITHDRAWN, STATE_EXPIRED}
)

ALL_STATES = (
    STATE_DRAFT,
    STATE_SUBMITTED,
    STATE_RISK_REVIEW,
    STATE_COMPLIANCE_REVIEW,
    STATE_PENDING_DECISION,
    STATE_APPROVED,
    STATE_REJECTED,
    STATE_WITHDRAWN,
    STATE_EXPIRED,
)

# Approval kinds
APPROVAL_RISK = "risk"
APPROVAL_COMPLIANCE = "compliance"
APPROVAL_AUTHORITY = "authority"

# How long a recorded review approval stays usable before the final
# decision must be taken. Deliberately short so expiry is testable.
APPROVAL_TTL = timedelta(hours=72)


@dataclass
class Approval:
    """A durable, individually-expiring review decision."""

    kind: str
    actor_id: str
    decision: str  # "approve" | "reject"
    rationale: str
    recorded_at: datetime
    expires_at: datetime

    def is_expired(self, now: datetime) -> bool:
        return now >= self.expires_at

    def to_dict(self, now: datetime | None = None) -> dict:
        data = {
            "kind": self.kind,
            "actor_id": self.actor_id,
            "decision": self.decision,
            "rationale": self.rationale,
            "recorded_at": self.recorded_at.isoformat(),
            "expires_at": self.expires_at.isoformat(),
        }
        if now is not None:
            data["expired"] = self.is_expired(now)
        return data


@dataclass
class ExceptionRequest:
    """The aggregate root. ``version`` drives optimistic concurrency."""

    exception_id: str
    tenant_id: str
    applicant_pseudonym: str
    rule_id: str
    requested_deviation_bps: int
    justification: str
    compensating_controls: tuple[str, ...]
    expires_at: datetime
    requires_compliance: bool
    requested_by: str
    created_at: datetime
    updated_at: datetime
    state: str = STATE_DRAFT
    version: int = 1
    approvals: list[Approval] = field(default_factory=list)
    decision_rationale: str | None = None
    decided_by: str | None = None
    decided_at: datetime | None = None

    # --- helpers ----------------------------------------------------------

    def approval_of(self, kind: str) -> Approval | None:
        for approval in self.approvals:
            if approval.kind == kind:
                return approval
        return None

    def required_approval_kinds(self) -> tuple[str, ...]:
        if self.requires_compliance:
            return (APPROVAL_RISK, APPROVAL_COMPLIANCE)
        return (APPROVAL_RISK,)

    def participating_actor_ids(self) -> set[str]:
        return {self.requested_by} | {a.actor_id for a in self.approvals}

    def is_expired(self, now: datetime) -> bool:
        return now >= self.expires_at

    def live_approvals(self, now: datetime) -> list[Approval]:
        return [a for a in self.approvals if not a.is_expired(now)]

    def expired_approvals(self, now: datetime) -> list[Approval]:
        return [a for a in self.approvals if a.is_expired(now)]

    def ready_for_decision(self, now: datetime) -> bool:
        if self.state != STATE_PENDING_DECISION:
            return False
        for kind in self.required_approval_kinds():
            approval = self.approval_of(kind)
            if approval is None or approval.is_expired(now):
                return False
            if approval.decision != "approve":
                return False
        return True

    def ensure_state(self, *allowed: str) -> None:
        if self.state not in allowed:
            raise InvalidState(
                f"Exception is in state '{self.state}'; expected one of "
                f"{sorted(allowed)}.",
                details={"current_state": self.state, "allowed_states": sorted(allowed)},
            )

    def touch(self, now: datetime) -> None:
        self.version += 1
        self.updated_at = now

    def to_dict(self, now: datetime | None = None) -> dict:
        now = now or datetime.now(timezone.utc)
        return {
            "exception_id": self.exception_id,
            "tenant_id": self.tenant_id,
            "applicant_pseudonym": self.applicant_pseudonym,
            "rule_id": self.rule_id,
            "requested_deviation_bps": self.requested_deviation_bps,
            "justification": self.justification,
            "compensating_controls": list(self.compensating_controls),
            "expires_at": self.expires_at.isoformat(),
            "requires_compliance": self.requires_compliance,
            "requested_by": self.requested_by,
            "created_at": self.created_at.isoformat(),
            "updated_at": self.updated_at.isoformat(),
            "state": self.state,
            "version": self.version,
            "approvals": [a.to_dict(now) for a in self.approvals],
            "required_approval_kinds": list(self.required_approval_kinds()),
            "ready_for_decision": self.ready_for_decision(now),
            "expired": self.is_expired(now),
            "decision_rationale": self.decision_rationale,
            "decided_by": self.decided_by,
            "decided_at": self.decided_at.isoformat() if self.decided_at else None,
        }
