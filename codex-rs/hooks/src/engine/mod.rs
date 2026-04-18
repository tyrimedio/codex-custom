pub(crate) mod command_runner;
pub(crate) mod config;
pub(crate) mod discovery;
pub(crate) mod dispatcher;
pub(crate) mod output_parser;
pub(crate) mod schema_loader;

use codex_config::ConfigLayerStack;
use codex_protocol::protocol::HookRunSummary;
use codex_utils_absolute_path::AbsolutePathBuf;

use crate::events::post_tool_use::PostToolUseOutcome;
use crate::events::post_tool_use::PostToolUseRequest;
use crate::events::pre_tool_use::PreToolUseOutcome;
use crate::events::pre_tool_use::PreToolUseRequest;
use crate::events::session_start::SessionStartOutcome;
use crate::events::session_start::SessionStartRequest;
use crate::events::stop::StopOutcome;
use crate::events::stop::StopRequest;
use crate::events::user_prompt_submit::UserPromptSubmitOutcome;
use crate::events::user_prompt_submit::UserPromptSubmitRequest;

#[derive(Debug, Clone)]
pub(crate) struct CommandShell {
    pub program: String,
    pub args: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConfiguredHandler {
    pub event_name: codex_protocol::protocol::HookEventName,
    pub matcher: Option<String>,
    pub command: String,
    pub timeout_sec: u64,
    pub status_message: Option<String>,
    pub source_path: AbsolutePathBuf,
    pub display_order: i64,
}

impl ConfiguredHandler {
    pub fn run_id(&self) -> String {
        format!(
            "{}:{}:{}",
            self.event_name_label(),
            self.display_order,
            self.source_path.display()
        )
    }

    fn event_name_label(&self) -> &'static str {
        match self.event_name {
            codex_protocol::protocol::HookEventName::SessionStart => "session-start",
            codex_protocol::protocol::HookEventName::SessionResume => "session-resume",
            codex_protocol::protocol::HookEventName::SessionEnd => "session-end",
            codex_protocol::protocol::HookEventName::SessionInterrupted => "session-interrupted",
            codex_protocol::protocol::HookEventName::UserPromptSubmit => "user-prompt-submit",
            codex_protocol::protocol::HookEventName::TurnStart => "turn-start",
            codex_protocol::protocol::HookEventName::TurnComplete => "turn-complete",
            codex_protocol::protocol::HookEventName::TurnAbort => "turn-abort",
            codex_protocol::protocol::HookEventName::TurnError => "turn-error",
            codex_protocol::protocol::HookEventName::PreToolUse => "pre-tool-use",
            codex_protocol::protocol::HookEventName::PostToolUse => "post-tool-use",
            codex_protocol::protocol::HookEventName::ToolError => "tool-error",
            codex_protocol::protocol::HookEventName::PermissionRequest => "permission-request",
            codex_protocol::protocol::HookEventName::PermissionDenied => "permission-denied",
            codex_protocol::protocol::HookEventName::ApprovalGranted => "approval-granted",
            codex_protocol::protocol::HookEventName::TaskCreated => "task-created",
            codex_protocol::protocol::HookEventName::TaskStarted => "task-started",
            codex_protocol::protocol::HookEventName::TaskCompleted => "task-completed",
            codex_protocol::protocol::HookEventName::TaskFailed => "task-failed",
            codex_protocol::protocol::HookEventName::SubagentStart => "subagent-start",
            codex_protocol::protocol::HookEventName::SubagentComplete => "subagent-complete",
            codex_protocol::protocol::HookEventName::SubagentEscalation => "subagent-escalation",
            codex_protocol::protocol::HookEventName::FileChanged => "file-changed",
            codex_protocol::protocol::HookEventName::CwdChanged => "cwd-changed",
            codex_protocol::protocol::HookEventName::ConfigChanged => "config-changed",
            codex_protocol::protocol::HookEventName::MemoryUpdated => "memory-updated",
            codex_protocol::protocol::HookEventName::SkillChanged => "skill-changed",
            codex_protocol::protocol::HookEventName::CompactionStart => "compaction-start",
            codex_protocol::protocol::HookEventName::CompactionComplete => "compaction-complete",
            codex_protocol::protocol::HookEventName::ContextTruncated => "context-truncated",
            codex_protocol::protocol::HookEventName::PromptCacheHit => "prompt-cache-hit",
            codex_protocol::protocol::HookEventName::PromptCacheMiss => "prompt-cache-miss",
            codex_protocol::protocol::HookEventName::Stop => "stop",
        }
    }
}

#[derive(Clone)]
pub(crate) struct ClaudeHooksEngine {
    handlers: Vec<ConfiguredHandler>,
    warnings: Vec<String>,
    shell: CommandShell,
}

impl ClaudeHooksEngine {
    pub(crate) fn new(
        enabled: bool,
        config_layer_stack: Option<&ConfigLayerStack>,
        shell: CommandShell,
    ) -> Self {
        if !enabled {
            return Self {
                handlers: Vec::new(),
                warnings: Vec::new(),
                shell,
            };
        }

        let _ = schema_loader::generated_hook_schemas();
        let discovered = discovery::discover_handlers(config_layer_stack);
        Self {
            handlers: discovered.handlers,
            warnings: discovered.warnings,
            shell,
        }
    }

    pub(crate) fn warnings(&self) -> &[String] {
        &self.warnings
    }

    pub(crate) fn preview_session_start(
        &self,
        request: &SessionStartRequest,
    ) -> Vec<HookRunSummary> {
        crate::events::session_start::preview(&self.handlers, request)
    }

    pub(crate) fn preview_pre_tool_use(&self, request: &PreToolUseRequest) -> Vec<HookRunSummary> {
        crate::events::pre_tool_use::preview(&self.handlers, request)
    }

    pub(crate) fn preview_post_tool_use(
        &self,
        request: &PostToolUseRequest,
    ) -> Vec<HookRunSummary> {
        crate::events::post_tool_use::preview(&self.handlers, request)
    }

    pub(crate) async fn run_session_start(
        &self,
        request: SessionStartRequest,
        turn_id: Option<String>,
    ) -> SessionStartOutcome {
        crate::events::session_start::run(&self.handlers, &self.shell, request, turn_id).await
    }

    pub(crate) async fn run_pre_tool_use(&self, request: PreToolUseRequest) -> PreToolUseOutcome {
        crate::events::pre_tool_use::run(&self.handlers, &self.shell, request).await
    }

    pub(crate) async fn run_post_tool_use(
        &self,
        request: PostToolUseRequest,
    ) -> PostToolUseOutcome {
        crate::events::post_tool_use::run(&self.handlers, &self.shell, request).await
    }

    pub(crate) fn preview_user_prompt_submit(
        &self,
        request: &UserPromptSubmitRequest,
    ) -> Vec<HookRunSummary> {
        crate::events::user_prompt_submit::preview(&self.handlers, request)
    }

    pub(crate) async fn run_user_prompt_submit(
        &self,
        request: UserPromptSubmitRequest,
    ) -> UserPromptSubmitOutcome {
        crate::events::user_prompt_submit::run(&self.handlers, &self.shell, request).await
    }

    pub(crate) fn preview_stop(&self, request: &StopRequest) -> Vec<HookRunSummary> {
        crate::events::stop::preview(&self.handlers, request)
    }

    pub(crate) async fn run_stop(&self, request: StopRequest) -> StopOutcome {
        crate::events::stop::run(&self.handlers, &self.shell, request).await
    }
}
