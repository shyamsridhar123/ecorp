"""Dependency-free unit and integration tests for the credit-exception scenario.

Run:  python -m unittest discover -s tests -v
      (from ``scenarios/credit-exception``)

Integration tests bind a real HTTP server on an ephemeral port and speak to
it with ``urllib`` — no third-party test client.
"""

from __future__ import annotations

import json
import sys
import threading
import unittest
import urllib.error
import urllib.request
from datetime import datetime, timedelta, timezone
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from creditexc import build_server, rbac  # noqa: E402
from creditexc.audit import GENESIS_HASH  # noqa: E402
from creditexc.domain import (  # noqa: E402
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
)
from creditexc.errors import (  # noqa: E402
    ApprovalExpired,
    Forbidden,
    IdempotencyMismatch,
    InvalidState,
    MakerCheckerViolation,
    NotFound,
    PolicyViolation,
    Unauthenticated,
    ValidationError,
    VersionConflict,
)
from creditexc.service import ExceptionService  # noqa: E402

BASE = datetime(2026, 9, 3, 12, 0, 0, tzinfo=timezone.utc)


class MovableClock:
    def __init__(self, start: datetime = BASE):
        self.now = start

    def __call__(self) -> datetime:
        return self.now

    def advance(self, **kwargs) -> None:
        self.now += timedelta(**kwargs)


def payload(**overrides) -> dict:
    body = {
        "applicant_pseudonym": "APP-QX7781",
        "rule_id": "CP-101",
        "requested_deviation_bps": 1200,
        "justification": "Seasonal revenue timing depresses the ratio for one quarter only.",
        "compensating_controls": ["Quarterly covenant monitoring by risk team"],
        "expires_at": (BASE + timedelta(days=30)).isoformat(),
    }
    body.update(overrides)
    return body


def actor(name: str) -> rbac.Actor:
    return rbac.ACTORS[name]


class ServiceTestBase(unittest.TestCase):
    def setUp(self) -> None:
        self.clock = MovableClock()
        self.svc = ExceptionService(clock=self.clock)

    def make_draft(self, who: str = "nw-requester-1", **overrides) -> dict:
        record, status = self.svc.create_exception(actor(who), payload(**overrides))
        self.assertEqual(status, 201)
        return record

    def drive_to_pending(self, who: str = "nw-requester-1") -> dict:
        rec = self.make_draft(who)
        eid = rec["exception_id"]
        rec, _ = self.svc.submit_exception(actor(who), eid, rec["version"])
        rec, _ = self.svc.analyze_exception(actor("nw-risk-1"), eid, rec["version"])
        rec, _ = self.svc.record_review(
            actor("nw-risk-1"), eid, kind="risk", decision="approve",
            rationale="Risk accepts the seasonal argument.", expected_version=rec["version"],
        )
        rec, _ = self.svc.record_review(
            actor("nw-compliance-1"), eid, kind="compliance", decision="approve",
            rationale="Compliance has no objection.", expected_version=rec["version"],
        )
        self.assertEqual(rec["state"], STATE_PENDING_DECISION)
        return rec


# --------------------------------------------------------------------------
# Validation and deterministic policy
# --------------------------------------------------------------------------


