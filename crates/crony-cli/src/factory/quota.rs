//! Pure, shared admission state for the trusted factory's GitHub requests.
//!
//! The caller owns execution, sleeping and persistence. A successful response is
//! always usable when its quota observation is valid, even if the *next* request
//! must wait. Local backoff is capped; an upstream deadline is never capped down.

use std::{
    error::Error,
    fmt,
    sync::{Arc, Mutex, MutexGuard},
};

use anyhow::{Result, anyhow, bail};
use chrono::{DateTime, TimeDelta, Utc};
use crony_domain::{FactoryPollingState, FactoryRetryReason, GithubGraphqlQuota};
use serde_json::Value;

const RESET_MARGIN_SECONDS: i64 = 5;
const BASE_BACKOFF_SECONDS: i64 = 60;
const MAX_BACKOFF_SECONDS: i64 = 15 * 60;
const MAX_FAILURES: u32 = 16;
const MAX_QUOTA_POINTS: u64 = 1_000_000;
const MAX_QUOTA_WINDOW_SECONDS: i64 = 24 * 60 * 60;
const MAX_HEADER_BYTES: usize = 64 * 1024;
const MAX_HEADER_LINES: usize = 256;
const MAX_HEADER_BLOCKS: usize = 8;
const MAX_DIAGNOSTIC_BYTES: usize = 64 * 1024;
const MAX_ERROR_BODY_BYTES: usize = 1024 * 1024;

/// Contains only safe, typed metadata, never command arguments or remote text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct GitHubWait {
    pub(crate) reason: FactoryRetryReason,
    pub(crate) next_retry_at: DateTime<Utc>,
}

impl fmt::Display for GitHubWait {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let reason = match self.reason {
            FactoryRetryReason::GraphqlQuota => "GraphQL quota reserve",
            FactoryRetryReason::PrimaryRateLimit => "primary rate limit",
            FactoryRetryReason::SecondaryRateLimit => "secondary rate limit",
            FactoryRetryReason::GithubUnavailable => "upstream unavailable",
        };
        if self.next_retry_at == DateTime::<Utc>::MAX_UTC {
            write!(
                f,
                "GitHub {reason}; retry deadline requires operator recovery"
            )
        } else {
            write!(
                f,
                "GitHub {reason}; retry not before {}",
                self.next_retry_at
            )
        }
    }
}

impl Error for GitHubWait {}

#[derive(Debug, Clone, Default)]
pub(crate) struct BudgetState {
    state: Arc<Mutex<FactoryPollingState>>,
}

impl BudgetState {
    fn lock(&self) -> MutexGuard<'_, FactoryPollingState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(crate) fn snapshot(&self) -> FactoryPollingState {
        self.lock().clone()
    }

    /// Merge, rather than replace: a stale reconcile cannot cancel a live wait.
    /// Invalid persisted quota/partial failure state gets a conservative backoff.
    pub(crate) fn restore(&self, persisted: FactoryPollingState, now: DateTime<Utc>) {
        let mut state = self.lock();
        state.consecutive_failures = state
            .consecutive_failures
            .max(persisted.consecutive_failures)
            .min(MAX_FAILURES);
        if let Some(deadline) = persisted.next_retry_at {
            extend_wait(
                &mut state,
                persisted
                    .retry_reason
                    .unwrap_or(FactoryRetryReason::GithubUnavailable),
                deadline,
            );
        } else if persisted.retry_reason.is_some() || persisted.consecutive_failures > 0 {
            repair_wait(&mut state, now);
        }
        if let Some(quota) = persisted.graphql {
            if valid_quota(&quota) && quota.observed_at <= now {
                merge_quota(&mut state, quota);
            } else {
                repair_wait(&mut state, now);
            }
        }
        retain_quota_wait(&mut state, now);
    }

    pub(crate) fn check(&self, now: DateTime<Utc>) -> Result<()> {
        let mut state = self.lock();
        retain_quota_wait(&mut state, now);
        if let Some(deadline) = state.next_retry_at
            && outstanding(deadline, now)
        {
            return Err(GitHubWait {
                reason: state
                    .retry_reason
                    .unwrap_or(FactoryRetryReason::GithubUnavailable),
                next_retry_at: deadline,
            }
            .into());
        }
        Ok(())
    }

    /// Missing quota is not an observation (including REST `/rate_limit` data).
    /// Bad observations are rejected without mutating known state. Stale ones
    /// are ignored without discarding the otherwise successful response.
    pub(crate) fn observe_json(&self, response: &Value, now: DateTime<Utc>) -> Result<()> {
        let Some(value) = response.pointer("/data/rateLimit") else {
            return Ok(());
        };
        let invalid = || anyhow!("invalid GitHub GraphQL quota observation");
        let number = |key: &str| value.get(key).and_then(Value::as_u64).ok_or_else(invalid);
        let reset = value
            .get("resetAt")
            .and_then(Value::as_str)
            .filter(|text| text.len() <= 64)
            .ok_or_else(invalid)?;
        let quota = GithubGraphqlQuota {
            limit: number("limit")?,
            remaining: number("remaining")?,
            cost: number("cost")?,
            reset_at: DateTime::parse_from_rfc3339(reset)
                .map_err(|_| invalid())?
                .with_timezone(&Utc),
            observed_at: now,
        };
        if !valid_counters(&quota) {
            return Err(invalid());
        }
        if !outstanding(after(quota.reset_at, RESET_MARGIN_SECONDS), now) {
            return Ok(());
        }
        if !valid_quota(&quota) {
            return Err(invalid());
        }
        let mut state = self.lock();
        merge_quota(&mut state, quota);
        retain_quota_wait(&mut state, now);
        Ok(())
    }

    /// Only typed failures are retryable; arbitrary error text is never parsed.
    /// A GraphqlQuota admission wait is not a failed attempt.
    pub(crate) fn failure(&self, error: &anyhow::Error, now: DateTime<Utc>) -> bool {
        let Some(wait) = error.downcast_ref::<GitHubWait>() else {
            return false;
        };
        let mut state = self.lock();
        let mut deadline = wait.next_retry_at;
        if wait.reason != FactoryRetryReason::GraphqlQuota {
            state.consecutive_failures = state
                .consecutive_failures
                .saturating_add(1)
                .min(MAX_FAILURES);
            deadline = deadline.max(after(now, backoff(state.consecutive_failures)));
        }
        if wait.reason == FactoryRetryReason::PrimaryRateLimit
            && let Some(quota) = &state.graphql
        {
            // A reported primary exhaustion supersedes a previously high balance.
            deadline = deadline.max(after(quota.reset_at, RESET_MARGIN_SECONDS));
        }
        extend_wait(&mut state, wait.reason, deadline);
        retain_quota_wait(&mut state, now);
        true
    }

    pub(crate) fn success(&self, now: DateTime<Utc>) {
        let mut state = self.lock();
        if !state
            .next_retry_at
            .is_some_and(|deadline| outstanding(deadline, now))
        {
            state.next_retry_at = None;
            state.retry_reason = None;
            state.consecutive_failures = 0;
        } else if state.retry_reason == Some(FactoryRetryReason::GraphqlQuota) {
            state.consecutive_failures = 0;
        }
        retain_quota_wait(&mut state, now);
    }
}

