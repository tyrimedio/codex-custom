use super::control::clear_memory_root_contents;
use super::storage::rebuild_raw_memories_file_from_memories;
use super::storage::sync_rollout_summaries_from_memories;
use crate::memories::ensure_layout;
use crate::memories::memory_root;
use crate::memories::raw_memories_file;
use crate::memories::rollout_summaries_dir;
use chrono::TimeZone;
use chrono::Utc;
use codex_config::types::DEFAULT_MEMORIES_MAX_RAW_MEMORIES_FOR_CONSOLIDATION;
use codex_protocol::ThreadId;
use codex_state::Stage1Output;
use codex_utils_absolute_path::AbsolutePathBuf;
use pretty_assertions::assert_eq;
use serde_json::Value;
use std::path::PathBuf;
use tempfile::tempdir;

#[test]
fn memory_root_uses_shared_global_path() {
    let codex_home = AbsolutePathBuf::current_dir().expect("cwd").join("codex");
    assert_eq!(memory_root(&codex_home), codex_home.join("memories"));
}

#[test]
fn stage_one_output_schema_requires_rollout_slug_and_keeps_it_nullable() {
    let schema = crate::memories::phase1::output_schema();
    let properties = schema
        .get("properties")
        .and_then(Value::as_object)
        .expect("properties object");
    let required = schema
        .get("required")
        .and_then(Value::as_array)
        .expect("required array");

    let mut required_keys = required
        .iter()
        .map(|key| key.as_str().expect("required key string"))
        .collect::<Vec<_>>();
    required_keys.sort_unstable();

    assert!(
        properties.contains_key("rollout_slug"),
        "schema should declare rollout_slug"
    );

    let rollout_slug_type = properties
        .get("rollout_slug")
        .and_then(Value::as_object)
        .and_then(|schema| schema.get("type"))
        .and_then(Value::as_array)
        .expect("rollout_slug type array");
    let mut rollout_slug_types = rollout_slug_type
        .iter()
        .map(|entry| entry.as_str().expect("type entry string"))
        .collect::<Vec<_>>();
    rollout_slug_types.sort_unstable();

    assert_eq!(
        required_keys,
        vec!["raw_memory", "rollout_slug", "rollout_summary"]
    );
    assert_eq!(rollout_slug_types, vec!["null", "string"]);
}

