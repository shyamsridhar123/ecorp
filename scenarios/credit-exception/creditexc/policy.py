"""Deterministic credit-policy catalog and exception validation.

The rules below are intentionally *deterministic*: given the same request
payload and the same ``now``, validation always produces the same verdict.
No randomness, no clock-dependent heuristics beyond explicit expiry math.
"""

from __future__ import annotations

import re
from dataclasses import dataclass
from datetime import datetime, timedelta, timezone

from .errors import PolicyViolation, ValidationError

MAX_EXPIRY_DAYS = 180
MIN_JUSTIFICATION_CHARS = 40
MIN_CONTROL_CHARS = 15
MIN_CONTROLS = 1
MAX_CONTROLS = 6


@dataclass(frozen=True)
class PolicyRule:
    rule_id: str
    title: str
    description: str
    # Exceptions to this rule can never be granted at all.
    prohibited: bool
    # Deviation ceiling in basis points for this specific rule.
    max_deviation_bps: int
    # Whether a compliance review is mandatory on top of risk review.
    requires_compliance: bool


POLICY_RULES: dict[str, PolicyRule] = {
    r.rule_id: r
    for r in (
        PolicyRule(
            "CP-101",
            "Maximum debt-to-income ratio",
            "Applicant DTI must not exceed the product threshold.",
            prohibited=False,
            max_deviation_bps=2000,
            requires_compliance=True,
        ),
        PolicyRule(
            "CP-102",
            "Minimum credit score band",
            "Applicant score band must meet the product floor.",
            prohibited=False,
            max_deviation_bps=1500,
            requires_compliance=True,
        ),
        PolicyRule(
            "CP-201",
            "Collateral coverage ratio",
            "Secured facilities must maintain minimum collateral coverage.",
            prohibited=False,
            max_deviation_bps=1000,
            requires_compliance=False,
        ),
        PolicyRule(
            "CP-202",
            "Concentration limit per sector",
            "Portfolio exposure to a single sector is capped.",
            prohibited=False,
            max_deviation_bps=800,
            requires_compliance=True,
        ),
        PolicyRule(
            "CP-900",
            "Sanctions and prohibited-party screening",
            "Screening failures are absolute; no exception may be granted.",
            prohibited=True,
            max_deviation_bps=0,
            requires_compliance=True,
        ),
        PolicyRule(
            "CP-901",
            "Statutory affordability assessment",
            "Regulator-mandated affordability checks cannot be waived.",
            prohibited=True,
            max_deviation_bps=0,
            requires_compliance=True,
        ),
    )
}

PSEUDONYM_RE = re.compile(r"^APP-[A-Z0-9]{6,12}$")


def parse_iso8601(value: str, field: str) -> datetime:
    text = value.strip()
    if text.endswith("Z"):
        text = text[:-1] + "+00:00"
    try:
        parsed = datetime.fromisoformat(text)
    except (ValueError, TypeError) as exc:
        raise ValidationError(
            f"Field '{field}' must be an ISO-8601 timestamp.", details={"field": field}
        ) from exc
    if parsed.tzinfo is None:
        parsed = parsed.replace(tzinfo=timezone.utc)
    return parsed.astimezone(timezone.utc)


def _require_str(payload: dict, field: str, *, min_len: int = 1, max_len: int = 4000) -> str:
    raw = payload.get(field)
    if not isinstance(raw, str):
        raise ValidationError(
            f"Field '{field}' is required and must be a string.",
            details={"field": field},
        )
    value = raw.strip()
    if len(value) < min_len:
        raise ValidationError(
            f"Field '{field}' must be at least {min_len} characters.",
            details={"field": field, "min_length": min_len, "actual_length": len(value)},
        )
    if len(value) > max_len:
        raise ValidationError(
            f"Field '{field}' must be at most {max_len} characters.",
            details={"field": field, "max_length": max_len, "actual_length": len(value)},
        )
    return value


@dataclass(frozen=True)
class ValidatedRequest:
    applicant_pseudonym: str
    rule_id: str
    requested_deviation_bps: int
    justification: str
    compensating_controls: tuple[str, ...]
    expires_at: datetime
    requires_compliance: bool