class TestValidationAndPolicy(ServiceTestBase):
    def test_happy_path_create(self):
        rec = self.make_draft()
        self.assertEqual(rec["state"], STATE_DRAFT)
        self.assertEqual(rec["version"], 1)
        self.assertTrue(rec["exception_id"].startswith("EXC-"))
        self.assertEqual(rec["required_approval_kinds"], ["risk", "compliance"])

    def test_missing_field_rejected(self):
        body = payload()
        del body["justification"]
        with self.assertRaises(ValidationError) as ctx:
            self.svc.create_exception(actor("nw-requester-1"), body)
        self.assertEqual(ctx.exception.details["field"], "justification")

    def test_short_justification_rejected(self):
        with self.assertRaises(ValidationError):
            self.svc.create_exception(
                actor("nw-requester-1"), payload(justification="too short")
            )

    def test_empty_controls_rejected(self):
        with self.assertRaises(ValidationError):
            self.svc.create_exception(
                actor("nw-requester-1"), payload(compensating_controls=[])
            )

    def test_duplicate_controls_rejected(self):
        with self.assertRaises(ValidationError):
            self.svc.create_exception(
                actor("nw-requester-1"),
                payload(compensating_controls=[
                    "Quarterly covenant monitoring", "quarterly covenant monitoring",
                ]),
            )

    def test_real_identifier_shape_rejected(self):
        """Data minimization: only APP-XXXXXX pseudonyms are accepted."""
        with self.assertRaises(ValidationError) as ctx:
            self.svc.create_exception(
                actor("nw-requester-1"), payload(applicant_pseudonym="Jane Q Public")
            )
        self.assertEqual(ctx.exception.details["field"], "applicant_pseudonym")

    def test_prohibited_rule_rejected(self):
        with self.assertRaises(PolicyViolation) as ctx:
            self.svc.create_exception(actor("nw-requester-1"), payload(rule_id="CP-900"))
        self.assertEqual(ctx.exception.details["reason"], "prohibited_rule")

    def test_statutory_rule_rejected(self):
        with self.assertRaises(PolicyViolation):
            self.svc.create_exception(actor("nw-requester-1"), payload(rule_id="CP-901"))

    def test_deviation_over_rule_ceiling_rejected(self):
        with self.assertRaises(PolicyViolation) as ctx:
            self.svc.create_exception(
                actor("nw-requester-1"),
                payload(rule_id="CP-201", requested_deviation_bps=1400),
            )
        self.assertEqual(ctx.exception.details["reason"], "rule_ceiling_exceeded")

    def test_deviation_over_tenant_ceiling_rejected(self):
        """Cascadia caps at 1500 bps even though CP-101 allows 2000."""
        with self.assertRaises(PolicyViolation) as ctx:
            self.svc.create_exception(
                actor("cs-requester-1"), payload(requested_deviation_bps=1800)
            )
        self.assertEqual(ctx.exception.details["reason"], "tenant_ceiling_exceeded")

    def test_same_request_allowed_for_larger_tenant(self):
        rec = self.svc.create_exception(
            actor("nw-requester-1"), payload(requested_deviation_bps=1800)
        )[0]
        self.assertEqual(rec["requested_deviation_bps"], 1800)

    def test_negative_deviation_rejected(self):
        with self.assertRaises(ValidationError):
            self.svc.create_exception(
                actor("nw-requester-1"), payload(requested_deviation_bps=-5)
            )

    def test_boolean_deviation_rejected(self):
        with self.assertRaises(ValidationError):
            self.svc.create_exception(
                actor("nw-requester-1"), payload(requested_deviation_bps=True)
            )

    def test_past_expiry_rejected(self):
        with self.assertRaises(ValidationError):
            self.svc.create_exception(
                actor("nw-requester-1"),
                payload(expires_at=(BASE - timedelta(days=1)).isoformat()),
            )

    def test_expiry_beyond_horizon_rejected(self):
        with self.assertRaises(PolicyViolation) as ctx:
            self.svc.create_exception(
                actor("nw-requester-1"),
                payload(expires_at=(BASE + timedelta(days=400)).isoformat()),
            )
        self.assertEqual(ctx.exception.details["reason"], "expiry_horizon_exceeded")

    def test_unknown_rule_rejected(self):
        with self.assertRaises(ValidationError):
            self.svc.create_exception(actor("nw-requester-1"), payload(rule_id="CP-999"))

    def test_risk_only_rule_skips_compliance(self):
        rec = self.make_draft(rule_id="CP-201", requested_deviation_bps=500)
        self.assertEqual(rec["required_approval_kinds"], ["risk"])

    def test_analysis_is_deterministic(self):
        a = self.make_draft()
        b = self.make_draft()
        for rec in (a, b):
            self.svc.submit_exception(actor("nw-requester-1"), rec["exception_id"], 1)
        out_a = self.svc.analyze_exception(actor("nw-risk-1"), a["exception_id"], 2)[0]
        out_b = self.svc.analyze_exception(actor("nw-risk-1"), b["exception_id"], 2)[0]
        self.assertEqual(out_a["analysis"], out_b["analysis"])
        self.assertEqual(out_a["analysis"]["ceiling_utilization_pct"], 60.0)
        self.assertEqual(out_a["analysis"]["risk_band"], "moderate")


# --------------------------------------------------------------------------
# Tenant isolation
# --------------------------------------------------------------------------


class TestTenantIsolation(ServiceTestBase):
    def test_other_tenant_cannot_read(self):
        rec = self.make_draft()
        with self.assertRaises(NotFound):
            self.svc.get_exception(actor("cs-requester-1"), rec["exception_id"])

    def test_other_tenant_cannot_act(self):
        rec = self.make_draft()
        with self.assertRaises(NotFound):
            self.svc.submit_exception(actor("cs-requester-1"), rec["exception_id"], 1)

    def test_listing_is_tenant_scoped(self):
        self.make_draft("nw-requester-1")
        self.make_draft("nw-requester-2")
        self.svc.create_exception(actor("cs-requester-1"), payload())
        self.assertEqual(len(self.svc.list_exceptions(actor("nw-auditor-1"))), 2)
        self.assertEqual(len(self.svc.list_exceptions(actor("cs-requester-1"))), 1)

    def test_cross_tenant_audit_read_denied(self):
        rec = self.make_draft()
        with self.assertRaises(NotFound):
            self.svc.get_audit(actor("cs-compliance-1"), rec["exception_id"])

    def test_cross_tenant_existence_not_leaked(self):
        """A foreign record must look identical to a nonexistent one."""
        rec = self.make_draft()
        with self.assertRaises(NotFound) as real:
            self.svc.get_exception(actor("cs-requester-1"), rec["exception_id"])
        with self.assertRaises(NotFound) as fake:
            self.svc.get_exception(actor("cs-requester-1"), "EXC-NORT-99999")
        self.assertEqual(real.exception.code, fake.exception.code)
        self.assertEqual(real.exception.status, fake.exception.status)

    def test_tenant_audit_counts_are_scoped(self):
        self.make_draft("nw-requester-1")
        self.svc.create_exception(actor("cs-requester-1"), payload())
        nw = self.svc.verify_audit(actor("nw-auditor-1"))
        cs = self.svc.verify_audit(actor("cs-requester-1"))
        self.assertEqual(nw["tenant_entry_count"], 1)
        self.assertEqual(cs["tenant_entry_count"], 1)
        self.assertEqual(nw["entry_count"], 2)  # global chain sees both


# --------------------------------------------------------------------------
# RBAC / negative authorization
# --------------------------------------------------------------------------


