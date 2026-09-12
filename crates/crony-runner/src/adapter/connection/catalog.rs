use std::collections::HashSet;

use chrono::{DateTime, Utc};
use crony_domain::{CodingAgent, NativeSignInInputKind, NativeSignInInstruction, RunnerModel};
use serde_json::Value;
use url::Url;

use super::{ProbeError, ProbeResult};
use crate::adapter::AdapterModel;

pub(super) const MODEL_LIMIT: usize = 512;

pub(super) enum AccountState {
    SignedOut,
    SignedIn(Option<String>),
}

pub(super) fn codex_account(result: &Value) -> ProbeResult<AccountState> {
    let account = result.get("account").ok_or(ProbeError::Incompatible(
        "Codex omitted its native account status.",
    ))?;
    if account.is_null() {
        return Ok(AccountState::SignedOut);
    }
    match account.get("type").and_then(Value::as_str) {
        Some("chatgpt") => Ok(AccountState::SignedIn(email_hint(account.get("email")))),
        Some("apiKey") => Ok(AccountState::SignedIn(None)),
        _ => Err(ProbeError::Incompatible(
            "Codex returned an unsupported native account type.",
        )),
    }
}

pub(super) fn claude_account(result: &Value, successful_exit: bool) -> ProbeResult<AccountState> {
    match result.get("loggedIn").and_then(Value::as_bool) {
        Some(false) => Ok(AccountState::SignedOut),
        Some(true) if successful_exit => {
            Ok(AccountState::SignedIn(email_hint(result.get("email"))))
        }
        _ => Err(ProbeError::Incompatible(
            "Claude did not confirm its native authentication status.",
        )),
    }
}

pub(super) fn email_hint(value: Option<&Value>) -> Option<String> {
    let text = value?.as_str()?;
    let (local, domain) = text.split_once('@')?;
    (text.len() <= 254
        && !local.is_empty()
        && domain.contains('.')
        && text
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"@._+-".contains(&byte)))
    .then(|| text.to_owned())
}

pub(super) fn github_hint(text: Option<&str>) -> Option<String> {
    let text = text?;
    (!text.is_empty()
        && text.len() <= 39
        && text
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-'))
    .then(|| text.to_owned())
}

fn model_id(value: Option<&Value>) -> ProbeResult<String> {
    let id = value
        .and_then(Value::as_str)
        .filter(|id| {
            !id.is_empty()
                && id.len() <= 160
                && id
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"._-/:".contains(&byte))
        })
        .ok_or(ProbeError::Incompatible(
            "Native model catalog contained an invalid model ID.",
        ))?;
    Ok(id.to_owned())
}

fn label(value: Option<&Value>, default: &str) -> String {
    value
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty() && text.len() <= 160 && !text.chars().any(char::is_control))
        .unwrap_or(default)
        .to_owned()
}

fn empty_model(id: String, name: String) -> RunnerModel {
    RunnerModel {
        id,
        name,
        policy_state: None,
        policy_terms: None,
        supports_vision: false,
        supports_reasoning_effort: false,
        max_prompt_tokens: None,
        max_context_window_tokens: None,
        supported_reasoning_efforts: Vec::new(),
        default_reasoning_effort: None,
        billing_multiplier: None,
    }
}

pub(super) fn codex_models(result: &Value) -> ProbeResult<Vec<RunnerModel>> {
    let models = bounded_array(result.get("data"))?;
    let mut catalog = Vec::new();
    for model in models {
        if model.get("hidden").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        let id = model_id(model.get("model").or_else(|| model.get("id")))?;
        let mut entry = empty_model(id.clone(), label(model.get("displayName"), &id));
        entry.supports_vision = model
            .get("inputModalities")
            .and_then(Value::as_array)
            .is_some_and(|items| items.iter().any(|item| item.as_str() == Some("image")));
        if let Some(efforts) = model.get("supportedReasoningEfforts") {
            entry.supported_reasoning_efforts = bounded_array(Some(efforts))?
                .iter()
                .map(|effort| model_id(effort.get("reasoningEffort")))
                .collect::<ProbeResult<Vec<_>>>()?;
        }
        entry.supports_reasoning_effort = !entry.supported_reasoning_efforts.is_empty();
        entry.default_reasoning_effort = model
            .get("defaultReasoningEffort")
            .filter(|value| !value.is_null())
            .map(|value| model_id(Some(value)))
            .transpose()?;
        catalog.push(entry);
    }
    Ok(catalog)
}

