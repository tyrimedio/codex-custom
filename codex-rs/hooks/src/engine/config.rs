use serde::Deserialize;

#[derive(Debug, Default, Deserialize)]
pub(crate) struct HooksFile {
    #[serde(default)]
    pub hooks: HookEvents,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct HookEvents {
    #[serde(rename = "SessionStart", default)]
    pub session_start: Vec<MatcherGroup>,
    #[serde(rename = "SessionResume", default)]
    pub session_resume: Vec<MatcherGroup>,
    #[serde(rename = "SessionEnd", default)]
    pub session_end: Vec<MatcherGroup>,
    #[serde(rename = "SessionInterrupted", default)]
    pub session_interrupted: Vec<MatcherGroup>,
    #[serde(rename = "UserPromptSubmit", default)]
    pub user_prompt_submit: Vec<MatcherGroup>,
    #[serde(rename = "TurnStart", default)]
    pub turn_start: Vec<MatcherGroup>,
    #[serde(rename = "TurnComplete", default)]
    pub turn_complete: Vec<MatcherGroup>,
    #[serde(rename = "TurnAbort", default)]
    pub turn_abort: Vec<MatcherGroup>,
    #[serde(rename = "TurnError", default)]
    pub turn_error: Vec<MatcherGroup>,
    #[serde(rename = "PreToolUse", default)]
    pub pre_tool_use: Vec<MatcherGroup>,
    #[serde(rename = "PostToolUse", default)]
    pub post_tool_use: Vec<MatcherGroup>,
    #[serde(rename = "ToolError", default)]
    pub tool_error: Vec<MatcherGroup>,
    #[serde(rename = "PermissionRequest", default)]
    pub permission_request: Vec<MatcherGroup>,
    #[serde(rename = "PermissionDenied", default)]
    pub permission_denied: Vec<MatcherGroup>,
    #[serde(rename = "ApprovalGranted", default)]
    pub approval_granted: Vec<MatcherGroup>,
    #[serde(rename = "TaskCreated", default)]
    pub task_created: Vec<MatcherGroup>,
    #[serde(rename = "TaskStarted", default)]
    pub task_started: Vec<MatcherGroup>,
    #[serde(rename = "TaskCompleted", default)]
    pub task_completed: Vec<MatcherGroup>,
    #[serde(rename = "TaskFailed", default)]
    pub task_failed: Vec<MatcherGroup>,
    #[serde(rename = "SubagentStart", default)]
    pub subagent_start: Vec<MatcherGroup>,
    #[serde(rename = "SubagentComplete", default)]
    pub subagent_complete: Vec<MatcherGroup>,
    #[serde(rename = "SubagentEscalation", default)]
    pub subagent_escalation: Vec<MatcherGroup>,
    #[serde(rename = "FileChanged", default)]
    pub file_changed: Vec<MatcherGroup>,
    #[serde(rename = "CwdChanged", default)]
    pub cwd_changed: Vec<MatcherGroup>,
    #[serde(rename = "ConfigChanged", default)]
    pub config_changed: Vec<MatcherGroup>,
    #[serde(rename = "MemoryUpdated", default)]
    pub memory_updated: Vec<MatcherGroup>,
    #[serde(rename = "SkillChanged", default)]
    pub skill_changed: Vec<MatcherGroup>,
    #[serde(rename = "CompactionStart", default)]
    pub compaction_start: Vec<MatcherGroup>,
    #[serde(rename = "CompactionComplete", default)]
    pub compaction_complete: Vec<MatcherGroup>,
    #[serde(rename = "ContextTruncated", default)]
    pub context_truncated: Vec<MatcherGroup>,
    #[serde(rename = "PromptCacheHit", default)]
    pub prompt_cache_hit: Vec<MatcherGroup>,
    #[serde(rename = "PromptCacheMiss", default)]
    pub prompt_cache_miss: Vec<MatcherGroup>,
    #[serde(rename = "Stop", default)]
    pub stop: Vec<MatcherGroup>,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct MatcherGroup {
    #[serde(default)]
    pub matcher: Option<String>,
    #[serde(default)]
    pub hooks: Vec<HookHandlerConfig>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type")]
pub(crate) enum HookHandlerConfig {
    #[serde(rename = "command")]
    Command {
        command: String,
        #[serde(default, rename = "timeout", alias = "timeoutSec")]
        timeout_sec: Option<u64>,
        #[serde(default)]
        r#async: bool,
        #[serde(default, rename = "statusMessage")]
        status_message: Option<String>,
    },
    #[serde(rename = "prompt")]
    Prompt {},
    #[serde(rename = "agent")]
    Agent {},
}
