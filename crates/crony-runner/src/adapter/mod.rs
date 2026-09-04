mod codex;
mod copilot;
mod external;
mod fake;
mod permission;
mod process_tree;

use std::{collections::HashMap, path::PathBuf, sync::Arc};

use async_trait::async_trait;
use tokio::sync::mpsc;
use uuid::Uuid;

pub use codex::CodexAdapter;
pub use copilot::{CopilotSdkAdapter, CopilotSdkConfig};
pub use external::{ExternalCliAdapter, ExternalFlavor};
pub use fake::FakeProcessAdapter;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FeatureSupport {
    Supported,
    Unsupported { reason: String },
}

impl FeatureSupport {
    pub fn supported(&self) -> bool {
        matches!(self, Self::Supported)
    }

    #[allow(dead_code)] // Used by default unsupported operations and conformance tests.
    pub fn reason(&self) -> Option<&str> {
        match self {
            Self::Supported => None,
            Self::Unsupported { reason } => Some(reason),
        }
    }
}

#[derive(Debug, Clone)]
pub struct AdapterCapabilities {
    pub spawn: FeatureSupport,
    pub stream: FeatureSupport,
    pub steer: FeatureSupport,
    pub interrupt: FeatureSupport,
    pub stop: FeatureSupport,
    pub resume: FeatureSupport,
    pub usage: FeatureSupport,
    pub artifacts: FeatureSupport,
}

impl AdapterCapabilities {
    pub fn summary(&self) -> String {
        let features = [
            ("spawn", &self.spawn),
            ("stream", &self.stream),
            ("steer", &self.steer),
            ("interrupt", &self.interrupt),
            ("stop", &self.stop),
            ("resume", &self.resume),
            ("usage", &self.usage),
            ("artifacts", &self.artifacts),
        ];
        features
            .into_iter()
            .map(|(name, support)| {
                if support.supported() {
                    format!("{name}=yes")
                } else {
                    format!("{name}=no")
                }
            })
            .collect::<Vec<_>>()
            .join(", ")
    }
}

#[derive(Clone)]
pub struct AdapterRunRequest {
    pub run_id: Uuid,
    pub mission_id: Uuid,
    pub task_id: Uuid,
    pub agent_id: Uuid,
    pub mission_title: String,
    pub model: Option<String>,
    pub reasoning_effort: Option<String>,
    pub workspace: PathBuf,
    pub environment: HashMap<String, String>,
}

impl std::fmt::Debug for AdapterRunRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AdapterRunRequest")
            .field("run_id", &self.run_id)
            .field("mission_id", &self.mission_id)
            .field("task_id", &self.task_id)
            .field("agent_id", &self.agent_id)
            .field("mission_title", &self.mission_title)
            .field("model", &self.model)
            .field("reasoning_effort", &self.reasoning_effort)
            .field("workspace", &self.workspace)
            .field(
                "environment_keys",
                &self.environment.keys().collect::<Vec<_>>(),
            )
            .finish()
    }
}
#[derive(Debug, Clone)]
pub enum AdapterControl {
    Steer {
        actor_id: Uuid,
        text: String,
    },
    #[allow(dead_code)] // Reserved for adapters that distinguish interrupt from stop.
    Interrupt {
        reason: String,
    },
    Stop {
        reason: String,
    },
    ApprovalDecision {
        approval_id: Uuid,
        approved: bool,
        note: String,
    },
    CircuitBreaker {
        stage: String,
        reason: String,
    },
}

#[derive(Debug, Clone)]
pub struct AdapterArtifact {
    pub path: PathBuf,
    pub sha256: String,
    pub bytes: usize,
    pub media_type: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AdapterModel {
    pub id: String,
    pub name: String,
    pub policy_state: Option<String>,
    pub policy_terms: Option<String>,
    pub supports_vision: bool,
    pub supports_reasoning_effort: bool,
    pub max_prompt_tokens: Option<u64>,
    pub max_context_window_tokens: Option<u64>,
    pub supported_reasoning_efforts: Vec<String>,
    pub default_reasoning_effort: Option<String>,
    pub billing_multiplier: Option<f64>,
}

#[derive(Debug, Clone)]
pub enum AdapterEvent {
    Session {
        session_id: String,
    },
    Started {
        workspace: PathBuf,
    },
    Status {
        status: String,
        station: String,
        message: String,
    },
    Output {
        stream: String,
        text: String,
    },
    Artifact(AdapterArtifact),
    #[allow(dead_code)] // Real model adapters emit usage; fake-process reports it unsupported.
    Usage(UsageSnapshot),
    ApprovalRequested {
        approval_id: Uuid,
        action_key: String,
        action: String,
        risk: String,
        rationale: String,
        required_roles: Vec<String>,
        expires_in_seconds: u64,
    },
    ToolActivity {
        signature: String,
        progressed: bool,
        human_conversation: bool,
    },
    TeardownUncertain {
        detail: String,
    },
    Completed {
        summary: String,
    },
    Failed {
        error: String,
    },
    Cancelled {
        reason: String,
    },
}

#[derive(Debug, Clone, Default)]
pub struct UsageSnapshot {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_microusd: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdapterExit {
    Completed,
    Failed,
    Cancelled,
}

pub trait AdapterEventSink: Send + Sync {
    fn emit(&self, event: AdapterEvent);
}

#[derive(Debug, thiserror::Error)]
pub enum AdapterError {
    #[allow(dead_code)] // Produced by default lifecycle methods and conformance tests.
    #[error("{feature} is unsupported: {reason}")]
    Unsupported {
        feature: &'static str,
        reason: String,
    },
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Runtime(#[from] anyhow::Error),
}

#[async_trait]
pub trait AgentAdapter: Send + Sync {
    fn id(&self) -> &'static str;
    fn display_name(&self) -> &'static str;
    fn capabilities(&self) -> AdapterCapabilities;