fn backoff(failures: u32) -> i64 {
    (BASE_BACKOFF_SECONDS * (1_i64 << failures.saturating_sub(1).min(4))).min(MAX_BACKOFF_SECONDS)
}

// MAX_UTC is a fail-closed sentinel for an unrepresentable positive deadline.
// In particular, overflowing Retry-After must not become an immediate retry.
fn after(now: DateTime<Utc>, seconds: i64) -> DateTime<Utc> {
    TimeDelta::try_seconds(seconds)
        .and_then(|delta| now.checked_add_signed(delta))
        .unwrap_or(DateTime::<Utc>::MAX_UTC)
}

fn outstanding(deadline: DateTime<Utc>, now: DateTime<Utc>) -> bool {
    deadline == DateTime::<Utc>::MAX_UTC || deadline > now
}

fn extend_wait(
    state: &mut FactoryPollingState,
    reason: FactoryRetryReason,
    deadline: DateTime<Utc>,
) {
    if state.next_retry_at.is_none_or(|current| deadline > current)
        || (state.next_retry_at == Some(deadline)
            && state.retry_reason == Some(FactoryRetryReason::GraphqlQuota)
            && reason != FactoryRetryReason::GraphqlQuota)
    {
        state.next_retry_at = Some(deadline);
        state.retry_reason = Some(reason);
    }
}

fn repair_wait(state: &mut FactoryPollingState, now: DateTime<Utc>) {
    state.consecutive_failures = state.consecutive_failures.clamp(1, MAX_FAILURES);
    extend_wait(
        state,
        FactoryRetryReason::GithubUnavailable,
        after(now, backoff(state.consecutive_failures)),
    );
}

fn valid_counters(quota: &GithubGraphqlQuota) -> bool {
    quota.limit > 0
        && quota.limit <= MAX_QUOTA_POINTS
        && quota.remaining <= quota.limit
        && quota.cost <= quota.limit
}

fn valid_quota(quota: &GithubGraphqlQuota) -> bool {
    let window = quota.reset_at.signed_duration_since(quota.observed_at);
    valid_counters(quota)
        && window >= TimeDelta::seconds(-RESET_MARGIN_SECONDS)
        && window <= TimeDelta::seconds(MAX_QUOTA_WINDOW_SECONDS)
}

fn merge_quota(state: &mut FactoryPollingState, quota: GithubGraphqlQuota) {
    if let Some(current) = &state.graphql
        && (quota.observed_at < current.observed_at
            || quota.reset_at < current.reset_at
            || (quota.reset_at == current.reset_at && quota.remaining > current.remaining))
    {
        return;
    }
    state.graphql = Some(quota);
}

fn retain_quota_wait(state: &mut FactoryPollingState, now: DateTime<Utc>) {
    if let Some(quota) = &state.graphql {
        // One percent, at least one point and never more than 25. Keep room
        // for another request at its actual last observed cost as well.
        let reserve = (quota.limit / 100).clamp(1, 25);
        let required = quota.cost.max(1).saturating_add(reserve);
        let deadline = after(quota.reset_at, RESET_MARGIN_SECONDS);
        if quota.remaining < required && outstanding(deadline, now) {
            extend_wait(state, FactoryRetryReason::GraphqlQuota, deadline);
        }
    }
}

/// Strip only consecutive HTTP preambles, not arbitrary prefixes or JSON text.
/// JSON decoding belongs to the caller; plain fixtures are returned unchanged.
pub(crate) fn json_body(stdout: &[u8]) -> Result<&[u8]> {
    split_headers(stdout).map(|response| response.body)
}

struct Response<'a> {
    body: &'a [u8],
    status: Option<u16>,
    headers: Vec<(&'a str, &'a str)>,
}