#[tokio::test]
async fn clear_memory_root_contents_preserves_root_directory() {
    let dir = tempdir().expect("tempdir");
    let root = dir.path().join("memory");
    let nested_dir = root.join("rollout_summaries");
    tokio::fs::create_dir_all(&nested_dir)
        .await
        .expect("create rollout summaries dir");
    tokio::fs::write(root.join("MEMORY.md"), "stale memory index\n")
        .await
        .expect("write memory index");
    tokio::fs::write(nested_dir.join("rollout.md"), "stale rollout\n")
        .await
        .expect("write rollout summary");

    clear_memory_root_contents(&root)
        .await
        .expect("clear memory root contents");

    assert!(
        tokio::fs::try_exists(&root)
            .await
            .expect("check memory root existence"),
        "memory root should still exist after clearing contents"
    );
    let mut entries = tokio::fs::read_dir(&root)
        .await
        .expect("read memory root after clear");
    assert!(
        entries
            .next_entry()
            .await
            .expect("read next entry")
            .is_none(),
        "memory root should be empty after clearing contents"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn clear_memory_root_contents_rejects_symlinked_root() {
    let dir = tempdir().expect("tempdir");
    let target = dir.path().join("outside");
    tokio::fs::create_dir_all(&target)
        .await
        .expect("create symlink target dir");
    let target_file = target.join("keep.txt");
    tokio::fs::write(&target_file, "keep\n")
        .await
        .expect("write target file");

    let root = dir.path().join("memory");
    std::os::unix::fs::symlink(&target, &root).expect("create memory root symlink");

    let err = clear_memory_root_contents(&root)
        .await
        .expect_err("symlinked memory root should be rejected");
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    assert!(
        tokio::fs::try_exists(&target_file)
            .await
            .expect("check target file existence"),
        "rejecting a symlinked memory root should not delete the symlink target"
    );
}

#[tokio::test]
async fn sync_rollout_summaries_and_raw_memories_file_keeps_latest_memories_only() {
    let dir = tempdir().expect("tempdir");
    let root = dir.path().join("memory");
    ensure_layout(&root).await.expect("ensure layout");

    let keep_id = ThreadId::default().to_string();
    let drop_id = ThreadId::default().to_string();
    let keep_path = rollout_summaries_dir(&root).join(format!("{keep_id}.md"));
    let drop_path = rollout_summaries_dir(&root).join(format!("{drop_id}.md"));
    tokio::fs::write(&keep_path, "keep")
        .await
        .expect("write keep");
    tokio::fs::write(&drop_path, "drop")
        .await
        .expect("write drop");

    let memories = vec![Stage1Output {
        thread_id: ThreadId::try_from(keep_id.clone()).expect("thread id"),
        source_updated_at: Utc.timestamp_opt(100, 0).single().expect("timestamp"),
        raw_memory: "raw memory".to_string(),
        rollout_summary: "short summary".to_string(),
        rollout_slug: None,
        rollout_path: PathBuf::from("/tmp/rollout-100.jsonl"),
        cwd: PathBuf::from("/tmp/workspace"),
        git_branch: None,
        generated_at: Utc.timestamp_opt(101, 0).single().expect("timestamp"),
    }];

    sync_rollout_summaries_from_memories(
        &root,
        &memories,
        DEFAULT_MEMORIES_MAX_RAW_MEMORIES_FOR_CONSOLIDATION,
    )
    .await
    .expect("sync rollout summaries");
    rebuild_raw_memories_file_from_memories(
        &root,
        &memories,
        DEFAULT_MEMORIES_MAX_RAW_MEMORIES_FOR_CONSOLIDATION,
    )
    .await
    .expect("rebuild raw memories");

    assert!(
        !tokio::fs::try_exists(&keep_path)
            .await
            .expect("check stale keep path"),
        "sync should prune stale filename that used thread id only"
    );
    assert!(
        !tokio::fs::try_exists(&drop_path)
            .await
            .expect("check stale drop path"),
        "sync should prune stale filename for dropped thread"
    );

    let mut dir = tokio::fs::read_dir(rollout_summaries_dir(&root))
        .await
        .expect("open rollout summaries dir");
    let mut files = Vec::new();
    while let Some(entry) = dir.next_entry().await.expect("read dir entry") {
        files.push(entry.file_name().to_string_lossy().to_string());
    }
    files.sort_unstable();
    assert_eq!(files.len(), 1);
    let canonical_rollout_summary_file = &files[0];

    let raw_memories = tokio::fs::read_to_string(raw_memories_file(&root))
        .await
        .expect("read raw memories");
    assert!(raw_memories.contains("raw memory"));
    assert!(raw_memories.contains(&keep_id));
    assert!(raw_memories.contains("cwd: /tmp/workspace"));
    assert!(raw_memories.contains("rollout_path: /tmp/rollout-100.jsonl"));
    assert!(raw_memories.contains(&format!(
        "rollout_summary_file: {canonical_rollout_summary_file}"
    )));
    let thread_header = format!("## Thread `{keep_id}`");
    let thread_pos = raw_memories
        .find(&thread_header)
        .expect("thread header should exist");
    let updated_pos = raw_memories[thread_pos..]
        .find("updated_at: ")
        .map(|offset| thread_pos + offset)
        .expect("updated_at should exist after thread header");
    let cwd_pos = raw_memories[thread_pos..]
        .find("cwd: /tmp/workspace")
        .map(|offset| thread_pos + offset)
        .expect("cwd should exist after thread header");
    let rollout_path_pos = raw_memories[thread_pos..]
        .find("rollout_path: /tmp/rollout-100.jsonl")
        .map(|offset| thread_pos + offset)
        .expect("rollout_path should exist after thread header");
    let file_pos = raw_memories[thread_pos..]
        .find(&format!(
            "rollout_summary_file: {canonical_rollout_summary_file}"
        ))
        .map(|offset| thread_pos + offset)
        .expect("rollout_summary_file should exist after thread header");
    assert!(thread_pos < updated_pos);
    assert!(updated_pos < cwd_pos);
    assert!(cwd_pos < rollout_path_pos);
    assert!(rollout_path_pos < file_pos);
}

#[tokio::test]
async fn sync_rollout_summaries_uses_timestamp_hash_and_sanitized_slug_filename() {
    let dir = tempdir().expect("tempdir");
    let root = dir.path().join("memory");
    ensure_layout(&root).await.expect("ensure layout");

    let thread_id = ThreadId::new();
    let stale_unslugged_path = rollout_summaries_dir(&root).join(format!("{thread_id}.md"));
    let stale_old_slug_path =
        rollout_summaries_dir(&root).join(format!("{thread_id}--old-slug.md"));
    tokio::fs::write(&stale_unslugged_path, "stale")
        .await
        .expect("write stale unslugged file");
    tokio::fs::write(&stale_old_slug_path, "stale")
        .await
        .expect("write stale old-slug file");

    let memories = vec![Stage1Output {
        thread_id,
        source_updated_at: Utc.timestamp_opt(200, 0).single().expect("timestamp"),
        raw_memory: "raw memory".to_string(),
        rollout_summary: "short summary".to_string(),
        rollout_slug: Some("Unsafe Slug/With Spaces & Symbols + EXTRA_LONG_12345".to_string()),
        rollout_path: PathBuf::from("/tmp/rollout-200.jsonl"),
        cwd: PathBuf::from("/tmp/workspace"),
        git_branch: Some("feature/memory-branch".to_string()),
        generated_at: Utc.timestamp_opt(201, 0).single().expect("timestamp"),
    }];

    sync_rollout_summaries_from_memories(
        &root,
        &memories,
        DEFAULT_MEMORIES_MAX_RAW_MEMORIES_FOR_CONSOLIDATION,
    )
    .await
    .expect("sync rollout summaries");

    let mut dir = tokio::fs::read_dir(rollout_summaries_dir(&root))
        .await
        .expect("open rollout summaries dir");
    let mut files = Vec::new();
    while let Some(entry) = dir.next_entry().await.expect("read dir entry") {
        files.push(entry.file_name().to_string_lossy().to_string());
    }
    files.sort_unstable();

    assert_eq!(files.len(), 1);
    let file_name = &files[0];
    let stem = file_name
        .strip_suffix(".md")
        .expect("rollout summary file should end with .md");
    let (prefix, slug) = stem
        .rsplit_once('-')
        .expect("rollout summary filename should include slug");
    let (timestamp, short_hash) = prefix
        .rsplit_once('-')
        .expect("rollout summary filename should include short hash");

    assert_eq!(timestamp.len(), 19, "timestamp should be second precision");
    let parsed_timestamp = chrono::NaiveDateTime::parse_from_str(timestamp, "%Y-%m-%dT%H-%M-%S");
    assert!(
        parsed_timestamp.is_ok(),
        "timestamp should use YYYY-MM-DDThh-mm-ss"
    );
    assert_eq!(short_hash.len(), 4, "short hash should be exactly 4 chars");
    assert!(
        short_hash.chars().all(|ch| ch.is_ascii_alphanumeric()),
        "short hash should use only alphanumeric chars"
    );
    assert!(slug.len() <= 60, "slug should be capped at 60 chars");
    assert!(
        slug.chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_'),
        "slug should be file-safe lowercase ascii with underscores"
    );

    let summary = tokio::fs::read_to_string(rollout_summaries_dir(&root).join(file_name))
        .await
        .expect("read rollout summary");
    assert!(summary.contains(&format!("thread_id: {thread_id}")));
    assert!(summary.contains("rollout_path: /tmp/rollout-200.jsonl"));
    assert!(summary.contains("git_branch: feature/memory-branch"));
    assert!(
        !tokio::fs::try_exists(&stale_unslugged_path)
            .await
            .expect("check stale unslugged path"),
        "slugged sync should prune stale unslugged filename for same thread"
    );
    assert!(
        !tokio::fs::try_exists(&stale_old_slug_path)
            .await
            .expect("check stale old slug path"),
        "slugged sync should prune stale slugged filename for same thread"
    );
}

#[tokio::test]
async fn rebuild_raw_memories_file_adds_canonical_rollout_summary_file_header() {
    let dir = tempdir().expect("tempdir");
    let root = dir.path().join("memory");
    ensure_layout(&root).await.expect("ensure layout");

    let thread_id =
        ThreadId::try_from("0194f5a6-89ab-7cde-8123-456789abcdef").expect("valid thread id");
    let memories = vec![Stage1Output {
        thread_id,
        source_updated_at: Utc.timestamp_opt(200, 0).single().expect("timestamp"),
        raw_memory: "\
---
description: Added a migration test
keywords: codex-state, migrations
---
### Task 1: migration-test
task: add-migration-test
task_group: codex-state
task_outcome: success
- Added regression coverage for migration uniqueness.

### Task 2: validate-migration
task: validate-migration-ordering
task_group: codex-state
task_outcome: success
- Confirmed no ordering regressions."
            .to_string(),
        rollout_summary: "short summary".to_string(),
        rollout_slug: Some("Unsafe Slug/With Spaces & Symbols + EXTRA_LONG_12345".to_string()),
        rollout_path: PathBuf::from("/tmp/rollout-200.jsonl"),
        cwd: PathBuf::from("/tmp/workspace"),
        git_branch: None,
        generated_at: Utc.timestamp_opt(201, 0).single().expect("timestamp"),
    }];

    sync_rollout_summaries_from_memories(
        &root,
        &memories,
        DEFAULT_MEMORIES_MAX_RAW_MEMORIES_FOR_CONSOLIDATION,
    )
    .await
    .expect("sync rollout summaries");
    rebuild_raw_memories_file_from_memories(
        &root,
        &memories,
        DEFAULT_MEMORIES_MAX_RAW_MEMORIES_FOR_CONSOLIDATION,
    )
    .await
    .expect("rebuild raw memories");

    let mut dir = tokio::fs::read_dir(rollout_summaries_dir(&root))
        .await
        .expect("open rollout summaries dir");
    let mut files = Vec::new();
    while let Some(entry) = dir.next_entry().await.expect("read dir entry") {
        files.push(entry.file_name().to_string_lossy().to_string());
    }
    files.sort_unstable();
    assert_eq!(files.len(), 1);
    let canonical_rollout_summary_file = &files[0];

    let raw_memories = tokio::fs::read_to_string(raw_memories_file(&root))
        .await
        .expect("read raw memories");
    let summary = tokio::fs::read_to_string(
        rollout_summaries_dir(&root).join(canonical_rollout_summary_file),
    )
    .await
    .expect("read rollout summary");
    assert!(summary.contains("rollout_path: /tmp/rollout-200.jsonl"));
    assert!(raw_memories.contains(&format!(
        "rollout_summary_file: {canonical_rollout_summary_file}"
    )));
    assert!(raw_memories.contains("description: Added a migration test"));
    assert!(raw_memories.contains("### Task 1: migration-test"));
    assert!(raw_memories.contains("task: add-migration-test"));
    assert!(raw_memories.contains("task_group: codex-state"));
    assert!(raw_memories.contains("task_outcome: success"));
}

mod phase2 {
    use crate::ThreadManager;
    use crate::agent::AgentControl;
    use crate::codex::Session;
    use crate::codex::make_session_and_context;
    use crate::config::Config;
    use crate::config::test_config;
    use crate::memories::PROJECT_MEMORY_AUTO_SECTION_BEGIN;
    use crate::memories::PROJECT_MEMORY_AUTO_SECTION_END;
    use crate::memories::ProjectMemoryTarget;
    use crate::memories::memory_root;
    use crate::memories::phase2;
    use crate::memories::project_memory::apply_project_memory_updates;
    use crate::memories::project_memory::project_fact_candidate_file;
    use crate::memories::project_memory::project_facts_store_file;
    use crate::memories::raw_memories_file;
    use crate::memories::rollout_summaries_dir;
    use crate::project_doc::GLOBAL_MEMORY_DOC_FILENAME;
    use chrono::Duration as ChronoDuration;
    use chrono::Utc;
    use codex_config::Constrained;
    use codex_login::CodexAuth;
    use codex_protocol::ThreadId;
    use codex_protocol::protocol::AskForApproval;
    use codex_protocol::protocol::Op;
    use codex_protocol::protocol::SandboxPolicy;
    use codex_protocol::protocol::SessionSource;
    use codex_protocol::user_input::UserInput;
    use codex_state::Phase2JobClaimOutcome;
    use codex_state::Stage1Output;
    use codex_state::ThreadMetadataBuilder;
    use serde_json::json;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::Duration;
    use tempfile::TempDir;

    fn thread_id(value: &str) -> ThreadId {
        ThreadId::from_string(value).expect("valid thread id")
    }

    fn stage1_output_with_source_updated_at(source_updated_at: i64) -> Stage1Output {
        Stage1Output {
            thread_id: ThreadId::new(),
            source_updated_at: chrono::DateTime::<Utc>::from_timestamp(source_updated_at, 0)
                .expect("valid source_updated_at timestamp"),
            raw_memory: "raw memory".to_string(),
            rollout_summary: "rollout summary".to_string(),
            rollout_slug: None,
            rollout_path: PathBuf::from("/tmp/rollout-summary.jsonl"),
            cwd: PathBuf::from("/tmp/workspace"),
            git_branch: None,
            generated_at: chrono::DateTime::<Utc>::from_timestamp(source_updated_at + 1, 0)
                .expect("valid generated_at timestamp"),
        }
    }

    struct DispatchHarness {
        _codex_home: TempDir,
        config: Arc<Config>,
        session: Arc<Session>,
        manager: ThreadManager,
        state_db: Arc<codex_state::StateRuntime>,
    }

    impl DispatchHarness {
        async fn new() -> Self {
            let codex_home = tempfile::tempdir().expect("create temp codex home");
            let mut config = test_config().await;
            config.codex_home =
                codex_utils_absolute_path::AbsolutePathBuf::from_absolute_path(codex_home.path())
                    .expect("codex home is absolute");
            config.cwd = config.codex_home.clone();
            let config = Arc::new(config);

            let state_db = codex_state::StateRuntime::init(
                config.codex_home.to_path_buf(),
                config.model_provider_id.clone(),
            )
            .await
            .expect("initialize state db");

            let manager = ThreadManager::with_models_provider_and_home_for_tests(
                CodexAuth::from_api_key("dummy"),
                config.model_provider.clone(),
                config.codex_home.to_path_buf(),
                std::sync::Arc::new(codex_exec_server::EnvironmentManager::new(
                    /*exec_server_url*/ None,
                )),
            );
            let (mut session, _turn_context) = make_session_and_context().await;
            session.services.state_db = Some(Arc::clone(&state_db));
            session.services.agent_control = manager.agent_control();

            Self {
                _codex_home: codex_home,
                config,
                session: Arc::new(session),
                manager,
                state_db,
            }
        }

        async fn seed_stage1_output(&self, source_updated_at: i64) {
            self.seed_stage1_output_in_cwd(source_updated_at, self.config.cwd.to_path_buf())
                .await;
        }

        async fn seed_stage1_output_in_cwd(&self, source_updated_at: i64, cwd: PathBuf) {
            let thread_id = ThreadId::new();
            let mut metadata_builder = ThreadMetadataBuilder::new(
                thread_id,
                cwd.join(format!("rollout-{thread_id}.jsonl")),
                Utc::now(),
                SessionSource::Cli,
            );
            metadata_builder.cwd = cwd;
            metadata_builder.model_provider = Some(self.config.model_provider_id.clone());
            let metadata = metadata_builder.build(&self.config.model_provider_id);

            self.state_db
                .upsert_thread(&metadata)
                .await
                .expect("upsert thread metadata");

            let claim = self
                .state_db
                .try_claim_stage1_job(
                    thread_id,
                    self.session.conversation_id,
                    source_updated_at,
                    /*lease_seconds*/ 3_600,
                    /*max_running_jobs*/ 64,
                )
                .await
                .expect("claim stage-1 job");
            let ownership_token = match claim {
                codex_state::Stage1JobClaimOutcome::Claimed { ownership_token } => ownership_token,
                other => panic!("unexpected stage-1 claim outcome: {other:?}"),
            };
            assert!(
                self.state_db
                    .mark_stage1_job_succeeded(
                        thread_id,
                        &ownership_token,
                        source_updated_at,
                        "raw memory",
                        "rollout summary",
                        /*rollout_slug*/ None,
                    )
                    .await
                    .expect("mark stage-1 success"),
                "stage-1 success should enqueue global consolidation"
            );
        }

        async fn shutdown_threads(&self) {
            let report = self
                .manager
                .shutdown_all_threads_bounded(std::time::Duration::from_secs(10))
                .await;
            assert!(report.submit_failed.is_empty());
            assert!(report.timed_out.is_empty());
        }

        fn user_input_ops_count(&self) -> usize {
            self.manager
                .captured_ops()
                .into_iter()
                .filter(|(_, op)| matches!(op, Op::UserInput { .. }))
                .count()
        }

        fn first_user_input_text(&self) -> Option<String> {
            self.manager
                .captured_ops()
                .into_iter()
                .find_map(|(_, op)| match op {
                    Op::UserInput { items, .. } => items.into_iter().find_map(|item| match item {
                        UserInput::Text { text, .. } => Some(text),
                        _ => None,
                    }),
                    _ => None,
                })
        }
    }

    #[test]
    fn completion_watermark_never_regresses_below_claimed_input_watermark() {
        let stage1_output = stage1_output_with_source_updated_at(/*source_updated_at*/ 123);

        let completion = phase2::get_watermark(/*claimed_watermark*/ 1_000, &[stage1_output]);
        pretty_assertions::assert_eq!(completion, 1_000);
    }

    #[test]
    fn completion_watermark_uses_claimed_watermark_when_there_are_no_memories() {
        let completion = phase2::get_watermark(/*claimed_watermark*/ 777, &[]);
        pretty_assertions::assert_eq!(completion, 777);
    }

    #[test]
    fn completion_watermark_uses_latest_memory_timestamp_when_it_is_newer() {
        let older = stage1_output_with_source_updated_at(/*source_updated_at*/ 123);
        let newer = stage1_output_with_source_updated_at(/*source_updated_at*/ 456);

        let completion = phase2::get_watermark(/*claimed_watermark*/ 200, &[older, newer]);
        pretty_assertions::assert_eq!(completion, 456);
    }

    #[tokio::test]
    async fn dispatch_skips_when_global_job_is_not_dirty() {
        let harness = DispatchHarness::new().await;

        phase2::run(&harness.session, Arc::clone(&harness.config)).await;

        pretty_assertions::assert_eq!(harness.user_input_ops_count(), 0);
        let thread_ids = harness.manager.list_thread_ids().await;
        pretty_assertions::assert_eq!(thread_ids.len(), 0);
    }

    #[tokio::test]
    async fn dispatch_skips_when_global_job_is_already_running() {
        let harness = DispatchHarness::new().await;
        harness
            .state_db
            .enqueue_global_consolidation(/*input_watermark*/ 123)
            .await
            .expect("enqueue global consolidation");
        let claimed = harness
            .state_db
            .try_claim_global_phase2_job(ThreadId::new(), /*lease_seconds*/ 3_600)
            .await
            .expect("claim running global lock");
        assert!(
            matches!(claimed, Phase2JobClaimOutcome::Claimed { .. }),
            "precondition should claim the running lock"
        );

        phase2::run(&harness.session, Arc::clone(&harness.config)).await;

        let running_claim = harness
            .state_db
            .try_claim_global_phase2_job(ThreadId::new(), /*lease_seconds*/ 3_600)
            .await
            .expect("claim while lock is still running");
        pretty_assertions::assert_eq!(running_claim, Phase2JobClaimOutcome::SkippedRunning);
        pretty_assertions::assert_eq!(harness.user_input_ops_count(), 0);
        let thread_ids = harness.manager.list_thread_ids().await;
        pretty_assertions::assert_eq!(thread_ids.len(), 0);
    }

    #[tokio::test]
    async fn dispatch_reclaims_stale_global_lock_and_starts_consolidation() {
        let harness = DispatchHarness::new().await;
        harness.seed_stage1_output(Utc::now().timestamp()).await;

        let stale_claim = harness
            .state_db
            .try_claim_global_phase2_job(ThreadId::new(), /*lease_seconds*/ 0)
            .await
            .expect("claim stale global lock");
        assert!(
            matches!(stale_claim, Phase2JobClaimOutcome::Claimed { .. }),
            "stale lock precondition should be claimed"
        );

        phase2::run(&harness.session, Arc::clone(&harness.config)).await;

        let post_dispatch_claim = harness
            .state_db
            .try_claim_global_phase2_job(ThreadId::new(), /*lease_seconds*/ 3_600)
            .await
            .expect("claim after stale lock dispatch");
        assert!(
            matches!(
                post_dispatch_claim,
                Phase2JobClaimOutcome::SkippedRunning | Phase2JobClaimOutcome::SkippedNotDirty
            ),
            "stale-lock dispatch should either keep the reclaimed job running or finish it before re-claim"
        );

        let user_input_ops = harness.user_input_ops_count();
        pretty_assertions::assert_eq!(user_input_ops, 1);
        let thread_ids = harness.manager.list_thread_ids().await;
        pretty_assertions::assert_eq!(thread_ids.len(), 1);
        let thread_id = thread_ids[0];
        let subagent = harness
            .manager
            .get_thread(thread_id)
            .await
            .expect("get consolidation thread");
        let config_snapshot = subagent.config_snapshot().await;
        pretty_assertions::assert_eq!(config_snapshot.approval_policy, AskForApproval::Never);
        pretty_assertions::assert_eq!(
            config_snapshot.cwd.as_path(),
            memory_root(&harness.config.codex_home).as_path()
        );
        match config_snapshot.sandbox_policy {
            SandboxPolicy::WorkspaceWrite { writable_roots, .. } => {
                assert!(
                    writable_roots
                        .iter()
                        .any(|root| root.as_path() == harness.config.codex_home.as_path()),
                    "consolidation subagent should have codex_home as writable root"
                );
            }
            other => panic!("unexpected sandbox policy: {other:?}"),
        }
        subagent.codex.session.ensure_rollout_materialized().await;
        subagent
            .codex
            .session
            .flush_rollout()
            .await
            .expect("subagent rollout should flush");
        let rollout_path = subagent
            .rollout_path()
            .expect("consolidation thread should have a rollout path");
        codex_rollout::state_db::read_repair_rollout_path(
            Some(harness.state_db.as_ref()),
            Some(thread_id),
            Some(/*archived_only*/ false),
            rollout_path.as_path(),
        )
        .await;
        let memory_mode = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let memory_mode = harness
                    .state_db
                    .get_thread_memory_mode(thread_id)
                    .await
                    .expect("read consolidation thread memory mode");
                if memory_mode.is_some() {
                    break memory_mode;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("timed out waiting for consolidation thread memory mode to persist");
        pretty_assertions::assert_eq!(memory_mode.as_deref(), Some("disabled"));

        harness.shutdown_threads().await;
    }

    #[tokio::test]
    async fn dispatch_includes_repo_memory_targets_in_prompt_and_writable_roots() {
        let harness = DispatchHarness::new().await;
        let repo_dir = tempfile::tempdir().expect("create temp repo dir");
        let repo_root = repo_dir.path().join("demo-repo");
        let nested_cwd = repo_root.join("apps/web");
        tokio::fs::create_dir_all(repo_root.join(".git"))
            .await
            .expect("create git marker");
        tokio::fs::create_dir_all(&nested_cwd)
            .await
            .expect("create nested cwd");
        harness
            .seed_stage1_output_in_cwd(Utc::now().timestamp(), nested_cwd.clone())
            .await;

        phase2::run(&harness.session, Arc::clone(&harness.config)).await;

        pretty_assertions::assert_eq!(harness.user_input_ops_count(), 1);
        let project_memory_dir = repo_root.join(".codex");
        assert!(
            !tokio::fs::try_exists(&project_memory_dir)
                .await
                .expect("check project memory dir existence"),
            "phase2 should not pre-write the repo-local .codex directory before merging drafts"
        );
        let canonical_repo_root =
            std::fs::canonicalize(&repo_root).unwrap_or_else(|_| repo_root.clone());
        let canonical_project_memory_dir = canonical_repo_root.join(".codex");
        let canonical_project_memory_file =
            canonical_project_memory_dir.join(GLOBAL_MEMORY_DOC_FILENAME);
        let project_memory_candidate = project_fact_candidate_file(
            memory_root(&harness.config.codex_home).as_path(),
            canonical_repo_root.as_path(),
        );
        let project_facts_store = project_facts_store_file(
            memory_root(&harness.config.codex_home).as_path(),
            canonical_repo_root.as_path(),
        );
        let prompt = harness
            .first_user_input_text()
            .expect("consolidation prompt should be captured");
        assert!(
            prompt.contains(&canonical_project_memory_file.display().to_string()),
            "consolidation prompt should list the repo-local MEMORY target"
        );
        assert!(
            prompt.contains(&project_memory_candidate.display().to_string()),
            "consolidation prompt should list the project-memory candidate path"
        );
        assert!(
            prompt.contains(&project_facts_store.display().to_string()),
            "consolidation prompt should list the accepted project-facts store path"
        );
        assert!(
            prompt.contains("\"schema_version\": 1"),
            "consolidation prompt should describe the candidate JSON schema"
        );
        assert!(
            prompt.contains(PROJECT_MEMORY_AUTO_SECTION_BEGIN),
            "consolidation prompt should mention the generated section begin marker"
        );
        assert!(
            prompt.contains(PROJECT_MEMORY_AUTO_SECTION_END),
            "consolidation prompt should mention the generated section end marker"
        );

        let thread_ids = harness.manager.list_thread_ids().await;
        pretty_assertions::assert_eq!(thread_ids.len(), 1);
        let thread_id = thread_ids[0];
        let subagent = harness
            .manager
            .get_thread(thread_id)
            .await
            .expect("get consolidation thread");
        let config_snapshot = subagent.config_snapshot().await;
        match config_snapshot.sandbox_policy {
            SandboxPolicy::WorkspaceWrite { writable_roots, .. } => {
                assert!(
                    writable_roots
                        .iter()
                        .any(|root| root.as_path() == harness.config.codex_home.as_path()),
                    "consolidation subagent should have codex_home as writable root"
                );
                assert!(
                    writable_roots
                        .iter()
                        .all(|root| root.as_path() != canonical_project_memory_dir.as_path()),
                    "consolidation subagent should not get direct repo-local write access"
                );
            }
            other => panic!("unexpected sandbox policy: {other:?}"),
        }

        harness.shutdown_threads().await;
    }

    #[tokio::test]
    async fn apply_project_memory_updates_merges_generated_section_into_repo_memory_file() {
        let temp = tempfile::tempdir().expect("create temp dir");
        let global_memory_root = temp.path().join("memories");
        let repo_root = temp.path().join("demo-repo");
        let memory_dir = repo_root.join(".codex");
        let memory_file = memory_dir.join(GLOBAL_MEMORY_DOC_FILENAME);
        let candidate_file = project_fact_candidate_file(&global_memory_root, &repo_root);
        let facts_file = project_facts_store_file(&global_memory_root, &repo_root);
        tokio::fs::create_dir_all(candidate_file.parent().expect("candidate parent"))
            .await
            .expect("create candidate dir");
        tokio::fs::create_dir_all(&memory_dir)
            .await
            .expect("create repo memory dir");
        let selected_thread_id = thread_id("00000000-0000-7000-8000-000000000031");
        let removed_thread_id = thread_id("00000000-0000-7000-8000-000000000032");
        tokio::fs::write(
            &memory_file,
            format!(
                "Manual repo note\n\n{PROJECT_MEMORY_AUTO_SECTION_BEGIN}\nOld auto memory\n{PROJECT_MEMORY_AUTO_SECTION_END}\n\nManual footer\n"
            ),
        )
        .await
        .expect("write existing repo memory");
        tokio::fs::write(
            &facts_file,
            serde_json::to_string_pretty(&json!({
                "schema_version": 1,
                "project_root": repo_root.clone(),
                "facts": [
                    {
                        "category": "pitfalls",
                        "fact": "Remove me",
                        "details": ["Only supported by removed thread."],
                        "evidence_thread_ids": [removed_thread_id],
                    }
                ]
            }))
            .expect("serialize existing facts"),
        )
        .await
        .expect("write existing project facts");
        tokio::fs::write(
            &candidate_file,
            serde_json::to_string_pretty(&json!({
                "schema_version": 1,
                "facts": [
                    {
                        "category": "tooling",
                        "fact": "Use pnpm instead of npm.",
                        "details": ["Run `pnpm test --filter web` for targeted checks."],
                        "evidence_thread_ids": [selected_thread_id],
                    },
                    {
                        "category": "architecture",
                        "fact": "Tests live under apps/web.",
                        "details": [],
                        "evidence_thread_ids": [selected_thread_id],
                    }
                ]
            }))
            .expect("serialize candidate facts"),
        )
        .await
        .expect("write project memory candidate");

        let target = ProjectMemoryTarget {
            project_root: repo_root.clone(),
            memory_dir,
            memory_file: memory_file.clone(),
            candidate_file: candidate_file.clone(),
            facts_file: facts_file.clone(),
            selected_thread_ids: vec![selected_thread_id],
            removed_thread_ids: vec![removed_thread_id],
        };

        apply_project_memory_updates(&[target])
            .await
            .expect("apply project memory updates");

        let updated = tokio::fs::read_to_string(&memory_file)
            .await
            .expect("read updated repo memory");
        pretty_assertions::assert_eq!(
            updated,
            format!(
                "Manual repo note\n\n{PROJECT_MEMORY_AUTO_SECTION_BEGIN}\n## Codex Auto Memory\n\n### Architecture\n- Tests live under apps/web.\n\n### Tooling\n- Use pnpm instead of npm.\n  - Run `pnpm test --filter web` for targeted checks.\n{PROJECT_MEMORY_AUTO_SECTION_END}\n\nManual footer\n"
            )
        );
        let stored_facts = tokio::fs::read_to_string(&facts_file)
            .await
            .expect("read accepted project facts");
        let stored_facts_json: serde_json::Value =
            serde_json::from_str(&stored_facts).expect("parse accepted facts json");
        pretty_assertions::assert_eq!(
            stored_facts_json,
            json!({
                "schema_version": 1,
                "project_root": repo_root,
                "facts": [
                    {
                        "category": "architecture",
                        "fact": "Tests live under apps/web.",
                        "details": [],
                        "evidence_thread_ids": [selected_thread_id],
                    },
                    {
                        "category": "tooling",
                        "fact": "Use pnpm instead of npm.",
                        "details": ["Run `pnpm test --filter web` for targeted checks."],
                        "evidence_thread_ids": [selected_thread_id],
                    }
                ]
            })
        );
        assert!(
            !tokio::fs::try_exists(&candidate_file)
                .await
                .expect("check candidate cleanup"),
            "project memory candidates should be removed after applying"
        );
    }

    #[tokio::test]
    async fn dispatch_with_empty_stage1_outputs_rebuilds_local_artifacts() {
        let harness = DispatchHarness::new().await;
        let root = memory_root(&harness.config.codex_home);
        let summaries_dir = rollout_summaries_dir(&root);
        tokio::fs::create_dir_all(&summaries_dir)
            .await
            .expect("create rollout summaries dir");

        let stale_summary_path = summaries_dir.join(format!("{}.md", ThreadId::new()));
        tokio::fs::write(&stale_summary_path, "stale summary\n")
            .await
            .expect("write stale rollout summary");
        let raw_memories_path = raw_memories_file(&root);
        tokio::fs::write(&raw_memories_path, "stale raw memories\n")
            .await
            .expect("write stale raw memories");
        let memory_index_path = root.join("MEMORY.md");
        tokio::fs::write(&memory_index_path, "stale memory index\n")
            .await
            .expect("write stale memory index");
        let memory_summary_path = root.join("memory_summary.md");
        tokio::fs::write(&memory_summary_path, "stale memory summary\n")
            .await
            .expect("write stale memory summary");
        let stale_skill_file = root.join("skills/demo/SKILL.md");
        tokio::fs::create_dir_all(
            stale_skill_file
                .parent()
                .expect("skills subdirectory parent should exist"),
        )
        .await
        .expect("create stale skills dir");
        tokio::fs::write(&stale_skill_file, "stale skill\n")
            .await
            .expect("write stale skill");

        harness
            .state_db
            .enqueue_global_consolidation(/*input_watermark*/ 999)
            .await
            .expect("enqueue global consolidation");

        phase2::run(&harness.session, Arc::clone(&harness.config)).await;

        assert!(
            !tokio::fs::try_exists(&stale_summary_path)
                .await
                .expect("check stale summary existence"),
            "empty consolidation should prune stale rollout summary files"
        );
        let raw_memories = tokio::fs::read_to_string(&raw_memories_path)
            .await
            .expect("read rebuilt raw memories");
        pretty_assertions::assert_eq!(raw_memories, "# Raw Memories\n\nNo raw memories yet.\n");
        assert!(
            !tokio::fs::try_exists(&memory_index_path)
                .await
                .expect("check memory index existence"),
            "empty consolidation should remove stale MEMORY.md"
        );
        assert!(
            !tokio::fs::try_exists(&memory_summary_path)
                .await
                .expect("check memory summary existence"),
            "empty consolidation should remove stale memory_summary.md"
        );
        assert!(
            !tokio::fs::try_exists(&stale_skill_file)
                .await
                .expect("check stale skill existence"),
            "empty consolidation should remove stale skills artifacts"
        );
        assert!(
            !tokio::fs::try_exists(root.join("skills"))
                .await
                .expect("check skills dir existence"),
            "empty consolidation should remove stale skills directory"
        );
        let next_claim = harness
            .state_db
            .try_claim_global_phase2_job(ThreadId::new(), /*lease_seconds*/ 3_600)
            .await
            .expect("claim global job after empty consolidation success");
        pretty_assertions::assert_eq!(next_claim, Phase2JobClaimOutcome::SkippedNotDirty);
        pretty_assertions::assert_eq!(harness.user_input_ops_count(), 0);
        let thread_ids = harness.manager.list_thread_ids().await;
        pretty_assertions::assert_eq!(thread_ids.len(), 0);

        harness.shutdown_threads().await;
    }

    #[tokio::test]
    async fn dispatch_marks_job_for_retry_when_sandbox_policy_cannot_be_overridden() {
        let harness = DispatchHarness::new().await;
        harness
            .state_db
            .enqueue_global_consolidation(/*input_watermark*/ 99)
            .await
            .expect("enqueue global consolidation");
        let mut constrained_config = harness.config.as_ref().clone();
        constrained_config.permissions.sandbox_policy =
            Constrained::allow_only(SandboxPolicy::DangerFullAccess);

        phase2::run(&harness.session, Arc::new(constrained_config)).await;

        let retry_claim = harness
            .state_db
            .try_claim_global_phase2_job(ThreadId::new(), /*lease_seconds*/ 3_600)
            .await
            .expect("claim global job after sandbox policy failure");
        pretty_assertions::assert_eq!(retry_claim, Phase2JobClaimOutcome::SkippedNotDirty);
        pretty_assertions::assert_eq!(harness.user_input_ops_count(), 0);
        let thread_ids = harness.manager.list_thread_ids().await;
        pretty_assertions::assert_eq!(thread_ids.len(), 0);
    }

    #[tokio::test]
    async fn dispatch_marks_job_for_retry_when_syncing_artifacts_fails() {
        let harness = DispatchHarness::new().await;
        harness.seed_stage1_output(/*source_updated_at*/ 100).await;
        let root = memory_root(&harness.config.codex_home);
        tokio::fs::write(&root, "not a directory")
            .await
            .expect("create file at memory root");

        phase2::run(&harness.session, Arc::clone(&harness.config)).await;

        let retry_claim = harness
            .state_db
            .try_claim_global_phase2_job(ThreadId::new(), /*lease_seconds*/ 3_600)
            .await
            .expect("claim global job after sync failure");
        pretty_assertions::assert_eq!(retry_claim, Phase2JobClaimOutcome::SkippedNotDirty);
        pretty_assertions::assert_eq!(harness.user_input_ops_count(), 0);
        let thread_ids = harness.manager.list_thread_ids().await;
        pretty_assertions::assert_eq!(thread_ids.len(), 0);
    }

    #[tokio::test]
    async fn dispatch_marks_job_for_retry_when_rebuilding_raw_memories_fails() {
        let harness = DispatchHarness::new().await;
        harness.seed_stage1_output(/*source_updated_at*/ 100).await;
        let root = memory_root(&harness.config.codex_home);
        tokio::fs::create_dir_all(raw_memories_file(&root))
            .await
            .expect("create raw_memories.md as a directory");

        phase2::run(&harness.session, Arc::clone(&harness.config)).await;

        let retry_claim = harness
            .state_db
            .try_claim_global_phase2_job(ThreadId::new(), /*lease_seconds*/ 3_600)
            .await
            .expect("claim global job after rebuild failure");
        pretty_assertions::assert_eq!(retry_claim, Phase2JobClaimOutcome::SkippedNotDirty);
        pretty_assertions::assert_eq!(harness.user_input_ops_count(), 0);
        let thread_ids = harness.manager.list_thread_ids().await;
        pretty_assertions::assert_eq!(thread_ids.len(), 0);
    }

    #[tokio::test]
    async fn dispatch_marks_job_for_retry_when_spawn_agent_fails() {
        let codex_home = tempfile::tempdir().expect("create temp codex home");
        let mut config = test_config().await;
        config.codex_home =
            codex_utils_absolute_path::AbsolutePathBuf::from_absolute_path(codex_home.path())
                .expect("codex home is absolute");
        config.cwd = config.codex_home.clone();
        let config = Arc::new(config);

        let state_db = codex_state::StateRuntime::init(
            config.codex_home.to_path_buf(),
            config.model_provider_id.clone(),
        )
        .await
        .expect("initialize state db");

        let (mut session, _turn_context) = make_session_and_context().await;
        session.services.state_db = Some(Arc::clone(&state_db));
        session.services.agent_control = AgentControl::default();
        let session = Arc::new(session);

        let thread_id = ThreadId::new();
        let mut metadata_builder = ThreadMetadataBuilder::new(
            thread_id,
            config
                .codex_home
                .join(format!("rollout-{thread_id}.jsonl"))
                .to_path_buf(),
            Utc::now(),
            SessionSource::Cli,
        );
        metadata_builder.cwd = config.cwd.to_path_buf();
        metadata_builder.model_provider = Some(config.model_provider_id.clone());
        let metadata = metadata_builder.build(&config.model_provider_id);
        state_db
            .upsert_thread(&metadata)
            .await
            .expect("upsert thread metadata");

        let claim = state_db
            .try_claim_stage1_job(
                thread_id,
                session.conversation_id,
                /*source_updated_at*/ 100,
                /*lease_seconds*/ 3_600,
                /*max_running_jobs*/ 64,
            )
            .await
            .expect("claim stage-1 job");
        let ownership_token = match claim {
            codex_state::Stage1JobClaimOutcome::Claimed { ownership_token } => ownership_token,
            other => panic!("unexpected stage-1 claim outcome: {other:?}"),
        };
        assert!(
            state_db
                .mark_stage1_job_succeeded(
                    thread_id,
                    &ownership_token,
                    /*source_updated_at*/ 100,
                    "raw memory",
                    "rollout summary",
                    /*rollout_slug*/ None,
                )
                .await
                .expect("mark stage-1 success"),
            "stage-1 success should enqueue global consolidation"
        );

        let telepathy_resources = config
            .codex_home
            .join("memories_extensions/telepathy/resources");
        tokio::fs::create_dir_all(&telepathy_resources)
            .await
            .expect("create telepathy resources");
        tokio::fs::write(
            config
                .codex_home
                .join("memories_extensions/telepathy/instructions.md"),
            "instructions",
        )
        .await
        .expect("write telepathy instructions");
        let old_file = telepathy_resources.join(format!(
            "{}-abcd-10min-old.md",
            (Utc::now() - ChronoDuration::days(8)).format("%Y-%m-%dT%H-%M-%S")
        ));
        tokio::fs::write(&old_file, "old resource")
            .await
            .expect("write old extension resource");

        phase2::run(&session, Arc::clone(&config)).await;

        let retry_claim = state_db
            .try_claim_global_phase2_job(ThreadId::new(), /*lease_seconds*/ 3_600)
            .await
            .expect("claim global job after spawn failure");
        pretty_assertions::assert_eq!(
            retry_claim,
            Phase2JobClaimOutcome::SkippedNotDirty,
            "spawn failures should leave the job in retry backoff instead of running"
        );
        assert!(
            tokio::fs::try_exists(&old_file)
                .await
                .expect("check old extension resource"),
            "spawn failures should not prune extension resources before retry"
        );
    }
}