class TestAuthorization(ServiceTestBase):
    def test_unknown_token_rejected(self):
        with self.assertRaises(Unauthenticated):
            rbac.authenticate("not-a-real-actor")

    def test_missing_token_rejected(self):
        with self.assertRaises(Unauthenticated):
            rbac.authenticate(None)

    def test_reviewer_cannot_create(self):
        with self.assertRaises(Forbidden):
            self.svc.create_exception(actor("nw-risk-1"), payload())

    def test_auditor_cannot_create(self):
        with self.assertRaises(Forbidden):
            self.svc.create_exception(actor("nw-auditor-1"), payload())

    def test_requester_cannot_perform_risk_review(self):
        rec = self.make_draft()
        eid = rec["exception_id"]
        rec, _ = self.svc.submit_exception(actor("nw-requester-1"), eid, 1)
        rec, _ = self.svc.analyze_exception(actor("nw-risk-1"), eid, rec["version"])
        with self.assertRaises(Forbidden):
            self.svc.record_review(
                actor("nw-requester-2"), eid, kind="risk", decision="approve",
                rationale="not my seat to fill", expected_version=rec["version"],
            )

    def test_risk_reviewer_cannot_do_compliance_review(self):
        rec = self.make_draft()
        eid = rec["exception_id"]
        rec, _ = self.svc.submit_exception(actor("nw-requester-1"), eid, 1)
        rec, _ = self.svc.analyze_exception(actor("nw-risk-1"), eid, rec["version"])
        rec, _ = self.svc.record_review(
            actor("nw-risk-1"), eid, kind="risk", decision="approve",
            rationale="risk is comfortable", expected_version=rec["version"],
        )
        with self.assertRaises(Forbidden):
            self.svc.record_review(
                actor("nw-risk-1"), eid, kind="compliance", decision="approve",
                rationale="wrong role entirely", expected_version=rec["version"],
            )

    def test_reviewer_cannot_make_final_decision(self):
        rec = self.drive_to_pending()
        with self.assertRaises(Forbidden):
            self.svc.decide_exception(
                actor("nw-risk-1"), rec["exception_id"], decision="approve",
                rationale="not the authority", expected_version=rec["version"],
            )

    def test_non_owner_requester_cannot_submit(self):
        rec = self.make_draft("nw-requester-1")
        with self.assertRaises(Forbidden):
            self.svc.submit_exception(actor("nw-requester-2"), rec["exception_id"], 1)

    def test_non_owner_cannot_withdraw(self):
        rec = self.make_draft("nw-requester-1")
        with self.assertRaises(Forbidden):
            self.svc.withdraw_exception(
                actor("nw-requester-2"), rec["exception_id"], 1, "not mine to pull"
            )

    def test_capability_matrix_is_least_privilege(self):
        self.assertNotIn(rbac.CAP_DECIDE, actor("nw-risk-1").capabilities)
        self.assertNotIn(rbac.CAP_CREATE, actor("nw-authority-1").capabilities)
        self.assertNotIn(rbac.CAP_REVIEW_RISK, actor("nw-compliance-1").capabilities)
        self.assertNotIn(rbac.CAP_CREATE, actor("nw-auditor-1").capabilities)


# --------------------------------------------------------------------------
# Maker-checker separation
# --------------------------------------------------------------------------


class TestMakerChecker(ServiceTestBase):
    def test_requester_cannot_self_approve_as_authority(self):
        """The dual-hat actor holds both roles; separation must still bind."""
        rec = self.make_draft("nw-dual-1")
        eid = rec["exception_id"]
        rec, _ = self.svc.submit_exception(actor("nw-dual-1"), eid, 1)
        rec, _ = self.svc.analyze_exception(actor("nw-risk-1"), eid, rec["version"])
        rec, _ = self.svc.record_review(
            actor("nw-risk-1"), eid, kind="risk", decision="approve",
            rationale="risk signs off here", expected_version=rec["version"],
        )
        rec, _ = self.svc.record_review(
            actor("nw-compliance-1"), eid, kind="compliance", decision="approve",
            rationale="compliance signs off", expected_version=rec["version"],
        )
        with self.assertRaises(MakerCheckerViolation) as ctx:
            self.svc.decide_exception(
                actor("nw-dual-1"), eid, decision="approve",
                rationale="approving my own request", expected_version=rec["version"],
            )
        self.assertIn("nw-dual-1", ctx.exception.details["participants"])

    def test_reviewer_cannot_also_be_final_authority(self):
        rec = self.drive_to_pending()
        with self.assertRaises(Forbidden):
            self.svc.decide_exception(
                actor("nw-compliance-1"), rec["exception_id"], decision="approve",
                rationale="already reviewed this", expected_version=rec["version"],
            )

    def test_same_actor_cannot_fill_two_review_seats(self):
        """A hypothetical dual-role reviewer still cannot double-sign."""
        rec = self.make_draft()
        eid = rec["exception_id"]
        rec, _ = self.svc.submit_exception(actor("nw-requester-1"), eid, 1)
        rec, _ = self.svc.analyze_exception(actor("nw-risk-1"), eid, rec["version"])
        rec, _ = self.svc.record_review(
            actor("nw-risk-1"), eid, kind="risk", decision="approve",
            rationale="risk approves this", expected_version=rec["version"],
        )
        dual = rbac.Actor(
            "nw-risk-1", "tenant-northwind", "N. Risk Reviewer",
            (rbac.ROLE_RISK_REVIEWER, rbac.ROLE_COMPLIANCE_REVIEWER),
        )
        with self.assertRaises(MakerCheckerViolation):
            self.svc.record_review(
                dual, eid, kind="compliance", decision="approve",
                rationale="second seat by same person", expected_version=rec["version"],
            )

    def test_independent_authority_can_approve(self):
        rec = self.drive_to_pending()
        final, _ = self.svc.decide_exception(
            actor("nw-authority-1"), rec["exception_id"], decision="approve",
            rationale="Independent authority authorizes.", expected_version=rec["version"],
        )
        self.assertEqual(final["state"], STATE_APPROVED)
        self.assertEqual(final["decided_by"], "nw-authority-1")

    def test_requester_cannot_review_own_request(self):
        dual_requester = rbac.Actor(
            "nw-requester-1", "tenant-northwind", "N. Requester One",
            (rbac.ROLE_REQUESTER, rbac.ROLE_RISK_REVIEWER),
        )
        rec = self.make_draft("nw-requester-1")
        eid = rec["exception_id"]
        rec, _ = self.svc.submit_exception(actor("nw-requester-1"), eid, 1)
        rec, _ = self.svc.analyze_exception(actor("nw-risk-1"), eid, rec["version"])
        with self.assertRaises(MakerCheckerViolation):
            self.svc.record_review(
                dual_requester, eid, kind="risk", decision="approve",
                rationale="approving my own paperwork", expected_version=rec["version"],
            )


