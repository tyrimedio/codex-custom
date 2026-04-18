use crate::codex::Session;
use crate::config::Config;
use crate::memories::phase1;
use crate::memories::phase2;
use codex_features::Feature;
use codex_protocol::protocol::SessionSource;
use std::sync::Arc;
use tracing::warn;

fn should_run_memories_pipeline(
    session: &Arc<Session>,
    config: &Config,
    source: &SessionSource,
) -> bool {
    if config.ephemeral
        || !config.features.enabled(Feature::MemoryTool)
        || matches!(source, SessionSource::SubAgent(_))
    {
        return false;
    }

    if session.services.state_db.is_none() {
        warn!("state db unavailable for memories pipeline; skipping");
        return false;
    }

    true
}

/// Starts the asynchronous startup memory pipeline for an eligible root
/// session.
pub(crate) fn start_memories_startup_task(
    session: &Arc<Session>,
    config: Arc<Config>,
    source: &SessionSource,
) {
    if !should_run_memories_pipeline(session, config.as_ref(), source) {
        return;
    }

    let weak_session = Arc::downgrade(session);
    tokio::spawn(async move {
        let Some(session) = weak_session.upgrade() else {
            return;
        };

        // Clean memories to make preserve DB size
        phase1::prune(&session, &config).await;
        // Run phase 1.
        phase1::run(&session, &config).await;
        // Run phase 2.
        phase2::run(&session, config).await;
    });
}

/// Starts the asynchronous current-thread memory pipeline immediately after a
/// completed root turn.
pub(crate) fn start_memories_current_thread_task(
    session: &Arc<Session>,
    config: Arc<Config>,
    source: &SessionSource,
    cutoff_turn_id: Option<String>,
) {
    if !should_run_memories_pipeline(session, config.as_ref(), source) {
        return;
    }

    let weak_session = Arc::downgrade(session);
    tokio::spawn(async move {
        let Some(session) = weak_session.upgrade() else {
            return;
        };

        phase1::run_for_current_thread(&session, &config, cutoff_turn_id.as_deref()).await;
        phase2::run(&session, config).await;
    });
}