fn split_headers(stdout: &[u8]) -> Result<Response<'_>> {
    let mut response = Response {
        body: stdout,
        status: None,
        headers: Vec::new(),
    };
    let mut rest = stdout.trim_ascii_start();
    let mut bytes = 0;
    let mut lines = 0;
    let mut blocks = 0;
    while rest.starts_with(b"HTTP/") {
        blocks += 1;
        if blocks > MAX_HEADER_BLOCKS {
            bail!("GitHub response HTTP headers exceed bounds");
        }
        let (status, tail) = header_line(rest, &mut bytes, &mut lines)?;
        rest = tail;
        let mut words = status.split_ascii_whitespace();
        if !matches!(
            words.next(),
            Some("HTTP/1.0" | "HTTP/1.1" | "HTTP/2" | "HTTP/2.0" | "HTTP/3" | "HTTP/3.0")
        ) {
            bail!("invalid GitHub response HTTP status");
        }
        response.status = Some(
            words
                .next()
                .filter(|code| code.len() == 3 && code.bytes().all(|b| b.is_ascii_digit()))
                .and_then(|code| code.parse::<u16>().ok())
                .filter(|code| (100..600).contains(code))
                .ok_or_else(|| anyhow!("invalid GitHub response HTTP status"))?,
        );
        loop {
            let (line, tail) = header_line(rest, &mut bytes, &mut lines)?;
            rest = tail;
            if line.is_empty() {
                break;
            }
            let (name, value) = line
                .split_once(':')
                .filter(|(name, _)| !name.is_empty())
                .ok_or_else(|| anyhow!("invalid GitHub response HTTP header"))?;
            // Retain only relevant borrowed metadata; never echo any header.
            if ["retry-after", "x-ratelimit-reset", "x-ratelimit-remaining"]
                .iter()
                .any(|key| name.eq_ignore_ascii_case(key))
            {
                response.headers.push((name, value.trim()));
            }
        }
        rest = rest.trim_ascii_start();
        response.body = rest;
    }
    Ok(response)
}

fn header_line<'a>(
    bytes: &'a [u8],
    total: &mut usize,
    lines: &mut usize,
) -> Result<(&'a str, &'a [u8])> {
    let available = MAX_HEADER_BYTES.saturating_sub(*total);
    let end = bytes
        .iter()
        .take(available)
        .position(|byte| *byte == b'\n')
        .filter(|_| *lines < MAX_HEADER_LINES)
        .ok_or_else(|| anyhow!("GitHub response HTTP headers are incomplete or exceed bounds"))?;
    *total += end + 1;
    *lines += 1;
    let line = bytes[..end].strip_suffix(b"\r").unwrap_or(&bytes[..end]);
    let text = std::str::from_utf8(line)
        .map_err(|_| anyhow!("invalid GitHub response HTTP header encoding"))?;
    Ok((text, &bytes[end + 1..]))
}

#[derive(Default)]
struct Signals {
    primary: bool,
    secondary: bool,
    unavailable: bool,
    too_many: bool,
    forbidden: bool,
}

impl Signals {
    fn text(&mut self, bytes: &[u8]) {
        let text = String::from_utf8_lossy(&bytes[..bytes.len().min(MAX_DIAGNOSTIC_BYTES)])
            .to_ascii_lowercase();
        self.secondary |= text.contains("secondary rate limit")
            || text.contains("secondary rate-limit")
            || text.contains("abuse detection")
            || text.contains("abuse rate limit");
        self.primary |= text.contains("primary rate limit")
            || text.contains("rate limit exceeded")
            || text.contains("rate limit was exceeded")
            || text.contains("rate limit has been exceeded");
        self.unavailable |= text.contains("service unavailable")
            || text.contains("temporarily unavailable")
            || text.contains("bad gateway")
            || text.contains("gateway timeout")
            || has_http_status(&text, "503");
        self.too_many |= has_http_status(&text, "429");
        self.forbidden |= has_http_status(&text, "403");
    }
}

fn has_http_status(text: &str, code: &str) -> bool {
    [
        "http ",
        "http",
        "http/1.0 ",
        "http/1.1 ",
        "http/2 ",
        "http/2.0 ",
    ]
    .iter()
    .any(|prefix| {
        text.match_indices(*prefix).any(|(index, _)| {
            let tail = &text[index + prefix.len()..];
            tail.starts_with(code)
                && !tail
                    .as_bytes()
                    .get(code.len())
                    .is_some_and(u8::is_ascii_digit)
        })
    })
}