pub(super) fn claude_models(initialization: &Value) -> ProbeResult<Vec<RunnerModel>> {
    let mut catalog = Vec::new();
    for model in bounded_array(initialization.get("models"))? {
        let id = model_id(model.get("value"))?;
        let mut entry = empty_model(id.clone(), label(model.get("displayName"), &id));
        entry.supports_reasoning_effort = model
            .get("supportsEffort")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if let Some(efforts) = model.get("supportedEffortLevels") {
            entry.supported_reasoning_efforts = bounded_array(Some(efforts))?
                .iter()
                .map(|value| model_id(Some(value)))
                .collect::<ProbeResult<Vec<_>>>()?;
        }
        catalog.push(entry);
    }
    validate_models(&catalog)?;
    Ok(catalog)
}

fn bounded_array(value: Option<&Value>) -> ProbeResult<&Vec<Value>> {
    value
        .and_then(Value::as_array)
        .filter(|items| items.len() <= MODEL_LIMIT)
        .ok_or(ProbeError::Incompatible(
            "Native model catalog is missing or exceeds its bound.",
        ))
}

pub(super) fn validate_models(models: &[RunnerModel]) -> ProbeResult<()> {
    if models.len() > MODEL_LIMIT {
        return Err(ProbeError::Incompatible(
            "Native model catalog exceeds its bound.",
        ));
    }
    let mut ids = HashSet::new();
    for model in models {
        model_id(Some(&Value::String(model.id.clone())))?;
        if !ids.insert(&model.id) {
            return Err(ProbeError::Incompatible(
                "Native model catalog contains duplicate IDs.",
            ));
        }
    }
    Ok(())
}

pub(super) fn runner_model(model: AdapterModel) -> RunnerModel {
    RunnerModel {
        id: model.id,
        name: model.name,
        policy_state: model.policy_state,
        policy_terms: model.policy_terms,
        supports_vision: model.supports_vision,
        supports_reasoning_effort: model.supports_reasoning_effort,
        max_prompt_tokens: model.max_prompt_tokens,
        max_context_window_tokens: model.max_context_window_tokens,
        supported_reasoning_efforts: model.supported_reasoning_efforts,
        default_reasoning_effort: model.default_reasoning_effort,
        billing_multiplier: model.billing_multiplier,
    }
}

pub(super) fn adapter_model(model: RunnerModel) -> AdapterModel {
    AdapterModel {
        id: model.id,
        name: model.name,
        policy_state: model.policy_state,
        policy_terms: model.policy_terms,
        supports_vision: model.supports_vision,
        supports_reasoning_effort: model.supports_reasoning_effort,
        max_prompt_tokens: model.max_prompt_tokens,
        max_context_window_tokens: model.max_context_window_tokens,
        supported_reasoning_efforts: model.supported_reasoning_efforts,
        default_reasoning_effort: model.default_reasoning_effort,
        billing_multiplier: model.billing_multiplier,
    }
}

pub(super) fn sign_in_instruction(
    agent: CodingAgent,
    uri: &str,
    code: Option<&str>,
    expires_at: DateTime<Utc>,
    input_kind: Option<NativeSignInInputKind>,
) -> ProbeResult<NativeSignInInstruction> {
    if !verification_uri_allowed(agent, uri)
        || code.is_some_and(|code| !device_code_allowed(code))
        || (agent != CodingAgent::ClaudeCode && input_kind.is_some())
    {
        return Err(ProbeError::Incompatible(
            "Native sign-in instructions failed the safety boundary.",
        ));
    }
    Ok(NativeSignInInstruction {
        provider: agent.as_str().to_owned(),
        verification_uri: uri.to_owned(),
        user_code: code.map(str::to_owned),
        expires_at,
        input_kind,
        input_id: None,
    })
}

pub(super) fn device_code_allowed(code: &str) -> bool {
    (4..=32).contains(&code.len())
        && code
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'-')
}