# --------------------------------------------------------------------------
# State machine
# --------------------------------------------------------------------------


class TestStateMachine(ServiceTestBase):
    def test_cannot_analyze_a_draft(self):
        rec = self.make_draft()
        with self.assertRaises(InvalidState):
            self.svc.analyze_exception(actor("nw-risk-1"), rec["exception_id"], 1)

    def test_cannot_decide_before_reviews(self):
        rec = self.make_draft()
        with self.assertRaises(InvalidState):
            self.svc.decide_exception(
                actor("nw-authority-1"), rec["exception_id"], decision="approve",
                rationale="jumping the queue", expected_version=1,
            )

    def test_cannot_submit_twice(self):
        rec = self.make_draft()
        eid = rec["exception_id"]
        rec, _ = self.svc.submit_exception(actor("nw-requester-1"), eid, 1)
        with self.assertRaises(InvalidState):
            self.svc.submit_exception(actor("nw-requester-1"), eid, rec["version"])

    def test_risk_rejection_is_terminal(self):
        rec = self.make_draft()
        eid = rec["exception_id"]
        rec, _ = self.svc.submit_exception(actor("nw-requester-1"), eid, 1)
        rec, _ = self.svc.analyze_exception(actor("nw-risk-1"), eid, rec["version"])
        rec, _ = self.svc.record_review(
            actor("nw-risk-1"), eid, kind="risk", decision="reject",
            rationale="Compensating controls are inadequate.",
            expected_version=rec["version"],
        )
        self.assertEqual(rec["state"], STATE_REJECTED)
        with self.assertRaises(InvalidState):
            self.svc.record_review(
                actor("nw-compliance-1"), eid, kind="compliance", decision="approve",
                rationale="too late for this", expected_version=rec["version"],
            )

    def test_withdraw_then_no_further_action(self):
        rec = self.make_draft()
        eid = rec["exception_id"]
        rec, _ = self.svc.withdraw_exception(
            actor("nw-requester-1"), eid, 1, "Business no longer needs it."
        )
        self.assertEqual(rec["state"], STATE_WITHDRAWN)
        with self.assertRaises(InvalidState):
            self.svc.submit_exception(actor("nw-requester-1"), eid, rec["version"])

    def test_withdraw_requires_reason(self):
        rec = self.make_draft()
        with self.assertRaises(ValidationError):
            self.svc.withdraw_exception(
                actor("nw-requester-1"), rec["exception_id"], 1, "no"
            )

    def test_full_lifecycle_states_observed(self):
        rec = self.make_draft()
        eid = rec["exception_id"]
        seen = [rec["state"]]
        rec, _ = self.svc.submit_exception(actor("nw-requester-1"), eid, rec["version"])
        seen.append(rec["state"])
        rec, _ = self.svc.analyze_exception(actor("nw-risk-1"), eid, rec["version"])
        seen.append(rec["state"])
        rec, _ = self.svc.record_review(
            actor("nw-risk-1"), eid, kind="risk", decision="approve",
            rationale="risk approves this one", expected_version=rec["version"],
        )
        seen.append(rec["state"])
        rec, _ = self.svc.record_review(
            actor("nw-compliance-1"), eid, kind="compliance", decision="approve",
            rationale="compliance approves too", expected_version=rec["version"],
        )
        seen.append(rec["state"])
        rec, _ = self.svc.decide_exception(
            actor("nw-authority-1"), eid, decision="approve",
            rationale="final authorization granted", expected_version=rec["version"],
        )
        seen.append(rec["state"])
        self.assertEqual(seen, [
            STATE_DRAFT, STATE_SUBMITTED, STATE_RISK_REVIEW,
            STATE_COMPLIANCE_REVIEW, STATE_PENDING_DECISION, STATE_APPROVED,
        ])

    def test_invalid_review_decision_rejected(self):
        rec = self.make_draft()
        eid = rec["exception_id"]
        rec, _ = self.svc.submit_exception(actor("nw-requester-1"), eid, 1)
        rec, _ = self.svc.analyze_exception(actor("nw-risk-1"), eid, rec["version"])
        with self.assertRaises(ValidationError):
            self.svc.record_review(
                actor("nw-risk-1"), eid, kind="risk", decision="maybe",
                rationale="not a real decision", expected_version=rec["version"],
            )


