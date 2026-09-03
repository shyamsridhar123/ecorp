"""Tenants, actors, roles, and the capability matrix.

Fixtures are deliberately data-minimized: applicants are referenced by an
opaque pseudonymous key only. No names, addresses, or government
identifiers exist anywhere in this scenario.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Iterable

from .errors import Forbidden, Unauthenticated

# --- Roles -----------------------------------------------------------------

ROLE_REQUESTER = "requester"
ROLE_RISK_REVIEWER = "risk_reviewer"
ROLE_COMPLIANCE_REVIEWER = "compliance_reviewer"
ROLE_CREDIT_AUTHORITY = "credit_authority"
ROLE_AUDITOR = "auditor"

ALL_ROLES = (
    ROLE_REQUESTER,
    ROLE_RISK_REVIEWER,
    ROLE_COMPLIANCE_REVIEWER,
    ROLE_CREDIT_AUTHORITY,
    ROLE_AUDITOR,
)

# --- Capabilities ----------------------------------------------------------

CAP_CREATE = "exception:create"
CAP_SUBMIT = "exception:submit"
CAP_WITHDRAW = "exception:withdraw"
CAP_READ = "exception:read"
CAP_REVIEW_RISK = "exception:review_risk"
CAP_REVIEW_COMPLIANCE = "exception:review_compliance"
CAP_DECIDE = "exception:decide"
CAP_AUDIT_READ = "audit:read"
CAP_AUDIT_VERIFY = "audit:verify"

ROLE_CAPABILITIES: dict[str, frozenset[str]] = {
    ROLE_REQUESTER: frozenset({CAP_CREATE, CAP_SUBMIT, CAP_WITHDRAW, CAP_READ}),
    ROLE_RISK_REVIEWER: frozenset({CAP_READ, CAP_REVIEW_RISK}),
    ROLE_COMPLIANCE_REVIEWER: frozenset({CAP_READ, CAP_REVIEW_COMPLIANCE}),
    ROLE_CREDIT_AUTHORITY: frozenset({CAP_READ, CAP_DECIDE}),
    ROLE_AUDITOR: frozenset({CAP_READ, CAP_AUDIT_READ, CAP_AUDIT_VERIFY}),
}


@dataclass(frozen=True)
class Actor:
    actor_id: str
    tenant_id: str
    display_name: str
    roles: tuple[str, ...]

    @property
    def capabilities(self) -> frozenset[str]:
        caps: set[str] = set()
        for role in self.roles:
            caps |= ROLE_CAPABILITIES.get(role, frozenset())
        return frozenset(caps)

    def has(self, capability: str) -> bool:
        return capability in self.capabilities

    def public(self) -> dict:
        return {
            "actor_id": self.actor_id,
            "tenant_id": self.tenant_id,
            "display_name": self.display_name,
            "roles": list(self.roles),
            "capabilities": sorted(self.capabilities),
        }


@dataclass(frozen=True)
class Tenant:
    tenant_id: str
    name: str
    # Maximum deviation (basis points of policy threshold) this tenant may
    # ever grant, regardless of approvals. Deterministic guardrail.
    max_deviation_bps: int


TENANTS: dict[str, Tenant] = {
    "tenant-northwind": Tenant("tenant-northwind", "Northwind Bank", 2500),
    "tenant-cascadia": Tenant("tenant-cascadia", "Cascadia Credit Union", 1500),
}


def _actor(actor_id: str, tenant: str, name: str, *roles: str) -> Actor:
    return Actor(actor_id, tenant, name, tuple(roles))


# Fixture directory. Token == actor_id keeps the demo honest about the fact
# that this is a scenario harness and not a real credential system.
ACTORS: dict[str, Actor] = {
    a.actor_id: a
    for a in (
        _actor("nw-requester-1", "tenant-northwind", "N. Requester One", ROLE_REQUESTER),
        _actor("nw-requester-2", "tenant-northwind", "N. Requester Two", ROLE_REQUESTER),
        _actor("nw-risk-1", "tenant-northwind", "N. Risk Reviewer", ROLE_RISK_REVIEWER),
        _actor("nw-compliance-1", "tenant-northwind", "N. Compliance Reviewer", ROLE_COMPLIANCE_REVIEWER),
        _actor("nw-authority-1", "tenant-northwind", "N. Credit Authority", ROLE_CREDIT_AUTHORITY),
        _actor("nw-auditor-1", "tenant-northwind", "N. Auditor", ROLE_AUDITOR),
        # Deliberate maker-checker trap: this actor can both request and
        # decide. The domain must still refuse self-approval.
        _actor(
            "nw-dual-1",
            "tenant-northwind",
            "N. Dual Hat",
            ROLE_REQUESTER,
            ROLE_CREDIT_AUTHORITY,
        ),
        _actor("cs-requester-1", "tenant-cascadia", "C. Requester One", ROLE_REQUESTER),
        _actor("cs-risk-1", "tenant-cascadia", "C. Risk Reviewer", ROLE_RISK_REVIEWER),
        _actor("cs-compliance-1", "tenant-cascadia", "C. Compliance Reviewer", ROLE_COMPLIANCE_REVIEWER),
        _actor("cs-authority-1", "tenant-cascadia", "C. Credit Authority", ROLE_CREDIT_AUTHORITY),
    )
}


def authenticate(token: str | None) -> Actor:
    if not token:
        raise Unauthenticated("Missing actor token; supply an X-Actor header.")
    actor = ACTORS.get(token.strip())
    if actor is None:
        raise Unauthenticated("Unknown actor token.")
    return actor


def require(actor: Actor, capability: str) -> None:
    if not actor.has(capability):
        raise Forbidden(
            f"Actor {actor.actor_id} lacks capability {capability}.",
            details={"required": capability, "held": sorted(actor.capabilities)},
        )


def require_any(actor: Actor, capabilities: Iterable[str]) -> None:
    wanted = list(capabilities)
    if not any(actor.has(c) for c in wanted):
        raise Forbidden(
            f"Actor {actor.actor_id} lacks all of {wanted}.",
            details={"required_any": wanted, "held": sorted(actor.capabilities)},
        )
