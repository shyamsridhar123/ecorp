"""Application service: the governed exception workflow.

Every material transition here does four things in order under the store
lock: authorize, validate, mutate with a version bump, and append to the
tamper-evident audit chain.
"""

from __future__ import annotations

from datetime import datetime, timezone

from . import rbac
from .domain import (
    APPROVAL_AUTHORITY,
    APPROVAL_COMPLIANCE,
    APPROVAL_RISK,
    APPROVAL_TTL,
    STATE_APPROVED,
    STATE_COMPLIANCE_REVIEW,
    STATE_DRAFT,
    STATE_EXPIRED,
    STATE_PENDING_DECISION,
    STATE_REJECTED,
    STATE_RISK_REVIEW,
    STATE_SUBMITTED,
    STATE_WITHDRAWN,
    Approval,
    ExceptionRequest,
)
from .errors import (
    ApprovalExpired,
    Forbidden,
    InvalidState,
    MakerCheckerViolation,
    ValidationError,
    VersionConflict,
)
from .policy import POLICY_RULES, validate_exception_payload
from .rbac import Actor, TENANTS
from .store import Store


def utcnow() -> datetime:
    return datetime.now(timezone.utc)


class ExceptionService:
    def __init__(self, store: Store | None = None, clock=utcnow):
        self.store = store or Store()
        self.clock = clock

    # --- internals --------------------------------------------------------

    def _audit(
        self,
        record: ExceptionRequest,
        action: str,
        actor: Actor,
        now: datetime,
        payload: dict | None = None,
    ) -> None:
        base = {"state": record.state, "version": record.version}
        if payload:
            base.update(payload)
        self.store.audit.append(
            tenant_id=record.tenant_id,
            subject_id=record.exception_id,
            action=action,
            actor_id=actor.actor_id,
            recorded_at=now,
            payload=base,
        )

    @staticmethod
    def _check_version(record: ExceptionRequest, expected_version: int | None) -> None:
        if expected_version is None:
            raise ValidationError(
                "Field 'expected_version' is required for mutating operations.",
                details={"field": "expected_version", "current_version": record.version},
            )
        if not isinstance(expected_version, int) or isinstance(expected_version, bool):
            raise ValidationError(
                "Field 'expected_version' must be an integer.",
                details={"field": "expected_version"},
            )
        if expected_version != record.version:
            raise VersionConflict(
                f"Expected version {expected_version} but the record is at "
                f"version {record.version}. Reload and retry.",
                details={
                    "expected_version": expected_version,
                    "current_version": record.version,
                },
            )

    def _load_active(self, actor: Actor, exception_id: str, now: datetime) -> ExceptionRequest:
        """Load a record, lazily transitioning it to expired when due."""
        record = self.store.get(actor.tenant_id, exception_id)
        if (
            record.is_expired(now)
            and record.state not in (STATE_APPROVED, STATE_REJECTED, STATE_WITHDRAWN, STATE_EXPIRED)
        ):
            record.state = STATE_EXPIRED
            record.touch(now)
            self.store.put(record)
            self.store.audit.append(
                tenant_id=record.tenant_id,
                subject_id=record.exception_id,
                action="exception.expired",
                actor_id="system",
                recorded_at=now,
                payload={"state": record.state, "version": record.version,
                         "reason": "expiry_reached"},
            )
        return record

    # --- reads ------------------------------------------------------------

    def get_exception(self, actor: Actor, exception_id: str) -> dict:
        rbac.require(actor, rbac.CAP_READ)
        now = self.clock()
        record = self._load_active(actor, exception_id, now)
        return record.to_dict(now)

    def list_exceptions(self, actor: Actor, *, state: str | None = None) -> list[dict]:
        rbac.require(actor, rbac.CAP_READ)
        now = self.clock()
        records = self.store.list_for_tenant(actor.tenant_id, state=state)
        return [r.to_dict(now) for r in records]

    def get_audit(self, actor: Actor, exception_id: str) -> list[dict]:
        rbac.require(actor, rbac.CAP_READ)
        # Confirms existence within the caller's tenant before disclosing.
        self.store.get(actor.tenant_id, exception_id)
        entries = self.store.audit.for_subject(actor.tenant_id, exception_id)
        return [e.to_dict() for e in entries]

    def verify_audit(self, actor: Actor) -> dict:
        rbac.require(actor, rbac.CAP_READ)
        result = self.store.audit.verify()
        tenant_entries = self.store.audit.for_tenant(actor.tenant_id)
        return {
            **result,
            "tenant_entry_count": len(tenant_entries),
            "tenant_id": actor.tenant_id,
        }

    # --- commands ---------------------------------------------------------

    def create_exception(self, actor: Actor, payload: dict) -> tuple[dict, int]:
        rbac.require(actor, rbac.CAP_CREATE)
        now = self.clock()
        tenant = TENANTS[actor.tenant_id]
        validated = validate_exception_payload(
            payload, tenant_max_bps=tenant.max_deviation_bps, now=now
        )
        with self.store.lock:
            exception_id = self.store.next_exception_id(actor.tenant_id)
            record = ExceptionRequest(
                exception_id=exception_id,
                tenant_id=actor.tenant_id,
                applicant_pseudonym=validated.applicant_pseudonym,
                rule_id=validated.rule_id,
                requested_deviation_bps=validated.requested_deviation_bps,
                justification=validated.justification,
                compensating_controls=validated.compensating_controls,
                expires_at=validated.expires_at,
                requires_compliance=validated.requires_compliance,
                requested_by=actor.actor_id,
                created_at=now,
                updated_at=now,
                state=STATE_DRAFT,
                version=1,
            )
            self.store.put(record)
            self._audit(
                record,
                "exception.created",
                actor,
                now,
                {"rule_id": record.rule_id,
                 "requested_deviation_bps": record.requested_deviation_bps},
            )
        return record.to_dict(now), 201

    def submit_exception(
        self, actor: Actor, exception_id: str, expected_version: int | None
    ) -> tuple[dict, int]:
        rbac.require(actor, rbac.CAP_SUBMIT)
        now = self.clock()
        with self.store.lock:
            record = self._load_active(actor, exception_id, now)
            if record.requested_by != actor.actor_id:
                raise Forbidden(
                    "Only the originating requester may submit this exception.",
                    details={"requested_by": record.requested_by},
                )
            self._check_version(record, expected_version)
            record.ensure_state(STATE_DRAFT)
            record.state = STATE_SUBMITTED
            record.touch(now)
            self.store.put(record)
            self._audit(record, "exception.submitted", actor, now)
        return record.to_dict(now), 200

    def withdraw_exception(
        self, actor: Actor, exception_id: str, expected_version: int | None, reason: str
    ) -> tuple[dict, int]:
        rbac.require(actor, rbac.CAP_WITHDRAW)
        now = self.clock()
        if not isinstance(reason, str) or len(reason.strip()) < 5:
            raise ValidationError(
                "Field 'reason' must be at least 5 characters.",
                details={"field": "reason"},
            )
        with self.store.lock:
            record = self._load_active(actor, exception_id, now)
            if record.requested_by != actor.actor_id:
                raise Forbidden(
                    "Only the originating requester may withdraw this exception.",
                    details={"requested_by": record.requested_by},
                )
            self._check_version(record, expected_version)
            record.ensure_state(
                STATE_DRAFT, STATE_SUBMITTED, STATE_RISK_REVIEW,
                STATE_COMPLIANCE_REVIEW, STATE_PENDING_DECISION,
            )
            record.state = STATE_WITHDRAWN
            record.touch(now)
            self.store.put(record)
            self._audit(record, "exception.withdrawn", actor, now,
                        {"reason": reason.strip()})
        return record.to_dict(now), 200

    def analyze_exception(
        self, actor: Actor, exception_id: str, expected_version: int | None
    ) -> tuple[dict, int]:
        """Deterministic eligibility analysis; routes to the first review."""
        rbac.require(actor, rbac.CAP_READ)
        now = self.clock()
        with self.store.lock:
            record = self._load_active(actor, exception_id, now)
            self._check_version(record, expected_version)
            record.ensure_state(STATE_SUBMITTED)
            rule = POLICY_RULES[record.rule_id]
            findings = _analyze(record, rule)
            record.state = STATE_RISK_REVIEW
            record.touch(now)
            self.store.put(record)
            self._audit(record, "exception.analyzed", actor, now,
                        {"findings": findings})
        result = record.to_dict(now)
        result["analysis"] = findings
        return result, 200

    def record_review(
        self,
        actor: Actor,
        exception_id: str,
        *,
        kind: str,
        decision: str,
        rationale: str,
        expected_version: int | None,
    ) -> tuple[dict, int]:
        if kind == APPROVAL_RISK:
            rbac.require(actor, rbac.CAP_REVIEW_RISK)
            required_state = STATE_RISK_REVIEW
        elif kind == APPROVAL_COMPLIANCE:
            rbac.require(actor, rbac.CAP_REVIEW_COMPLIANCE)
            required_state = STATE_COMPLIANCE_REVIEW
        else:
            raise ValidationError(
                f"Unknown review kind '{kind}'.",
                details={"field": "kind", "allowed": [APPROVAL_RISK, APPROVAL_COMPLIANCE]},
            )
        if decision not in ("approve", "reject"):
            raise ValidationError(
                "Field 'decision' must be 'approve' or 'reject'.",
                details={"field": "decision"},
            )
        if not isinstance(rationale, str) or len(rationale.strip()) < 10:
            raise ValidationError(
                "Field 'rationale' must be at least 10 characters.",
                details={"field": "rationale"},
            )

        now = self.clock()
        with self.store.lock:
            record = self._load_active(actor, exception_id, now)
            self._check_version(record, expected_version)
            record.ensure_state(required_state)

            # Maker-checker: the requester may never review their own request,
            # and no actor may occupy two review seats on one request.
            if actor.actor_id == record.requested_by:
                raise MakerCheckerViolation(
                    "The requester may not review their own exception.",
                    details={"actor_id": actor.actor_id, "stage": kind},
                )
            for existing in record.approvals:
                if existing.actor_id == actor.actor_id:
                    raise MakerCheckerViolation(
                        f"Actor already recorded the '{existing.kind}' review; "
                        "a second review seat requires a different actor.",
                        details={"actor_id": actor.actor_id,
                                 "existing_stage": existing.kind, "stage": kind},
                    )
            if record.approval_of(kind) is not None:
                raise InvalidState(
                    f"A '{kind}' review is already recorded.",
                    details={"stage": kind},
                )

            record.approvals.append(
                Approval(
                    kind=kind,
                    actor_id=actor.actor_id,
                    decision=decision,
                    rationale=rationale.strip(),
                    recorded_at=now,
                    expires_at=min(now + APPROVAL_TTL, record.expires_at),
                )
            )
            if decision == "reject":
                record.state = STATE_REJECTED
                record.decided_by = actor.actor_id
                record.decided_at = now
                record.decision_rationale = rationale.strip()
            elif kind == APPROVAL_RISK and record.requires_compliance:
                record.state = STATE_COMPLIANCE_REVIEW
            else:
                record.state = STATE_PENDING_DECISION
            record.touch(now)
            self.store.put(record)
            self._audit(record, f"review.{kind}.{decision}", actor, now,
                        {"rationale": rationale.strip()})
        return record.to_dict(now), 200

    def decide_exception(
        self,
        actor: Actor,
        exception_id: str,
        *,
        decision: str,
        rationale: str,
        expected_version: int | None,
    ) -> tuple[dict, int]:
        rbac.require(actor, rbac.CAP_DECIDE)
        if decision not in ("approve", "reject"):
            raise ValidationError(
                "Field 'decision' must be 'approve' or 'reject'.",
                details={"field": "decision"},
            )
        if not isinstance(rationale, str) or len(rationale.strip()) < 10:
            raise ValidationError(
                "Field 'rationale' must be at least 10 characters.",
                details={"field": "rationale"},
            )
        now = self.clock()
        with self.store.lock:
            record = self._load_active(actor, exception_id, now)
            self._check_version(record, expected_version)
            record.ensure_state(STATE_PENDING_DECISION)

            if actor.actor_id in record.participating_actor_ids():
                raise MakerCheckerViolation(
                    "The final authority must be independent of the requester "
                    "and every reviewer on this exception.",
                    details={
                        "actor_id": actor.actor_id,
                        "participants": sorted(record.participating_actor_ids()),
                    },
                )

            # Expired review approvals may never be reused to authorize.
            stale = [a for a in record.approvals if a.is_expired(now)]
            if stale and decision == "approve":
                raise ApprovalExpired(
                    "One or more review approvals have expired and cannot be "
                    "reused; the exception must be re-reviewed.",
                    details={"expired_stages": sorted(a.kind for a in stale)},
                )
            if decision == "approve":
                missing = [
                    k for k in record.required_approval_kinds()
                    if (a := record.approval_of(k)) is None or a.decision != "approve"
                ]
                if missing:
                    raise InvalidState(
                        "Required approvals are missing.",
                        details={"missing_stages": missing},
                    )

            record.state = STATE_APPROVED if decision == "approve" else STATE_REJECTED
            record.decided_by = actor.actor_id
            record.decided_at = now
            record.decision_rationale = rationale.strip()
            record.approvals.append(
                Approval(
                    kind=APPROVAL_AUTHORITY,
                    actor_id=actor.actor_id,
                    decision=decision,
                    rationale=rationale.strip(),
                    recorded_at=now,
                    expires_at=record.expires_at,
                )
            )
            record.touch(now)
            self.store.put(record)
            self._audit(record, f"decision.{decision}", actor, now,
                        {"rationale": rationale.strip()})
        return record.to_dict(now), 200


def _analyze(record: ExceptionRequest, rule) -> dict:
    """Pure, deterministic scoring used to explain the routing decision."""
    utilization = round(
        100.0 * record.requested_deviation_bps / max(rule.max_deviation_bps, 1), 2
    )
    if utilization >= 80:
        band = "high"
    elif utilization >= 40:
        band = "moderate"
    else:
        band = "low"
    return {
        "rule_id": rule.rule_id,
        "rule_max_deviation_bps": rule.max_deviation_bps,
        "requested_deviation_bps": record.requested_deviation_bps,
        "ceiling_utilization_pct": utilization,
        "risk_band": band,
        "compensating_control_count": len(record.compensating_controls),
        "requires_compliance": record.requires_compliance,
        "required_approval_kinds": list(record.required_approval_kinds()),
    }