def validate_exception_payload(
    payload: dict, *, tenant_max_bps: int, now: datetime
) -> ValidatedRequest:
    """Validate a create/update payload against schema and credit policy.

    Raises :class:`ValidationError` for shape problems and
    :class:`PolicyViolation` for well-formed but forbidden requests.
    """
    if not isinstance(payload, dict):
        raise ValidationError("Request body must be a JSON object.")

    pseudonym = _require_str(payload, "applicant_pseudonym", min_len=10, max_len=16)
    if not PSEUDONYM_RE.match(pseudonym):
        raise ValidationError(
            "Field 'applicant_pseudonym' must match APP-XXXXXX (pseudonymous key only).",
            details={"field": "applicant_pseudonym", "pattern": PSEUDONYM_RE.pattern},
        )

    rule_id = _require_str(payload, "rule_id", min_len=3, max_len=16).upper()
    rule = POLICY_RULES.get(rule_id)
    if rule is None:
        raise ValidationError(
            f"Unknown policy rule '{rule_id}'.",
            details={"field": "rule_id", "known": sorted(POLICY_RULES)},
        )
    if rule.prohibited:
        raise PolicyViolation(
            f"Policy rule {rule_id} ({rule.title}) admits no exceptions.",
            details={"rule_id": rule_id, "reason": "prohibited_rule"},
        )

    raw_bps = payload.get("requested_deviation_bps")
    if isinstance(raw_bps, bool) or not isinstance(raw_bps, int):
        raise ValidationError(
            "Field 'requested_deviation_bps' must be an integer.",
            details={"field": "requested_deviation_bps"},
        )
    if raw_bps <= 0:
        raise ValidationError(
            "Field 'requested_deviation_bps' must be greater than zero.",
            details={"field": "requested_deviation_bps", "actual": raw_bps},
        )
    if raw_bps > rule.max_deviation_bps:
        raise PolicyViolation(
            f"Requested deviation {raw_bps} bps exceeds the {rule_id} ceiling "
            f"of {rule.max_deviation_bps} bps.",
            details={
                "rule_id": rule_id,
                "requested_bps": raw_bps,
                "rule_max_bps": rule.max_deviation_bps,
                "reason": "rule_ceiling_exceeded",
            },
        )
    if raw_bps > tenant_max_bps:
        raise PolicyViolation(
            f"Requested deviation {raw_bps} bps exceeds the tenant ceiling "
            f"of {tenant_max_bps} bps.",
            details={
                "requested_bps": raw_bps,
                "tenant_max_bps": tenant_max_bps,
                "reason": "tenant_ceiling_exceeded",
            },
        )

    justification = _require_str(
        payload, "justification", min_len=MIN_JUSTIFICATION_CHARS, max_len=4000
    )

    raw_controls = payload.get("compensating_controls")
    if not isinstance(raw_controls, list):
        raise ValidationError(
            "Field 'compensating_controls' must be a list of strings.",
            details={"field": "compensating_controls"},
        )
    controls: list[str] = []
    for index, item in enumerate(raw_controls):
        if not isinstance(item, str):
            raise ValidationError(
                f"compensating_controls[{index}] must be a string.",
                details={"field": "compensating_controls", "index": index},
            )
        text = item.strip()
        if len(text) < MIN_CONTROL_CHARS:
            raise ValidationError(
                f"compensating_controls[{index}] must be at least "
                f"{MIN_CONTROL_CHARS} characters.",
                details={
                    "field": "compensating_controls",
                    "index": index,
                    "min_length": MIN_CONTROL_CHARS,
                },
            )
        controls.append(text)
    if len(controls) < MIN_CONTROLS:
        raise ValidationError(
            f"At least {MIN_CONTROLS} compensating control is required.",
            details={"field": "compensating_controls", "min_items": MIN_CONTROLS},
        )
    if len(controls) > MAX_CONTROLS:
        raise ValidationError(
            f"At most {MAX_CONTROLS} compensating controls are allowed.",
            details={"field": "compensating_controls", "max_items": MAX_CONTROLS},
        )
    if len({c.lower() for c in controls}) != len(controls):
        raise ValidationError(
            "compensating_controls must not contain duplicates.",
            details={"field": "compensating_controls", "reason": "duplicate_control"},
        )

    expires_raw = _require_str(payload, "expires_at", min_len=4, max_len=64)
    expires_at = parse_iso8601(expires_raw, "expires_at")
    if expires_at <= now:
        raise ValidationError(
            "Field 'expires_at' must be in the future.",
            details={"field": "expires_at", "now": now.isoformat()},
        )
    if expires_at > now + timedelta(days=MAX_EXPIRY_DAYS):
        raise PolicyViolation(
            f"Exception expiry may not exceed {MAX_EXPIRY_DAYS} days from now.",
            details={
                "field": "expires_at",
                "max_days": MAX_EXPIRY_DAYS,
                "reason": "expiry_horizon_exceeded",
            },
        )

    return ValidatedRequest(
        applicant_pseudonym=pseudonym,
        rule_id=rule_id,
        requested_deviation_bps=raw_bps,
        justification=justification,
        compensating_controls=tuple(controls),
        expires_at=expires_at,
        requires_compliance=rule.requires_compliance,
    )


def catalog() -> list[dict]:
    """Public, non-prohibited-inclusive policy catalog for the UI."""
    return [
        {
            "rule_id": r.rule_id,
            "title": r.title,
            "description": r.description,
            "prohibited": r.prohibited,
            "max_deviation_bps": r.max_deviation_bps,
            "requires_compliance": r.requires_compliance,
        }
        for r in sorted(POLICY_RULES.values(), key=lambda x: x.rule_id)
    ]