# --------------------------------------------------------------------------
# Expiry
# --------------------------------------------------------------------------


class TestExpiry(ServiceTestBase):
    def test_expired_approval_cannot_authorize(self):
        rec = self.drive_to_pending()
        self.clock.advance(seconds=int(APPROVAL_TTL.total_seconds()) + 60)
        with self.assertRaises(ApprovalExpired) as ctx:
            self.svc.decide_exception(
                actor("nw-authority-1"), rec["exception_id"], decision="approve",
                rationale="reusing stale approvals", expected_version=rec["version"],
            )
        self.assertIn("risk", ctx.exception.details["expired_stages"])

    def test_expired_approvals_do_not_block_rejection(self):
        """A stale file can still be declined; only approval is fenced."""
        rec = self.drive_to_pending()
        self.clock.advance(seconds=int(APPROVAL_TTL.total_seconds()) + 60)
        out, _ = self.svc.decide_exception(
            actor("nw-authority-1"), rec["exception_id"], decision="reject",
            rationale="Stale reviews; declining outright.",
            expected_version=rec["version"],
        )
        self.assertEqual(out["state"], STATE_REJECTED)

    def test_approval_valid_just_before_ttl(self):
        rec = self.drive_to_pending()
        self.clock.advance(seconds=int(APPROVAL_TTL.total_seconds()) - 60)
        out, _ = self.svc.decide_exception(
            actor("nw-authority-1"), rec["exception_id"], decision="approve",
            rationale="Within the approval window.", expected_version=rec["version"],
        )
        self.assertEqual(out["state"], STATE_APPROVED)

    def test_record_expires_when_horizon_passes(self):
        rec = self.make_draft()
        eid = rec["exception_id"]
        self.svc.submit_exception(actor("nw-requester-1"), eid, 1)
        self.clock.advance(days=31)
        out = self.svc.get_exception(actor("nw-requester-1"), eid)
        self.assertEqual(out["state"], STATE_EXPIRED)
        self.assertTrue(out["expired"])

    def test_expired_record_rejects_further_transitions(self):
        rec = self.make_draft()
        eid = rec["exception_id"]
        self.svc.submit_exception(actor("nw-requester-1"), eid, 1)
        self.clock.advance(days=31)
        current = self.svc.get_exception(actor("nw-requester-1"), eid)
        with self.assertRaises(InvalidState):
            self.svc.analyze_exception(actor("nw-risk-1"), eid, current["version"])

    def test_approval_ttl_never_outlives_record_expiry(self):
        """Approval TTL is clamped to the exception's own expiry."""
        rec = self.make_draft(expires_at=(BASE + timedelta(hours=5)).isoformat())
        eid = rec["exception_id"]
        rec, _ = self.svc.submit_exception(actor("nw-requester-1"), eid, 1)
        rec, _ = self.svc.analyze_exception(actor("nw-risk-1"), eid, rec["version"])
        rec, _ = self.svc.record_review(
            actor("nw-risk-1"), eid, kind="risk", decision="approve",
            rationale="approving a short-lived one", expected_version=rec["version"],
        )
        approval = rec["approvals"][0]
        self.assertLessEqual(approval["expires_at"], rec["expires_at"])

    def test_approved_record_stays_approved_after_horizon(self):
        """History is not rewritten; a granted decision remains recorded."""
        rec = self.drive_to_pending()
        eid = rec["exception_id"]
        out, _ = self.svc.decide_exception(
            actor("nw-authority-1"), eid, decision="approve",
            rationale="Authorized before expiry.", expected_version=rec["version"],
        )
        self.assertEqual(out["state"], STATE_APPROVED)
        self.clock.advance(days=60)
        later = self.svc.get_exception(actor("nw-auditor-1"), eid)
        self.assertEqual(later["state"], STATE_APPROVED)
        self.assertTrue(later["expired"])  # flagged, but not re-stated


# --------------------------------------------------------------------------
# Optimistic concurrency
# --------------------------------------------------------------------------