pub(super) fn verification_uri_allowed(agent: CodingAgent, uri: &str) -> bool {
    if uri.len() > 4096 || uri.chars().any(char::is_control) {
        return false;
    }
    let Ok(url) = Url::parse(uri) else {
        return false;
    };
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.fragment().is_some()
    {
        return false;
    }
    match agent {
        CodingAgent::Codex => {
            url.host_str() == Some("auth.openai.com")
                && matches!(url.path(), "/codex/device" | "/device")
                && url.query().is_none()
        }
        CodingAgent::GitHubCopilot => {
            url.host_str() == Some("github.com")
                && url.path() == "/login/device"
                && url.query().is_none()
        }
        CodingAgent::ClaudeCode => {
            if !matches!(
                url.host_str(),
                Some("claude.ai" | "console.anthropic.com" | "platform.claude.com")
            ) || url.path() != "/oauth/authorize"
            {
                return false;
            }
            let mut keys = HashSet::new();
            for (key, value) in url.query_pairs() {
                if !keys.insert(key.to_string())
                    || !matches!(
                        key.as_ref(),
                        "client_id"
                            | "code"
                            | "response_type"
                            | "redirect_uri"
                            | "scope"
                            | "state"
                            | "code_challenge"
                            | "code_challenge_method"
                            | "organizationUUID"
                            | "organization_uuid"
                            | "login_hint"
                    )
                    || value.len() > 1024
                    || value.chars().any(char::is_control)
                    // Claude's native manual-code URL uses code=true. An
                    // actual returned authorization code must NEVER be exposed.
                    || (key == "code" && value != "true")
                    || (key == "response_type" && value != "code")
                    || (key == "code_challenge_method" && value != "S256")
                    || (key == "redirect_uri" && !claude_callback_allowed(&value))
                {
                    return false;
                }
            }
            // Do not emit an arbitrary URL that merely resembles a login URL.
            [
                "client_id",
                "response_type",
                "redirect_uri",
                "state",
                "code_challenge",
            ]
            .iter()
            .all(|key| keys.contains(*key))
        }
    }
}

fn claude_callback_allowed(uri: &str) -> bool {
    let Ok(url) = Url::parse(uri) else {
        return false;
    };
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return false;
    }
    match (url.scheme(), url.host_str(), url.path()) {
        ("http", Some("localhost" | "127.0.0.1"), "/callback") => true,
        (
            "https",
            Some("claude.ai" | "console.anthropic.com" | "platform.claude.com"),
            "/oauth/code/callback",
        ) => url.port().is_none(),
        _ => false,
    }
}

/// Bounded, private native output accumulator. It extracts only known URLs and
/// the GitHub device-code shape. Raw lines, errors, auth codes and tokens are
/// never sent to a callback or returned as detail.
#[derive(Default)]
pub(super) struct LoginTranscript {
    text: String,
}

impl LoginTranscript {
    pub(super) fn push(&mut self, chunk: &str) -> ProbeResult<()> {
        if self.text.len() + chunk.len() > 64 * 1024 {
            return Err(ProbeError::Incompatible(
                "Native sign-in output exceeded its bound.",
            ));
        }
        self.text.push_str(chunk);
        Ok(())
    }

    fn plain(&self) -> String {
        let mut plain = String::new();
        let mut escape = false;
        let mut csi = false;
        for ch in self.text.chars() {
            if escape {
                if ch == '[' && !csi {
                    csi = true;
                } else if !csi || ('@'..='~').contains(&ch) {
                    escape = false;
                    csi = false;
                }
                continue;
            }
            if ch == '\x1b' {
                escape = true;
            } else {
                plain.push(ch);
            }
        }
        plain
    }

    pub(super) fn uri(&self, agent: CodingAgent) -> Option<String> {
        let plain = self.plain();
        // A final unterminated token may be just the first pipe chunk of a URL.
        // Wait for its delimiter rather than emit a truncated OAuth challenge.
        for word in plain.split_inclusive(char::is_whitespace) {
            if !word.ends_with(char::is_whitespace) {
                continue;
            }
            let word = word.trim();
            let candidate = word.trim_matches(['\'', '"', '<', '>', '(', ')']);
            if verification_uri_allowed(agent, candidate) {
                return Some(candidate.to_owned());
            }
        }
        None
    }

    pub(super) fn github_code(&self) -> Option<String> {
        let plain = self.plain();
        let lower = plain.to_ascii_lowercase();
        if !lower.contains("code") {
            return None;
        }
        plain
            .split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '-'))
            .find(|word| {
                word.len() == 9
                    && word.as_bytes()[4] == b'-'
                    && word
                        .bytes()
                        .enumerate()
                        .all(|(i, b)| i == 4 || b.is_ascii_uppercase() || b.is_ascii_digit())
            })
            .map(str::to_owned)
    }

    pub(super) fn claude_code_prompt(&self) -> bool {
        let plain = self.plain().to_ascii_lowercase();
        // Native auth-login prompts, not arbitrary words in an error message.
        plain.lines().any(|line| {
            let line = line.trim_start();
            line.starts_with("paste code here")
                || line.starts_with("paste the authorization code")
                || line.starts_with("enter the authorization code")
        })
    }
}

pub(super) fn authorization_code_allowed(code: &str) -> bool {
    !code.is_empty()
        && code.len() <= 2048
        && !code.chars().any(|ch| ch.is_control() || ch.is_whitespace())
}