/// Classify only error envelopes, diagnostics, and actual HTTP status/headers,
/// not arbitrary strings inside successful GraphQL data. RATE_LIMITED is a
/// primary failure; GraphqlQuota is reserved for proactive admission waits.
/// Truncated headers/diagnostics and overflowing positive retry hints fail
/// closed at MAX_UTC rather than silently substituting a shorter deadline.
pub(crate) fn classify_failure(
    stdout: &[u8],
    stderr: &[u8],
    now: DateTime<Utc>,
) -> Option<GitHubWait> {
    if stderr.len() > MAX_DIAGNOSTIC_BYTES
        || stderr
            .split(|byte| *byte == b'\n')
            .take(MAX_HEADER_LINES + 1)
            .count()
            > MAX_HEADER_LINES
    {
        // Do not truncate away a possibly longer Retry-After requirement.
        return Some(GitHubWait {
            reason: FactoryRetryReason::GithubUnavailable,
            next_retry_at: DateTime::<Utc>::MAX_UTC,
        });
    }
    let response = match split_headers(stdout) {
        Ok(response) => response,
        Err(_) => {
            return Some(GitHubWait {
                reason: FactoryRetryReason::GithubUnavailable,
                // An incomplete header block may hide a valid, longer deadline.
                next_retry_at: DateTime::<Utc>::MAX_UTC,
            });
        }
    };
    let mut signals = Signals::default();
    signals.text(stderr);
    if response.body.len() <= MAX_ERROR_BODY_BYTES {
        match serde_json::from_slice::<Value>(response.body) {
            Ok(value) => {
                if let Some(errors) = value.get("errors").and_then(Value::as_array) {
                    for error in errors.iter().take(32) {
                        signals.primary |= ["/type", "/extensions/code", "/extensions/type"]
                            .iter()
                            .any(|key| {
                                error.pointer(key).and_then(Value::as_str) == Some("RATE_LIMITED")
                            });
                        if let Some(message) = error.get("message").and_then(Value::as_str) {
                            signals.text(message.as_bytes());
                        }
                    }
                }
                if value.get("data").is_none()
                    && let Some(message) = value.get("message").and_then(Value::as_str)
                {
                    signals.text(message.as_bytes());
                }
            }
            Err(_) => signals.text(response.body),
        }
    }
    let stderr_text = String::from_utf8_lossy(stderr);
    let headers: Vec<_> = response
        .headers
        .iter()
        .copied()
        .chain(
            stderr_text
                .lines()
                .take(MAX_HEADER_LINES)
                .filter_map(|line| {
                    line.split_once(':')
                        .map(|(key, value)| (key.trim(), value.trim()))
                }),
        )
        .collect();
    let exhausted = headers.iter().any(|(key, value)| {
        key.eq_ignore_ascii_case("x-ratelimit-remaining") && value.parse::<u64>() == Ok(0)
    });
    let reason = if signals.secondary {
        FactoryRetryReason::SecondaryRateLimit
    } else if signals.primary
        || (exhausted
            && (matches!(response.status, Some(403 | 429))
                || signals.forbidden
                || signals.too_many))
    {
        FactoryRetryReason::PrimaryRateLimit
    } else if signals.too_many || response.status == Some(429) {
        FactoryRetryReason::SecondaryRateLimit
    } else if signals.unavailable || response.status == Some(503) {
        FactoryRetryReason::GithubUnavailable
    } else {
        return None;
    };
    // Primary window headers also accompany secondary limits and outages.
    // A healthy primary budget does not make its reset a retry requirement.
    let use_primary_reset = exhausted
        || matches!(
            reason,
            FactoryRetryReason::PrimaryRateLimit | FactoryRetryReason::GraphqlQuota
        );
    let next_retry_at = headers
        .iter()
        .filter(|(key, _)| use_primary_reset || !key.eq_ignore_ascii_case("x-ratelimit-reset"))
        .filter_map(|(key, value)| retry_hint(key, value, now))
        .filter(|deadline| outstanding(*deadline, now))
        .max()
        .unwrap_or_else(|| after(now, BASE_BACKOFF_SECONDS));
    Some(GitHubWait {
        reason,
        next_retry_at,
    })
}