class TestConcurrency(ServiceTestBase):
    def test_stale_version_rejected(self):
        rec = self.make_draft()
        eid = rec["exception_id"]
        self.svc.submit_exception(actor("nw-requester-1"), eid, 1)
        with self.assertRaises(VersionConflict) as ctx:
            self.svc.submit_exception(actor("nw-requester-1"), eid, 1)
        self.assertEqual(ctx.exception.details["current_version"], 2)

    def test_missing_version_rejected(self):
        rec = self.make_draft()
        with self.assertRaises(ValidationError):
            self.svc.submit_exception(actor("nw-requester-1"), rec["exception_id"], None)

    def test_non_integer_version_rejected(self):
        rec = self.make_draft()
        with self.assertRaises(ValidationError):
            self.svc.submit_exception(actor("nw-requester-1"), rec["exception_id"], "1")

    def test_version_increments_on_every_transition(self):
        rec = self.make_draft()
        versions = [rec["version"]]
        eid = rec["exception_id"]
        rec, _ = self.svc.submit_exception(actor("nw-requester-1"), eid, rec["version"])
        versions.append(rec["version"])
        rec, _ = self.svc.analyze_exception(actor("nw-risk-1"), eid, rec["version"])
        versions.append(rec["version"])
        self.assertEqual(versions, [1, 2, 3])

    def test_concurrent_reviews_only_one_wins(self):
        """Two threads race the same risk-review seat; exactly one succeeds."""
        rec = self.make_draft()
        eid = rec["exception_id"]
        rec, _ = self.svc.submit_exception(actor("nw-requester-1"), eid, 1)
        rec, _ = self.svc.analyze_exception(actor("nw-risk-1"), eid, rec["version"])
        version = rec["version"]

        results: list[str] = []
        barrier = threading.Barrier(2)
        lock = threading.Lock()

        def attempt(tag: str):
            barrier.wait()
            try:
                self.svc.record_review(
                    actor("nw-risk-1"), eid, kind="risk", decision="approve",
                    rationale=f"racing reviewer {tag}", expected_version=version,
                )
                outcome = "ok"
            except (VersionConflict, InvalidState) as exc:
                outcome = type(exc).__name__
            with lock:
                results.append(outcome)

        threads = [threading.Thread(target=attempt, args=(t,)) for t in ("a", "b")]
        for t in threads:
            t.start()
        for t in threads:
            t.join(timeout=10)

        self.assertEqual(len(results), 2)
        self.assertEqual(results.count("ok"), 1, f"expected exactly one winner: {results}")

    def test_concurrent_creates_get_distinct_ids(self):
        ids: list[str] = []
        lock = threading.Lock()

        def create():
            rec, _ = self.svc.create_exception(actor("nw-requester-1"), payload())
            with lock:
                ids.append(rec["exception_id"])

        threads = [threading.Thread(target=create) for _ in range(8)]
        for t in threads:
            t.start()
        for t in threads:
            t.join(timeout=10)
        self.assertEqual(len(ids), 8)
        self.assertEqual(len(set(ids)), 8)


# --------------------------------------------------------------------------
# Audit chain integrity
# --------------------------------------------------------------------------


class TestAuditIntegrity(ServiceTestBase):
    def test_chain_valid_after_full_workflow(self):
        rec = self.drive_to_pending()
        self.svc.decide_exception(
            actor("nw-authority-1"), rec["exception_id"], decision="approve",
            rationale="authorizing for audit test", expected_version=rec["version"],
        )
        result = self.svc.verify_audit(actor("nw-auditor-1"))
        self.assertTrue(result["valid"])
        self.assertEqual(result["entry_count"], 6)
        self.assertIsNone(result["broken_at"])

    def test_every_transition_is_audited(self):
        rec = self.drive_to_pending()
        eid = rec["exception_id"]
        self.svc.decide_exception(
            actor("nw-authority-1"), eid, decision="approve",
            rationale="authorizing for audit test", expected_version=rec["version"],
        )
        actions = [e["action"] for e in self.svc.get_audit(actor("nw-auditor-1"), eid)]
        self.assertEqual(actions, [
            "exception.created", "exception.submitted", "exception.analyzed",
            "review.risk.approve", "review.compliance.approve", "decision.approve",
        ])

    def test_genesis_link(self):
        self.make_draft()
        entries = self.svc.store.audit.all_entries()
        self.assertEqual(entries[0].prev_hash, GENESIS_HASH)
        self.assertEqual(entries[0].sequence, 1)

    def test_entries_are_hash_linked(self):
        self.drive_to_pending()
        entries = self.svc.store.audit.all_entries()
        for prev, curr in zip(entries, entries[1:]):
            self.assertEqual(curr.prev_hash, prev.entry_hash)

    def test_payload_tamper_detected(self):
        self.drive_to_pending()
        entries = self.svc.store.audit.all_entries()
        entries[2].payload["rationale"] = "silently rewritten"
        result = self.svc.store.audit.verify()
        self.assertFalse(result["valid"])
        self.assertEqual(result["broken_at"], 3)
        self.assertEqual(result["reason"], "entry_hash_mismatch")

    def test_actor_tamper_detected(self):
        self.drive_to_pending()
        entries = self.svc.store.audit.all_entries()
        entries[1].actor_id = "someone-else"
        result = self.svc.store.audit.verify()
        self.assertFalse(result["valid"])
        self.assertEqual(result["broken_at"], 2)

    def test_deletion_detected(self):
        self.drive_to_pending()
        chain = self.svc.store.audit
        del chain.all_entries()[0]  # no-op on a copy...
        self.assertTrue(chain.verify()["valid"])
        chain._entries.pop(1)  # ...now mutate the real list
        result = chain.verify()
        self.assertFalse(result["valid"])
        self.assertEqual(result["reason"], "sequence_gap")

    def test_reordering_detected(self):
        self.drive_to_pending()
        chain = self.svc.store.audit
        chain._entries[1], chain._entries[2] = chain._entries[2], chain._entries[1]
        self.assertFalse(chain.verify()["valid"])

    def test_head_hash_advances(self):
        before = self.svc.store.audit.head_hash()
        self.make_draft()
        self.assertNotEqual(before, self.svc.store.audit.head_hash())


# --------------------------------------------------------------------------
# Idempotency (store level)
# --------------------------------------------------------------------------