    async fn list_models(&self) -> Result<Vec<AdapterModel>, AdapterError> {
        Ok(Vec::new())
    }

    /// Runs one provider session to a terminal state. Implementations must not
    /// return while their child process or SDK client is still live.
    async fn execute(
        &self,
        request: AdapterRunRequest,
        controls: mpsc::UnboundedReceiver<AdapterControl>,
        sink: Arc<dyn AdapterEventSink>,
    ) -> Result<AdapterExit, AdapterError>;

    /// Resumes persisted provider state in a new bounded runtime. As with
    /// `execute`, returning means the live process/client has been stopped.
    #[allow(dead_code)] // Contract surface for adapters with persistent provider sessions.
    async fn resume(
        &self,
        _request: AdapterRunRequest,
        _session_id: &str,
        _controls: mpsc::UnboundedReceiver<AdapterControl>,
        _sink: Arc<dyn AdapterEventSink>,
    ) -> Result<AdapterExit, AdapterError> {
        Err(unsupported("resume", self.capabilities().resume.reason()))
    }

    #[allow(dead_code)] // Contract surface for adapters that expose post-run usage.
    async fn collect_usage(&self, _session_id: &str) -> Result<UsageSnapshot, AdapterError> {
        Err(unsupported("usage", self.capabilities().usage.reason()))
    }
}

#[allow(dead_code)] // Called by default lifecycle methods when a feature is unsupported.
fn unsupported(feature: &'static str, reason: Option<&str>) -> AdapterError {
    AdapterError::Unsupported {
        feature,
        reason: reason
            .unwrap_or("adapter did not provide a reason")
            .to_owned(),
    }
}

#[derive(Clone)]
pub struct AdapterRegistry {
    adapters: Arc<HashMap<String, Arc<dyn AgentAdapter>>>,
}

pub struct AdapterRegistryConfig {
    pub fake_agent_script: PathBuf,
    pub codex_command: PathBuf,
    pub codex_prefix_args: Vec<std::ffi::OsString>,
    pub claude_command: PathBuf,
    pub claude_prefix_args: Vec<std::ffi::OsString>,
    pub opencode_command: PathBuf,
    pub opencode_prefix_args: Vec<std::ffi::OsString>,
    pub copilot: CopilotSdkConfig,
}

impl AdapterRegistry {
    pub fn new(config: AdapterRegistryConfig) -> Self {
        let fake: Arc<dyn AgentAdapter> =
            Arc::new(FakeProcessAdapter::new(config.fake_agent_script));
        let codex: Arc<dyn AgentAdapter> = Arc::new(CodexAdapter::new_with_prefix(
            config.codex_command,
            config.codex_prefix_args,
        ));
        let claude: Arc<dyn AgentAdapter> = Arc::new(ExternalCliAdapter::new_with_prefix(
            ExternalFlavor::ClaudeCode,
            config.claude_command,
            config.claude_prefix_args,
        ));
        let opencode: Arc<dyn AgentAdapter> = Arc::new(ExternalCliAdapter::new_with_prefix(
            ExternalFlavor::OpenCode,
            config.opencode_command,
            config.opencode_prefix_args,
        ));
        let copilot: Arc<dyn AgentAdapter> = Arc::new(CopilotSdkAdapter::new(config.copilot));
        let mut adapters = HashMap::new();
        adapters.insert(fake.id().to_owned(), fake);
        adapters.insert(codex.id().to_owned(), codex);
        adapters.insert(claude.id().to_owned(), claude);
        adapters.insert(opencode.id().to_owned(), opencode);
        adapters.insert(copilot.id().to_owned(), copilot);
        Self {
            adapters: Arc::new(adapters),
        }
    }

    pub fn get(&self, id: &str) -> Option<Arc<dyn AgentAdapter>> {
        self.adapters.get(id).cloned()
    }

