use super::*;
use codex_protocol::ThreadId;
use pretty_assertions::assert_eq;
use std::path::PathBuf;

fn thread_id(value: &str) -> ThreadId {
    ThreadId::from_string(value).expect("valid thread id")
}

fn test_target(
    selected_thread_ids: Vec<ThreadId>,
    removed_thread_ids: Vec<ThreadId>,
) -> ProjectMemoryTarget {
    ProjectMemoryTarget {
        project_root: PathBuf::from("/tmp/repo"),
        memory_dir: PathBuf::from("/tmp/repo/.codex"),
        memory_file: PathBuf::from("/tmp/repo/.codex/MEMORY.md"),
        candidate_file: PathBuf::from("/tmp/codex/memories/project_facts/candidates/repo.json"),
        facts_file: PathBuf::from("/tmp/codex/memories/project_facts/repo.json"),
        selected_thread_ids,
        removed_thread_ids,
    }
}

#[test]
fn merge_project_memory_contents_appends_generated_section_when_markers_are_missing() {
    let merged = merge_project_memory_contents(
        Some("Manual intro\n\nManual footer\n"),
        Some("## Codex Auto Memory\n- Use pnpm.\n"),
    )
    .expect("merged contents");

    assert_eq!(
        merged,
        format!(
            "Manual intro\n\nManual footer\n\n{PROJECT_MEMORY_AUTO_SECTION_BEGIN}\n## Codex Auto Memory\n- Use pnpm.\n{PROJECT_MEMORY_AUTO_SECTION_END}\n"
        )
    );
}

#[test]
fn merge_project_memory_contents_removes_generated_section_when_draft_is_empty() {
    let merged = merge_project_memory_contents(
        Some(&format!(
            "Manual intro\n\n{PROJECT_MEMORY_AUTO_SECTION_BEGIN}\nOld auto memory\n{PROJECT_MEMORY_AUTO_SECTION_END}\n\nManual footer\n"
        )),
        None,
    )
    .expect("merged contents");

    assert_eq!(merged, "Manual intro\n\nManual footer\n");
}

#[test]
fn merge_project_memory_contents_returns_none_when_only_generated_section_remains_empty() {
    let merged = merge_project_memory_contents(
        Some(&format!(
            "{PROJECT_MEMORY_AUTO_SECTION_BEGIN}\nOld auto memory\n{PROJECT_MEMORY_AUTO_SECTION_END}\n"
        )),
        None,
    );

    assert_eq!(merged, None);
}

#[test]
fn accept_project_fact_candidates_filters_invalid_threads_and_normalizes_fields() {
    let selected_thread_id = thread_id("00000000-0000-7000-8000-000000000001");
    let other_thread_id = thread_id("00000000-0000-7000-8000-000000000099");
    let target = test_target(vec![selected_thread_id], vec![]);

    let accepted = accept_project_fact_candidates(
        &target,
        vec![
            ProjectFactCandidate {
                category: " tools ".to_string(),
                fact: " Use   pnpm ".to_string(),
                details: vec![" Prefer  pnpm test ".to_string(), "".to_string()],
                evidence_thread_ids: vec![selected_thread_id, other_thread_id],
            },
            ProjectFactCandidate {
                category: "workflow".to_string(),
                fact: "".to_string(),
                details: vec![],
                evidence_thread_ids: vec![selected_thread_id],
            },
            ProjectFactCandidate {
                category: "gotcha".to_string(),
                fact: "Watch for flaky seed data".to_string(),
                details: vec!["Re-run fixtures before integration tests.".to_string()],
                evidence_thread_ids: vec![selected_thread_id],
            },
            ProjectFactCandidate {
                category: "workflow".to_string(),
                fact: "Ignore me".to_string(),
                details: vec![],
                evidence_thread_ids: vec![other_thread_id],
            },
        ],
    );

    assert_eq!(
        accepted,
        vec![
            ProjectFact {
                category: "tooling".to_string(),
                fact: "Use pnpm".to_string(),
                details: vec!["Prefer pnpm test".to_string()],
                evidence_thread_ids: vec![selected_thread_id],
            },
            ProjectFact {
                category: "pitfalls".to_string(),
                fact: "Watch for flaky seed data".to_string(),
                details: vec!["Re-run fixtures before integration tests.".to_string()],
                evidence_thread_ids: vec![selected_thread_id],
            },
        ]
    );
}