fn retry_hint(name: &str, value: &str, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let reset = name.eq_ignore_ascii_case("x-ratelimit-reset");
    if !reset && !name.eq_ignore_ascii_case("retry-after") {
        return None;
    }
    if !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()) {
        let Ok(seconds) = value.parse::<i64>() else {
            return Some(DateTime::<Utc>::MAX_UTC);
        };
        return Some(if reset {
            DateTime::from_timestamp(seconds, 0)
                .map(|date| after(date, RESET_MARGIN_SECONDS))
                .unwrap_or(DateTime::<Utc>::MAX_UTC)
        } else {
            after(now, seconds)
        });
    }
    if !reset && value.len() <= 64 {
        return DateTime::parse_from_rfc2822(value)
            .ok()
            .map(|date| date.with_timezone(&Utc));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-09-06T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn observation(limit: u64, remaining: u64, cost: u64, reset: DateTime<Utc>) -> Value {
        json!({"data": {"rateLimit": {
            "limit": limit, "remaining": remaining, "cost": cost,
            "resetAt": reset.to_rfc3339()
        }}})
    }

    fn wait(state: &BudgetState, time: DateTime<Utc>) -> GitHubWait {
        *state
            .check(time)
            .unwrap_err()
            .downcast_ref::<GitHubWait>()
            .unwrap()
    }

    fn state_json(state: &BudgetState) -> Value {
        serde_json::to_value(state.snapshot()).unwrap()
    }

    #[test]
    fn low_quota_accepts_response_but_blocks_next_request_and_clones_share_state() {
        let state = BudgetState::default();
        let clone = state.clone();
        let reset = after(now(), 3600);
        state
            .observe_json(&observation(5000, 12, 3, reset), now())
            .unwrap();
        assert_eq!(
            wait(&clone, now()),
            GitHubWait {
                reason: FactoryRetryReason::GraphqlQuota,
                next_retry_at: after(reset, RESET_MARGIN_SECONDS),
            }
        );
        assert_eq!(state.snapshot().consecutive_failures, 0);
        let mut detached = clone.snapshot();
        detached.next_retry_at = None;
        assert!(detached.next_retry_at.is_none());
        assert!(state.check(now()).is_err());
    }

    #[test]
    fn reserve_scales_down_and_includes_actual_observed_cost() {
        for (limit, remaining, cost, blocked) in [
            (5000, 26, 1, false), // exactly cost + 25 is enough
            (5000, 25, 1, true),
            (100, 10, 1, false), // not a fixed 25-point reserve
            (100, 10, 10, true),
            (2, 2, 1, false),
            (2, 1, 1, true),
            (1, 0, 1, true),
            (100, 0, 0, true),
        ] {
            let state = BudgetState::default();
            state
                .observe_json(
                    &observation(limit, remaining, cost, after(now(), 3600)),
                    now(),
                )
                .unwrap();
            assert_eq!(state.check(now()).is_err(), blocked);
        }
    }

    #[test]
    fn reset_margin_expires_without_inventing_a_new_balance() {
        let state = BudgetState::default();
        let reset = after(now(), 60);
        state
            .observe_json(&observation(100, 0, 1, reset), now())
            .unwrap();
        state.success(after(now(), 30));
        assert!(state.check(after(reset, 4)).is_err());
        assert!(state.check(after(reset, 5)).is_ok());
        assert_eq!(state.snapshot().graphql.unwrap().remaining, 0);
        state.success(after(reset, 5));
        assert!(state.snapshot().next_retry_at.is_none());
        state
            .observe_json(
                &observation(100, 99, 1, after(reset, 3600)),
                after(reset, 6),
            )
            .unwrap();
        assert!(state.check(after(reset, 6)).is_ok());
    }

    #[test]
    fn high_balance_does_not_cancel_an_outstanding_wait() {
        let state = BudgetState::default();
        state
            .observe_json(&observation(100, 0, 1, after(now(), 300)), now())
            .unwrap();
        let expected = wait(&state, now());
        state
            .observe_json(
                &observation(100, 99, 1, after(now(), 3600)),
                after(now(), 1),
            )
            .unwrap();
        state.success(after(now(), 2));
        assert_eq!(wait(&state, after(now(), 2)), expected);
    }

    #[test]
    fn only_data_rate_limit_is_an_observation() {
        let state = BudgetState::default();
        state
            .observe_json(
                &json!({
                    "rateLimit": {"remaining": 0},
                    "rate": {"remaining": 0},
                    "resources": {"graphql": {"remaining": 0}}
                }),
                now(),
            )
            .unwrap();
        assert!(state.snapshot().graphql.is_none());
        assert!(state.check(now()).is_ok());
    }

    #[test]
    fn stale_and_out_of_order_observations_do_not_restore_spent_budget() {
        let state = BudgetState::default();
        let reset = after(now(), 3600);
        state
            .observe_json(&observation(100, 5, 1, reset), now())
            .unwrap();
        let before = state_json(&state);
        for (value, observed) in [
            (observation(100, 99, 1, reset), after(now(), 1)),
            (observation(100, 0, 1, reset), after(now(), -1)),
            (observation(100, 99, 1, after(now(), -60)), now()),
            (observation(100, 99, 1, after(reset, -60)), after(now(), 2)),
        ] {
            state.observe_json(&value, observed).unwrap();
            assert_eq!(state_json(&state), before);
        }
    }

    #[test]
    fn invalid_observations_leave_state_intact_and_errors_do_not_echo_values() {
        let state = BudgetState::default();
        let valid = observation(100, 0, 1, after(now(), 3600));
        state.observe_json(&valid, now()).unwrap();
        let before = state_json(&state);
        for (key, value) in [
            ("limit", json!(0)),
            ("limit", json!(u64::MAX)),
            ("remaining", json!(-1)),
            ("remaining", json!(101)),
            ("cost", json!(u64::MAX)),
            ("cost", json!(1.5)),
            ("cost", Value::Null),
            ("resetAt", json!("SECRET_QUERY_AND_TOKEN")),
            ("resetAt", json!("x".repeat(100))),
            ("resetAt", json!("9999-01-01T00:00:00Z")),
        ] {
            let mut value_under_test = valid.clone();
            value_under_test["data"]["rateLimit"][key] = value;
            let error = state.observe_json(&value_under_test, now()).unwrap_err();
            assert_eq!(
                error.to_string(),
                "invalid GitHub GraphQL quota observation"
            );
            assert_eq!(state_json(&state), before);
        }
        assert!(
            state
                .observe_json(&json!({"data":{"rateLimit":null}}), now())
                .is_err()
        );
    }

    #[test]
    fn restart_round_trip_and_stale_reconcile_preserve_the_longest_wait() {
        let original = BudgetState::default();
        let error = GitHubWait {
            reason: FactoryRetryReason::SecondaryRateLimit,
            next_retry_at: after(now(), 7200),
        };
        assert!(original.failure(&error.into(), now()));
        let persisted: FactoryPollingState = serde_json::from_value(state_json(&original)).unwrap();
        let restored = BudgetState::default();
        let clone = restored.clone();
        restored.restore(persisted, after(now(), 30));
        restored.restore(FactoryPollingState::default(), after(now(), 40));
        restored.success(after(now(), 50));
        assert_eq!(wait(&clone, after(now(), 50)), error);
        assert_eq!(clone.snapshot().consecutive_failures, 1);
        restored.success(after(now(), 7200));
        assert!(clone.check(after(now(), 7200)).is_ok());
        assert_eq!(clone.snapshot().consecutive_failures, 0);
    }

    #[test]
    fn restore_reconstructs_low_quota_wait_and_bounds_bad_persisted_counters() {
        let state = BudgetState::default();
        state.restore(
            FactoryPollingState {
                graphql: Some(GithubGraphqlQuota {
                    limit: 100,
                    remaining: 0,
                    cost: 1,
                    reset_at: after(now(), 3600),
                    observed_at: now(),
                }),
                ..FactoryPollingState::default()
            },
            now(),
        );
        let expected = wait(&state, now());
        state.restore(
            FactoryPollingState {
                consecutive_failures: u32::MAX,
                retry_reason: Some(FactoryRetryReason::SecondaryRateLimit),
                ..FactoryPollingState::default()
            },
            now(),
        );
        assert_eq!(wait(&state, now()), expected);
        assert_eq!(state.snapshot().consecutive_failures, MAX_FAILURES);
    }

    #[test]
    fn invalid_persisted_quota_fails_conservatively_without_shortening_valid_wait() {
        let state = BudgetState::default();
        let deadline = after(now(), 7 * 24 * 3600);
        state.restore(
            FactoryPollingState {
                next_retry_at: Some(deadline),
                graphql: Some(GithubGraphqlQuota {
                    limit: u64::MAX,
                    remaining: u64::MAX,
                    cost: 1,
                    reset_at: DateTime::<Utc>::MAX_UTC,
                    observed_at: after(now(), 100),
                }),
                ..FactoryPollingState::default()
            },
            now(),
        );
        assert!(state.snapshot().graphql.is_none());
        assert_eq!(wait(&state, now()).next_retry_at, deadline);
        assert_eq!(
            wait(&state, now()).reason,
            FactoryRetryReason::GithubUnavailable
        );
    }

    #[test]
    fn expired_failures_escalate_until_a_success_and_never_overflow() {
        let state = BudgetState::default();
        let mut time = now();
        for attempt in 1..=40 {
            let error = GitHubWait {
                reason: FactoryRetryReason::GithubUnavailable,
                next_retry_at: after(time, 60),
            };
            assert!(state.failure(&error.into(), time));
            let delay = [60, 120, 240, 480, 900][(attempt - 1).min(4)];
            assert_eq!(wait(&state, time).next_retry_at, after(time, delay));
            assert!(state.snapshot().consecutive_failures <= MAX_FAILURES);
            time = after(time, delay);
            assert!(state.check(time).is_ok());
        }
        state.success(time);
        assert_eq!(state.snapshot().consecutive_failures, 0);
        assert!(state.snapshot().retry_reason.is_none());
    }

    #[test]
    fn typed_failure_only_and_admission_wait_is_not_a_failed_attempt() {
        let state = BudgetState::default();
        assert!(!state.failure(
            &anyhow!("secondary rate limit SECRET_QUERY_AND_TOKEN"),
            now()
        ));
        assert!(state.snapshot().next_retry_at.is_none());
        state
            .observe_json(&observation(100, 0, 1, after(now(), 3600)), now())
            .unwrap();
        let error = state
            .check(now())
            .unwrap_err()
            .context("private caller context");
        assert!(state.failure(&error, now()));
        assert_eq!(state.snapshot().consecutive_failures, 0);
        assert!(!state_json(&state).to_string().contains("private caller"));
    }

    #[test]
    fn primary_failure_uses_known_reset_but_secondary_does_not_use_high_quota_reset() {
        for (reason, delay) in [
            (FactoryRetryReason::PrimaryRateLimit, 3605),
            (FactoryRetryReason::SecondaryRateLimit, 60),
        ] {
            let state = BudgetState::default();
            state
                .observe_json(&observation(5000, 4000, 1, after(now(), 3600)), now())
                .unwrap();
            state.failure(
                &GitHubWait {
                    reason,
                    next_retry_at: after(now(), 60),
                }
                .into(),
                now(),
            );
            assert_eq!(wait(&state, now()).next_retry_at, after(now(), delay));
        }
    }

    #[test]
    fn primary_headers_and_duplicate_hints_choose_the_latest_deadline() {
        let stdout = format!(
            "HTTP/2.0 403 Forbidden\r\nX-RateLimit-Remaining: 0\r\n\
             X-RateLimit-Reset: {}\r\nRetry-After: 7200\r\nRetry-After: 10\r\n\r\n\
             {{\"message\":\"API rate limit exceeded\"}}",
            after(now(), 3600).timestamp()
        );
        let error = classify_failure(stdout.as_bytes(), b"", now()).unwrap();
        assert_eq!(error.reason, FactoryRetryReason::PrimaryRateLimit);
        assert_eq!(error.next_retry_at, after(now(), 7200));
        let state = BudgetState::default();
        state.failure(&error.into(), now());
        assert_eq!(wait(&state, now()).next_retry_at, error.next_retry_at);
    }

    #[test]
    fn reset_header_has_a_safety_margin() {
        let stdout = format!(
            "HTTP/2 429\r\nx-ratelimit-remaining: 0\r\nx-ratelimit-reset: {}\r\n\r\n{{}}",
            after(now(), 3600).timestamp()
        );
        let error = classify_failure(stdout.as_bytes(), b"", now()).unwrap();
        assert_eq!(error.reason, FactoryRetryReason::PrimaryRateLimit);
        assert_eq!(error.next_retry_at, after(now(), 3605));
    }

    #[test]
    fn secondary_403_and_503_ignore_healthy_primary_reset_and_escalate_locally() {
        for (status, message, reason) in [
            (
                403,
                "You have exceeded a secondary rate limit.",
                FactoryRetryReason::SecondaryRateLimit,
            ),
            (
                503,
                "Service unavailable",
                FactoryRetryReason::GithubUnavailable,
            ),
        ] {
            let reset = after(now(), 3600);
            let body = json!({"message": message});
            let stdout = format!(
                "HTTP/2 {status}\r\nX-RateLimit-Remaining: 4990\r\n\
                 X-RateLimit-Reset: {}\r\nRetry-After: 2\r\n\r\n{body}",
                reset.timestamp()
            );
            let state = BudgetState::default();
            state
                .observe_json(&observation(5000, 4990, 1, reset), now())
                .unwrap();
            let first = classify_failure(stdout.as_bytes(), b"", now()).unwrap();
            assert_eq!(first.reason, reason);
            assert_eq!(first.next_retry_at, after(now(), 2));
            assert!(state.failure(&first.into(), now()));
            assert_eq!(wait(&state, now()).next_retry_at, after(now(), 60));

            let retry_at = after(now(), 60);
            assert!(state.check(retry_at).is_ok());
            let second = classify_failure(stdout.as_bytes(), b"", retry_at).unwrap();
            assert_eq!(second.next_retry_at, after(retry_at, 2));
            assert!(state.failure(&second.into(), retry_at));
            assert_eq!(wait(&state, retry_at).next_retry_at, after(retry_at, 120));
        }
    }

    #[test]
    fn primary_or_explicitly_exhausted_responses_still_respect_hourly_reset() {
        for (status, remaining, message, reason) in [
            (
                403,
                4990,
                "API rate limit exceeded",
                FactoryRetryReason::PrimaryRateLimit,
            ),
            (
                403,
                0,
                "Request forbidden",
                FactoryRetryReason::PrimaryRateLimit,
            ),
            (
                403,
                0,
                "You have exceeded a secondary rate limit.",
                FactoryRetryReason::SecondaryRateLimit,
            ),
            (
                503,
                0,
                "Service unavailable",
                FactoryRetryReason::GithubUnavailable,
            ),
        ] {
            let body = json!({"message": message});
            let stdout = format!(
                "HTTP/2 {status}\r\nX-RateLimit-Remaining: {remaining}\r\n\
                 X-RateLimit-Reset: {}\r\nRetry-After: 2\r\n\r\n{body}",
                after(now(), 3600).timestamp()
            );
            let error = classify_failure(stdout.as_bytes(), b"", now()).unwrap();
            assert_eq!(error.reason, reason);
            assert_eq!(error.next_retry_at, after(now(), 3605));
            let state = BudgetState::default();
            assert!(state.failure(&error.into(), now()));
            assert_eq!(wait(&state, now()).next_retry_at, error.next_retry_at);
        }
    }

    #[test]
    fn secondary_retry_after_supports_http_dates_and_stderr_headers() {
        let stdout = b"HTTP/2 403\r\nRetry-After: Sun, 06 Sep 2026 14:00:00 GMT\r\n\r\n\
                       {\"message\":\"You have exceeded a secondary rate limit.\"}";
        let error = classify_failure(stdout, b"", now()).unwrap();
        assert_eq!(error.reason, FactoryRetryReason::SecondaryRateLimit);
        assert_eq!(error.next_retry_at, after(now(), 7200));
        let stderr = b"gh: secondary rate limit (HTTP 403)\nretry-after: 1800";
        assert_eq!(
            classify_failure(b"{}", stderr, now())
                .unwrap()
                .next_retry_at,
            after(now(), 1800)
        );
    }

    #[test]
    fn graphql_rate_limited_and_http_failures_are_typed() {
        for stdout in [
            br#"{"errors":[{"type":"RATE_LIMITED","message":"hidden"}]}"#.as_slice(),
            br#"{"errors":[{"extensions":{"code":"RATE_LIMITED"}}]}"#.as_slice(),
        ] {
            let error = classify_failure(stdout, b"", now()).unwrap();
            assert_eq!(error.reason, FactoryRetryReason::PrimaryRateLimit);
            assert_eq!(error.next_retry_at, after(now(), 60));
        }
        for (stderr, reason) in [
            (
                "gh: request failed (HTTP 429)",
                FactoryRetryReason::SecondaryRateLimit,
            ),
            ("HTTP429", FactoryRetryReason::SecondaryRateLimit),
            (
                "gh: request failed (HTTP 503)",
                FactoryRetryReason::GithubUnavailable,
            ),
            (
                "abuse detection mechanism",
                FactoryRetryReason::SecondaryRateLimit,
            ),
        ] {
            assert_eq!(
                classify_failure(b"", stderr.as_bytes(), now())
                    .unwrap()
                    .reason,
                reason
            );
        }
    }

    #[test]
    fn successful_data_and_non_retryable_errors_are_not_misclassified() {
        for stdout in [
            br#"{"data":{"issue":{"body":"secondary rate limit HTTP 503 RATE_LIMITED"}}}"#
                .as_slice(),
            b"HTTP/2 200\nx-ratelimit-remaining: 0\n\n{\"data\":{}}".as_slice(),
            br#"{"errors":[{"type":"NOT_FOUND","message":"not found"}]}"#.as_slice(),
            b"HTTP/2 401\n\n{\"message\":\"Bad credentials\"}".as_slice(),
            b"HTTP/2 403\n\n{\"message\":\"Resource not accessible by integration\"}".as_slice(),
        ] {
            assert!(classify_failure(stdout, b"", now()).is_none());
        }
        assert!(classify_failure(b"{}", b"HTTP 4290", now()).is_none());
    }

    #[test]
    fn plain_json_fixtures_are_unchanged_and_http_preambles_are_stripped() {
        for plain in [
            b"{}".as_slice(),
            b" \r\n[1,2]\n".as_slice(),
            b"true".as_slice(),
        ] {
            assert_eq!(json_body(plain).unwrap(), plain);
        }
        for included in [
            b"HTTP/2.0 200 OK\r\nContent-Type: application/json\r\n\r\n{\"data\":{}}".as_slice(),
            b"HTTP/2 200\ncontent-type: application/json\n\n{\"data\":{}}".as_slice(),
            b"HTTP/1.1 200 Connection established\r\n\r\nHTTP/1.1 100 Continue\r\n\r\n\
              HTTP/2 200\r\nContent-Type: application/json\r\n\r\n{\"data\":{}}"
                .as_slice(),
        ] {
            assert_eq!(json_body(included).unwrap(), br#"{"data":{}}"#);
        }
    }

    #[test]
    fn final_http_status_is_used_and_prior_valid_retry_hints_are_not_shortened() {
        let stdout = b"HTTP/1.1 100 Continue\r\nRetry-After: 7200\r\n\r\n\
                       HTTP/2 503\r\nRetry-After: 60\r\n\r\n{}";
        let error = classify_failure(stdout, b"", now()).unwrap();
        assert_eq!(error.reason, FactoryRetryReason::GithubUnavailable);
        assert_eq!(error.next_retry_at, after(now(), 7200));
        assert_eq!(json_body(stdout).unwrap(), b"{}");
    }

    #[test]
    fn malformed_or_oversized_headers_have_safe_errors() {
        for bytes in [
            b"HTTP/2 200\r\nSECRET_QUERY_AND_TOKEN".as_slice(),
            b"HTTP/2 XYZ\n\n{}".as_slice(),
            b"HTTP/2 200\nnot-a-header\n\n{}".as_slice(),
            b"HTTP/2 200\nsecret:\xff\n\n{}".as_slice(),
        ] {
            let error = json_body(bytes).unwrap_err().to_string();
            assert!(!error.contains("SECRET_QUERY_AND_TOKEN"));
            assert!(error.len() < 120);
            assert_eq!(
                classify_failure(bytes, b"", now()).unwrap().next_retry_at,
                DateTime::<Utc>::MAX_UTC
            );
        }
        let oversized = format!(
            "HTTP/2 200\nx-header: {}\n\n{{}}",
            "x".repeat(MAX_HEADER_BYTES)
        );
        assert!(json_body(oversized.as_bytes()).is_err());
        let repeated = format!(
            "{}{{}}",
            "HTTP/1.1 100 Continue\n\n".repeat(MAX_HEADER_BLOCKS + 1)
        );
        assert!(json_body(repeated.as_bytes()).is_err());
    }

    #[test]
    fn oversized_diagnostics_cannot_hide_a_longer_retry_requirement() {
        for stderr in [
            format!("{}\nRetry-After: 7200", "x".repeat(MAX_DIAGNOSTIC_BYTES)),
            format!("{}Retry-After: 7200", "x\n".repeat(MAX_HEADER_LINES)),
        ] {
            let error = classify_failure(b"HTTP/2 429\n\n{}", stderr.as_bytes(), now()).unwrap();
            assert_eq!(error.next_retry_at, DateTime::<Utc>::MAX_UTC);
            assert!(error.to_string().len() < 120);
        }
    }

    #[test]
    fn malformed_hints_fall_back_and_unrepresentable_positive_hints_fail_closed() {
        for hint in ["invalid", "-1", "Sun, 06 Sep 2020 12:00:00 GMT"] {
            let stdout = format!("HTTP/2 503\nRetry-After: {hint}\n\n{{}}");
            assert_eq!(
                classify_failure(stdout.as_bytes(), b"", now())
                    .unwrap()
                    .next_retry_at,
                after(now(), 60)
            );
        }
        for header in [
            "Retry-After: 18446744073709551615",
            "x-ratelimit-reset: 18446744073709551615",
        ] {
            let stdout = format!("HTTP/2 429\nx-ratelimit-remaining: 0\n{header}\n\n{{}}");
            let error = classify_failure(stdout.as_bytes(), b"", now()).unwrap();
            assert_eq!(error.next_retry_at, DateTime::<Utc>::MAX_UTC);
            let state = BudgetState::default();
            state.failure(&error.into(), now());
            state.success(DateTime::<Utc>::MAX_UTC);
            assert!(state.check(DateTime::<Utc>::MAX_UTC).is_err());
        }
        assert_eq!(
            classify_failure(b"HTTP/2 503\n\n{}", b"", DateTime::<Utc>::MAX_UTC)
                .unwrap()
                .next_retry_at,
            DateTime::<Utc>::MAX_UTC
        );
    }

    #[test]
    fn typed_display_debug_and_error_chain_never_contain_remote_secrets() {
        let stdout = b"HTTP/2 429\nAuthorization: SECRET_QUERY_AND_TOKEN\nRetry-After: 60\n\n\
                       {\"errors\":[{\"type\":\"RATE_LIMITED\",\"message\":\"SECRET_QUERY_AND_TOKEN\"}]}";
        let error = classify_failure(stdout, b"gh: SECRET_QUERY_AND_TOKEN", now()).unwrap();
        assert_eq!(error.reason, FactoryRetryReason::PrimaryRateLimit);
        let safe: anyhow::Error = error.into();
        for rendered in [safe.to_string(), format!("{error:?}"), format!("{safe:#}")] {
            assert!(!rendered.contains("SECRET_QUERY_AND_TOKEN"));
            assert!(!rendered.contains("Authorization"));
            assert!(rendered.len() < 160);
        }
        assert!(Error::source(&error).is_none());
    }
}