class TestIdempotencyStore(ServiceTestBase):
    def test_replay_returns_prior_record(self):
        store = self.svc.store
        body = {"a": 1}
        self.assertIsNone(store.lookup_command("t", "cmd-1", "op", body))
        store.record_command(
            tenant_id="t", command_id="cmd-1", actor_id="x", operation="op",
            payload=body, response={"ok": True}, status=201, recorded_at=self.clock(),
        )
        found = store.lookup_command("t", "cmd-1", "op", body)
        self.assertIsNotNone(found)
        self.assertEqual(found.response, {"ok": True})

    def test_replay_with_different_payload_rejected(self):
        store = self.svc.store
        store.record_command(
            tenant_id="t", command_id="cmd-1", actor_id="x", operation="op",
            payload={"a": 1}, response={}, status=200, recorded_at=self.clock(),
        )
        with self.assertRaises(IdempotencyMismatch):
            store.lookup_command("t", "cmd-1", "op", {"a": 2})

    def test_command_ids_are_tenant_scoped(self):
        store = self.svc.store
        store.record_command(
            tenant_id="t1", command_id="shared", actor_id="x", operation="op",
            payload={}, response={}, status=200, recorded_at=self.clock(),
        )
        self.assertIsNone(store.lookup_command("t2", "shared", "op", {}))

    def test_key_order_does_not_change_fingerprint(self):
        store = self.svc.store
        store.record_command(
            tenant_id="t", command_id="c", actor_id="x", operation="op",
            payload={"a": 1, "b": 2}, response={}, status=200, recorded_at=self.clock(),
        )
        self.assertIsNotNone(store.lookup_command("t", "c", "op", {"b": 2, "a": 1}))


# --------------------------------------------------------------------------
# HTTP integration
# --------------------------------------------------------------------------