#[test]
fn merge_project_facts_forgets_removed_threads_and_merges_new_evidence() {
    let kept_thread_id = thread_id("00000000-0000-7000-8000-000000000010");
    let removed_thread_id = thread_id("00000000-0000-7000-8000-000000000090");
    let new_thread_id = thread_id("00000000-0000-7000-8000-000000000011");
    let target = test_target(vec![new_thread_id], vec![removed_thread_id]);

    let existing_store = Some(ProjectFactsFile {
        schema_version: 1,
        project_root: target.project_root.clone(),
        facts: vec![
            ProjectFact {
                category: "tooling".to_string(),
                fact: "Use pnpm".to_string(),
                details: vec!["Prefer pnpm test".to_string()],
                evidence_thread_ids: vec![kept_thread_id, removed_thread_id],
            },
            ProjectFact {
                category: "pitfalls".to_string(),
                fact: "Legacy env var is stale".to_string(),
                details: vec!["Remove OLD_FLAG from local shells.".to_string()],
                evidence_thread_ids: vec![removed_thread_id],
            },
        ],
    });

    let merged = merge_project_facts(
        &target,
        existing_store,
        vec![
            ProjectFact {
                category: "tooling".to_string(),
                fact: "Use pnpm".to_string(),
                details: vec!["Prefer pnpm --filter for targeted commands.".to_string()],
                evidence_thread_ids: vec![new_thread_id],
            },
            ProjectFact {
                category: "commands".to_string(),
                fact: "Run cargo test -p codex-core for memory changes".to_string(),
                details: vec![],
                evidence_thread_ids: vec![new_thread_id],
            },
        ],
    )
    .expect("merged store");

    assert_eq!(
        merged.facts,
        vec![
            ProjectFact {
                category: "tooling".to_string(),
                fact: "Use pnpm".to_string(),
                details: vec!["Prefer pnpm --filter for targeted commands.".to_string()],
                evidence_thread_ids: vec![kept_thread_id, new_thread_id],
            },
            ProjectFact {
                category: "commands".to_string(),
                fact: "Run cargo test -p codex-core for memory changes".to_string(),
                details: vec![],
                evidence_thread_ids: vec![new_thread_id],
            },
        ]
    );
}

#[test]
fn render_project_memory_body_from_facts_groups_categories_deterministically() {
    let rendered = render_project_memory_body_from_facts(&sort_project_facts(vec![
        ProjectFact {
            category: "commands".to_string(),
            fact: "Run cargo test -p codex-core".to_string(),
            details: vec!["Use targeted test names first.".to_string()],
            evidence_thread_ids: vec![thread_id("00000000-0000-7000-8000-000000000021")],
        },
        ProjectFact {
            category: "tooling".to_string(),
            fact: "Package manager is pnpm".to_string(),
            details: vec![],
            evidence_thread_ids: vec![thread_id("00000000-0000-7000-8000-000000000022")],
        },
        ProjectFact {
            category: "architecture".to_string(),
            fact: "Memory pipeline lives under codex-rs/core/src/memories".to_string(),
            details: vec!["Phase 2 owns accepted project facts.".to_string()],
            evidence_thread_ids: vec![thread_id("00000000-0000-7000-8000-000000000023")],
        },
    ]))
    .expect("rendered body");

    assert_eq!(
        rendered,
        "## Codex Auto Memory\n\n### Architecture\n- Memory pipeline lives under codex-rs/core/src/memories\n  - Phase 2 owns accepted project facts.\n\n### Tooling\n- Package manager is pnpm\n\n### Commands\n- Run cargo test -p codex-core\n  - Use targeted test names first.\n"
    );
}