    pub fn all(&self) -> Vec<Arc<dyn AgentAdapter>> {
        let mut adapters = self.adapters.values().cloned().collect::<Vec<_>>();
        adapters.sort_by_key(|adapter| adapter.id());
        adapters
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    #[derive(Default)]
    struct RecordingSink {
        events: Mutex<Vec<AdapterEvent>>,
    }

    impl AdapterEventSink for RecordingSink {
        fn emit(&self, event: AdapterEvent) {
            if let Ok(mut events) = self.events.lock() {
                events.push(event);
            }
        }
    }

    fn test_adapter() -> FakeProcessAdapter {
        let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../scripts/fake-agent.mjs");
        FakeProcessAdapter::new(script)
    }

    fn test_request(title: &str) -> AdapterRunRequest {
        let run_id = Uuid::new_v4();
        AdapterRunRequest {
            run_id,
            mission_id: Uuid::new_v4(),
            task_id: Uuid::new_v4(),
            agent_id: Uuid::new_v4(),
            mission_title: title.to_owned(),
            model: None,
            reasoning_effort: None,
            workspace: std::env::temp_dir()
                .join("crony-adapter-tests")
                .join(run_id.to_string()),
            environment: HashMap::new(),
        }
    }

    #[test]
    fn capabilities_are_explicit() {
        let capabilities = test_adapter().capabilities();
        assert!(capabilities.spawn.supported());
        assert!(capabilities.stream.supported());
        assert!(capabilities.steer.supported());
        assert!(capabilities.stop.supported());
        assert!(capabilities.artifacts.supported());
        assert!(!capabilities.interrupt.supported());
        assert!(capabilities.interrupt.reason().is_some());
        assert!(!capabilities.resume.supported());
        assert!(capabilities.resume.reason().is_some());
        assert!(!capabilities.usage.supported());
        assert!(capabilities.usage.reason().is_some());
    }

    #[tokio::test]
    async fn fake_adapter_passes_spawn_stream_steer_and_artifact_contract() {
        let adapter = test_adapter();
        let request = test_request("adapter conformance");
        let workspace = request.workspace.clone();
        let (control_tx, control_rx) = mpsc::unbounded_channel();
        let sink = Arc::new(RecordingSink::default());
        let task_adapter = adapter.clone();
        let task_sink = sink.clone();
        let task =
            tokio::spawn(async move { task_adapter.execute(request, control_rx, task_sink).await });
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        control_tx
            .send(AdapterControl::Steer {
                actor_id: Uuid::new_v4(),
                text: "record this control message".to_owned(),
            })
            .expect("send steer");
        let exit = task.await.expect("join adapter").expect("adapter execute");
        assert_eq!(exit, AdapterExit::Completed);
        let events = sink.events.lock().expect("recording lock");
        assert!(
            events
                .iter()
                .any(|event| matches!(event, AdapterEvent::Started { .. }))
        );
        assert!(events.iter().any(|event| {
            matches!(event, AdapterEvent::Output { stream, .. } if stream == "control")
        }));
        assert!(
            events
                .iter()
                .any(|event| matches!(event, AdapterEvent::Artifact(_)))
        );
        assert!(
            events
                .iter()
                .any(|event| matches!(event, AdapterEvent::Completed { .. }))
        );
        drop(events);
        assert!(workspace.join("result.md").is_file());
        let _ = std::fs::remove_dir_all(workspace);
    }

    #[tokio::test]
    async fn fake_adapter_passes_stop_contract() {
        let adapter = test_adapter();
        let request = test_request("[slow] adapter stop conformance");
        let workspace = request.workspace.clone();
        let (control_tx, control_rx) = mpsc::unbounded_channel();
        let sink = Arc::new(RecordingSink::default());
        let task_adapter = adapter.clone();
        let task_sink = sink.clone();
        let task =
            tokio::spawn(async move { task_adapter.execute(request, control_rx, task_sink).await });
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        control_tx
            .send(AdapterControl::Stop {
                reason: "contract stop".to_owned(),
            })
            .expect("send stop");
        let exit = task.await.expect("join adapter").expect("adapter execute");
        assert_eq!(exit, AdapterExit::Cancelled);
        assert!(sink.events.lock().expect("recording lock").iter().any(
            |event| matches!(event, AdapterEvent::Cancelled { reason } if reason == "contract stop")
        ));
        let _ = std::fs::remove_dir_all(workspace);
    }

    #[tokio::test]
    async fn unsupported_features_return_typed_errors() {
        let adapter = test_adapter();
        let request = test_request("unsupported contract");
        let (_tx, rx) = mpsc::unbounded_channel();
        let sink = Arc::new(RecordingSink::default());
        let resume = adapter.resume(request, "missing", rx, sink).await;
        assert!(matches!(
            resume,
            Err(AdapterError::Unsupported {
                feature: "resume",
                ..
            })
        ));
        let usage = adapter.collect_usage("missing").await;
        assert!(matches!(
            usage,
            Err(AdapterError::Unsupported {
                feature: "usage",
                ..
            })
        ));
    }
}