class HttpTestCase(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.httpd = build_server("127.0.0.1", 0)
        cls.port = cls.httpd.server_address[1]
        cls.thread = threading.Thread(
            target=cls.httpd.serve_forever, kwargs={"poll_interval": 0.1}, daemon=True
        )
        cls.thread.start()

    @classmethod
    def tearDownClass(cls):
        cls.httpd.shutdown()
        cls.httpd.server_close()
        cls.thread.join(timeout=5)

    def call(self, method, path, *, actor=None, body=None, command_id=None):
        url = f"http://127.0.0.1:{self.port}{path}"
        data = None if body is None else json.dumps(body).encode("utf-8")
        req = urllib.request.Request(url, data=data, method=method)
        if actor:
            req.add_header("X-Actor", actor)
        if command_id:
            req.add_header("X-Command-Id", command_id)
        if data is not None:
            req.add_header("Content-Type", "application/json")
        try:
            with urllib.request.urlopen(req, timeout=10) as res:
                raw = res.read().decode("utf-8")
                return res.status, (json.loads(raw) if raw else None), dict(res.headers)
        except urllib.error.HTTPError as err:
            raw = err.read().decode("utf-8")
            return err.code, (json.loads(raw) if raw else None), dict(err.headers)


class TestHttpApi(HttpTestCase):
    def test_health_is_open(self):
        status, body, _ = self.call("GET", "/api/health")
        self.assertEqual(status, 200)
        self.assertEqual(body["status"], "ok")

    def test_missing_actor_header_is_401(self):
        status, body, _ = self.call("GET", "/api/exceptions")
        self.assertEqual(status, 401)
        self.assertEqual(body["error"]["code"], "unauthenticated")

    def test_unknown_actor_is_401(self):
        status, body, _ = self.call("GET", "/api/exceptions", actor="mallory")
        self.assertEqual(status, 401)

    def test_unknown_route_is_404(self):
        status, body, _ = self.call("GET", "/api/nope", actor="nw-requester-1")
        self.assertEqual(status, 404)
        self.assertEqual(body["error"]["code"], "not_found")

    def test_delete_is_405(self):
        status, body, _ = self.call("DELETE", "/api/exceptions", actor="nw-requester-1")
        self.assertEqual(status, 405)
        self.assertEqual(body["error"]["code"], "method_not_allowed")

    def test_malformed_json_is_422(self):
        url = f"http://127.0.0.1:{self.port}/api/exceptions"
        req = urllib.request.Request(url, data=b"{not json", method="POST")
        req.add_header("X-Actor", "nw-requester-1")
        req.add_header("Content-Type", "application/json")
        try:
            with urllib.request.urlopen(req, timeout=10):
                self.fail("expected an HTTP error")
        except urllib.error.HTTPError as err:
            self.assertEqual(err.code, 422)

    def test_static_index_served(self):
        url = f"http://127.0.0.1:{self.port}/"
        with urllib.request.urlopen(url, timeout=10) as res:
            self.assertEqual(res.status, 200)
            self.assertIn("text/html", res.headers["Content-Type"])
            self.assertIn("Credit Policy Exception", res.read().decode("utf-8"))

    def test_static_traversal_blocked(self):
        status, _, _ = self.call("GET", "/../creditexc/service.py")
        self.assertIn(status, (403, 404))

    def test_policy_rules_listed(self):
        status, body, _ = self.call("GET", "/api/policy-rules")
        self.assertEqual(status, 200)
        ids = {r["rule_id"] for r in body["rules"]}
        self.assertIn("CP-101", ids)
        self.assertIn("CP-900", ids)

    def test_full_workflow_over_http(self):
        status, created, _ = self.call(
            "POST", "/api/exceptions", actor="nw-requester-1",
            body=payload(expires_at=(datetime.now(timezone.utc) + timedelta(days=20)).isoformat()),
            command_id="http-create-1",
        )
        self.assertEqual(status, 201)
        eid = created["exception_id"]

        status, rec, _ = self.call(
            "POST", f"/api/exceptions/{eid}/submit", actor="nw-requester-1",
            body={"expected_version": created["version"]}, command_id="http-submit-1",
        )
        self.assertEqual(status, 200)
        self.assertEqual(rec["state"], STATE_SUBMITTED)

        status, rec, _ = self.call(
            "POST", f"/api/exceptions/{eid}/analyze", actor="nw-risk-1",
            body={"expected_version": rec["version"]},
        )
        self.assertEqual(status, 200)
        self.assertIn("analysis", rec)

        status, rec, _ = self.call(
            "POST", f"/api/exceptions/{eid}/risk-review", actor="nw-risk-1",
            body={"expected_version": rec["version"], "decision": "approve",
                  "rationale": "Risk approves over HTTP."},
        )
        self.assertEqual(status, 200)

        status, rec, _ = self.call(
            "POST", f"/api/exceptions/{eid}/compliance-review", actor="nw-compliance-1",
            body={"expected_version": rec["version"], "decision": "approve",
                  "rationale": "Compliance approves over HTTP."},
        )
        self.assertEqual(status, 200)
        self.assertEqual(rec["state"], STATE_PENDING_DECISION)

        status, rec, _ = self.call(
            "POST", f"/api/exceptions/{eid}/decide", actor="nw-authority-1",
            body={"expected_version": rec["version"], "decision": "approve",
                  "rationale": "Authority approves over HTTP."},
        )
        self.assertEqual(status, 200)
        self.assertEqual(rec["state"], STATE_APPROVED)

        status, verify, _ = self.call("GET", "/api/audit/verify", actor="nw-auditor-1")
        self.assertEqual(status, 200)
        self.assertTrue(verify["valid"])

    def test_idempotent_replay_over_http(self):
        cmd = "http-idem-create"
        body = payload(
            applicant_pseudonym="APP-IDEM01",
            expires_at=(datetime.now(timezone.utc) + timedelta(days=15)).isoformat(),
        )
        s1, first, _ = self.call("POST", "/api/exceptions", actor="nw-requester-1",
                                 body=body, command_id=cmd)
        s2, second, headers = self.call("POST", "/api/exceptions", actor="nw-requester-1",
                                        body=body, command_id=cmd)
        self.assertEqual(s1, 201)
        self.assertEqual(s2, 201)
        self.assertEqual(first["exception_id"], second["exception_id"])
        self.assertEqual(headers.get("X-Idempotent-Replay"), "true")

    def test_idempotency_mismatch_over_http(self):
        cmd = "http-idem-mismatch"
        self.call("POST", "/api/exceptions", actor="nw-requester-1",
                  body=payload(applicant_pseudonym="APP-MIS001",
                               expires_at=(datetime.now(timezone.utc) + timedelta(days=15)).isoformat()),
                  command_id=cmd)
        status, body, _ = self.call(
            "POST", "/api/exceptions", actor="nw-requester-1",
            body=payload(applicant_pseudonym="APP-MIS002",
                         expires_at=(datetime.now(timezone.utc) + timedelta(days=15)).isoformat()),
            command_id=cmd,
        )
        self.assertEqual(status, 409)
        self.assertEqual(body["error"]["code"], "idempotency_mismatch")

    def test_version_conflict_over_http(self):
        status, created, _ = self.call(
            "POST", "/api/exceptions", actor="nw-requester-1",
            body=payload(applicant_pseudonym="APP-CONF01",
                         expires_at=(datetime.now(timezone.utc) + timedelta(days=15)).isoformat()),
        )
        eid = created["exception_id"]
        self.call("POST", f"/api/exceptions/{eid}/submit", actor="nw-requester-1",
                  body={"expected_version": 1})
        status, body, _ = self.call(
            "POST", f"/api/exceptions/{eid}/submit", actor="nw-requester-1",
            body={"expected_version": 1},
        )
        self.assertEqual(status, 409)
        self.assertEqual(body["error"]["code"], "version_conflict")

    def test_tenant_denial_over_http(self):
        status, created, _ = self.call(
            "POST", "/api/exceptions", actor="nw-requester-1",
            body=payload(applicant_pseudonym="APP-TEN001",
                         expires_at=(datetime.now(timezone.utc) + timedelta(days=15)).isoformat()),
        )
        eid = created["exception_id"]
        status, body, _ = self.call("GET", f"/api/exceptions/{eid}", actor="cs-requester-1")
        self.assertEqual(status, 404)

    def test_role_denial_over_http(self):
        status, body, _ = self.call(
            "POST", "/api/exceptions", actor="nw-auditor-1",
            body=payload(expires_at=(datetime.now(timezone.utc) + timedelta(days=15)).isoformat()),
        )
        self.assertEqual(status, 403)
        self.assertEqual(body["error"]["code"], "forbidden")

    def test_prohibited_rule_over_http(self):
        status, body, _ = self.call(
            "POST", "/api/exceptions", actor="nw-requester-1",
            body=payload(rule_id="CP-900",
                         expires_at=(datetime.now(timezone.utc) + timedelta(days=15)).isoformat()),
        )
        self.assertEqual(status, 422)
        self.assertEqual(body["error"]["code"], "policy_violation")

    def test_bad_command_id_rejected(self):
        status, body, _ = self.call(
            "POST", "/api/exceptions", actor="nw-requester-1",
            body=payload(), command_id="bad id with spaces",
        )
        self.assertEqual(status, 422)

    def test_invalid_state_filter_rejected(self):
        status, body, _ = self.call(
            "GET", "/api/exceptions?state=nonsense", actor="nw-requester-1"
        )
        self.assertEqual(status, 422)


if __name__ == "__main__":
    unittest.main(verbosity=2)
