use crate::memories::PROJECT_MEMORY_AUTO_SECTION_BEGIN;
use crate::memories::PROJECT_MEMORY_AUTO_SECTION_END;
use crate::memories::ProjectMemoryTarget;
use codex_protocol::ThreadId;
use serde::Deserialize;
use serde::Serialize;
use std::collections::BTreeMap;
use std::collections::HashSet;
use std::io;
use std::path::Path;
use std::path::PathBuf;

const PROJECT_FACTS_SUBDIR: &str = "project_facts";
const PROJECT_FACT_CANDIDATES_SUBDIR: &str = "candidates";
const PROJECT_FACTS_SCHEMA_VERSION: u32 = 1;
const PROJECT_MEMORY_HEADING: &str = "## Codex Auto Memory";
const PROJECT_FACT_CATEGORY_ORDER: &[(&str, &str)] = &[
    ("architecture", "Architecture"),
    ("tooling", "Tooling"),
    ("workflow", "Workflow"),
    ("commands", "Commands"),
    ("testing", "Testing"),
    ("conventions", "Conventions"),
    ("pitfalls", "Pitfalls"),
    ("preferences", "Preferences"),
    ("general", "General"),
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ProjectFactCandidateFile {
    #[serde(default = "project_facts_schema_version")]
    pub(super) schema_version: u32,
    #[serde(default)]
    pub(super) facts: Vec<ProjectFactCandidate>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ProjectFactCandidate {
    pub(super) category: String,
    pub(super) fact: String,
    #[serde(default)]
    pub(super) details: Vec<String>,
    #[serde(default)]
    pub(super) evidence_thread_ids: Vec<ThreadId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ProjectFactsFile {
    #[serde(default = "project_facts_schema_version")]
    pub(super) schema_version: u32,
    pub(super) project_root: PathBuf,
    #[serde(default)]
    pub(super) facts: Vec<ProjectFact>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ProjectFact {
    pub(super) category: String,
    pub(super) fact: String,
    #[serde(default)]
    pub(super) details: Vec<String>,
    #[serde(default)]
    pub(super) evidence_thread_ids: Vec<ThreadId>,
}

fn project_facts_schema_version() -> u32 {
    PROJECT_FACTS_SCHEMA_VERSION
}

pub(super) fn project_facts_dir(memory_root: &Path) -> PathBuf {
    memory_root.join(PROJECT_FACTS_SUBDIR)
}

fn project_fact_candidates_dir(memory_root: &Path) -> PathBuf {
    project_facts_dir(memory_root).join(PROJECT_FACT_CANDIDATES_SUBDIR)
}

pub(super) fn project_facts_store_file(memory_root: &Path, project_root: &Path) -> PathBuf {
    let display = project_root.display().to_string();
    let slug = sanitize_project_root_label(
        project_root
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("project"),
    );
    let hash = stable_hash_hex(&display);
    project_facts_dir(memory_root).join(format!("{slug}-{hash}.json"))
}

pub(super) fn project_fact_candidate_file(memory_root: &Path, project_root: &Path) -> PathBuf {
    let display = project_root.display().to_string();
    let slug = sanitize_project_root_label(
        project_root
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("project"),
    );
    let hash = stable_hash_hex(&display);
    project_fact_candidates_dir(memory_root).join(format!("{slug}-{hash}.json"))
}

pub(super) async fn prepare_project_memory_candidates(
    targets: &[ProjectMemoryTarget],
) -> io::Result<()> {
    let mut prepared_parents = HashSet::new();

    for target in targets {
        let Some(parent) = target.candidate_file.parent() else {
            return Err(io::Error::other(format!(
                "missing candidate parent for {}",
                target.candidate_file.display()
            )));
        };
        if prepared_parents.insert(parent.to_path_buf()) {
            tokio::fs::create_dir_all(parent).await?;
        }
        remove_if_exists(&target.candidate_file).await?;
    }

    Ok(())
}

pub(super) async fn apply_project_memory_updates(
    targets: &[ProjectMemoryTarget],
) -> io::Result<()> {
    for target in targets {
        let candidates = read_project_fact_candidate_file(&target.candidate_file).await?;
        let accepted_facts = accept_project_fact_candidates(target, candidates.facts);
        let existing_store = read_project_facts_store(&target.facts_file).await?;
        let merged_store = merge_project_facts(target, existing_store, accepted_facts);

        match merged_store.as_ref() {
            Some(store) => {
                let parent = target
                    .facts_file
                    .parent()
                    .ok_or_else(|| io::Error::other("missing project facts store parent"))?;
                tokio::fs::create_dir_all(parent).await?;
                let serialized = serde_json::to_string_pretty(store).map_err(io::Error::other)?;
                tokio::fs::write(&target.facts_file, format!("{serialized}\n")).await?;
            }
            None => {
                remove_if_exists(&target.facts_file).await?;
            }
        }

        let generated_body =
            merged_store.and_then(|store| render_project_memory_body_from_facts(&store.facts));
        let existing = tokio::fs::read_to_string(&target.memory_file).await.ok();

        match merge_project_memory_contents(existing.as_deref(), generated_body.as_deref()) {
            Some(contents) => {
                tokio::fs::create_dir_all(&target.memory_dir).await?;
                tokio::fs::write(&target.memory_file, contents).await?;
            }
            None => {
                remove_if_exists(&target.memory_file).await?;
            }
        }

        remove_if_exists(&target.candidate_file).await?;
    }

    Ok(())
}

async fn read_project_fact_candidate_file(path: &Path) -> io::Result<ProjectFactCandidateFile> {
    let contents = tokio::fs::read_to_string(path)
        .await
        .map_err(|err| match err.kind() {
            io::ErrorKind::NotFound => io::Error::other(format!(
                "missing project memory candidate file {}",
                path.display()
            )),
            _ => err,
        })?;
    let file: ProjectFactCandidateFile =
        serde_json::from_str(&contents).map_err(io::Error::other)?;
    if file.schema_version != PROJECT_FACTS_SCHEMA_VERSION {
        return Err(io::Error::other(format!(
            "unsupported project fact candidate schema_version {} for {}",
            file.schema_version,
            path.display()
        )));
    }
    Ok(file)
}

async fn read_project_facts_store(path: &Path) -> io::Result<Option<ProjectFactsFile>> {
    let contents = match tokio::fs::read_to_string(path).await {
        Ok(contents) => contents,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err),
    };
    let file: ProjectFactsFile = serde_json::from_str(&contents).map_err(io::Error::other)?;
    if file.schema_version != PROJECT_FACTS_SCHEMA_VERSION {
        return Err(io::Error::other(format!(
            "unsupported project facts schema_version {} for {}",
            file.schema_version,
            path.display()
        )));
    }
    Ok(Some(file))
}

fn accept_project_fact_candidates(
    target: &ProjectMemoryTarget,
    candidates: Vec<ProjectFactCandidate>,
) -> Vec<ProjectFact> {
    let allowed_thread_ids = target
        .selected_thread_ids
        .iter()
        .copied()
        .collect::<HashSet<_>>();
    let mut accepted = BTreeMap::new();

    for candidate in candidates {
        let fact = normalize_text(&candidate.fact);
        if fact.is_empty() {
            continue;
        }

        let evidence_thread_ids = candidate
            .evidence_thread_ids
            .into_iter()
            .filter(|thread_id| allowed_thread_ids.contains(thread_id))
            .collect::<Vec<_>>();
        let evidence_thread_ids = sort_and_dedup_thread_ids(evidence_thread_ids);
        if evidence_thread_ids.is_empty() {
            continue;
        }

        let category = normalize_category(&candidate.category);
        let details = normalize_details(candidate.details);
        let key = project_fact_key(&fact);

        accepted
            .entry(key)
            .and_modify(|existing: &mut ProjectFact| {
                existing.category = category.clone();
                existing.fact = fact.clone();
                if !details.is_empty() {
                    existing.details = details.clone();
                }
                existing
                    .evidence_thread_ids
                    .extend(evidence_thread_ids.clone());
                existing.evidence_thread_ids =
                    sort_and_dedup_thread_ids(existing.evidence_thread_ids.clone());
            })
            .or_insert_with(|| ProjectFact {
                category,
                fact,
                details,
                evidence_thread_ids,
            });
    }

    sort_project_facts(accepted.into_values().collect())
}

fn split_project_memory_contents(existing: &str) -> Option<(&str, &str)> {
    let begin = existing.find(PROJECT_MEMORY_AUTO_SECTION_BEGIN)?;
    let end = existing[begin + PROJECT_MEMORY_AUTO_SECTION_BEGIN.len()..]
        .find(PROJECT_MEMORY_AUTO_SECTION_END)
        .map(|offset| begin + PROJECT_MEMORY_AUTO_SECTION_BEGIN.len() + offset)?;

    Some((
        &existing[..begin],
        &existing[end + PROJECT_MEMORY_AUTO_SECTION_END.len()..],
    ))
}

fn merge_project_facts(
    target: &ProjectMemoryTarget,
    existing_store: Option<ProjectFactsFile>,
    accepted_facts: Vec<ProjectFact>,
) -> Option<ProjectFactsFile> {
    let removed_thread_ids = target
        .removed_thread_ids
        .iter()
        .copied()
        .collect::<HashSet<_>>();
    let mut merged = existing_store
        .map(|store| {
            store
                .facts
                .into_iter()
                .filter_map(|mut fact| {
                    fact.evidence_thread_ids
                        .retain(|thread_id| !removed_thread_ids.contains(thread_id));
                    fact.evidence_thread_ids =
                        sort_and_dedup_thread_ids(fact.evidence_thread_ids.clone());
                    (!fact.evidence_thread_ids.is_empty())
                        .then_some((project_fact_key(&fact.fact), fact))
                })
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();

    for accepted_fact in accepted_facts {
        let key = project_fact_key(&accepted_fact.fact);
        merged
            .entry(key)
            .and_modify(|existing| {
                let details = if accepted_fact.details.is_empty() {
                    existing.details.clone()
                } else {
                    accepted_fact.details.clone()
                };
                existing.category = accepted_fact.category.clone();
                existing.fact = accepted_fact.fact.clone();
                existing.details = details;
                existing
                    .evidence_thread_ids
                    .extend(accepted_fact.evidence_thread_ids.clone());
                existing.evidence_thread_ids =
                    sort_and_dedup_thread_ids(existing.evidence_thread_ids.clone());
            })
            .or_insert(accepted_fact);
    }

    let facts = sort_project_facts(merged.into_values().collect());
    if facts.is_empty() {
        None
    } else {
        Some(ProjectFactsFile {
            schema_version: PROJECT_FACTS_SCHEMA_VERSION,
            project_root: target.project_root.clone(),
            facts,
        })
    }
}

fn render_project_memory_body_from_facts(facts: &[ProjectFact]) -> Option<String> {
    if facts.is_empty() {
        return None;
    }

    let mut rendered = String::from(PROJECT_MEMORY_HEADING);
    let categories = ordered_categories_for_facts(facts);

    for category in categories {
        rendered.push_str("\n\n### ");
        rendered.push_str(display_category(&category));
        for fact in facts.iter().filter(|fact| fact.category == category) {
            rendered.push_str("\n- ");
            rendered.push_str(&fact.fact);
            for detail in &fact.details {
                rendered.push_str("\n  - ");
                rendered.push_str(detail);
            }
        }
    }

    rendered.push('\n');
    Some(rendered)
}

fn ordered_categories_for_facts(facts: &[ProjectFact]) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut ordered = Vec::new();

    for (category, _) in PROJECT_FACT_CATEGORY_ORDER {
        if facts.iter().any(|fact| fact.category == *category) && seen.insert(*category) {
            ordered.push((*category).to_string());
        }
    }

    let mut extras = facts
        .iter()
        .map(|fact| fact.category.clone())
        .filter(|category| !seen.contains(category.as_str()))
        .collect::<Vec<_>>();
    extras.sort();
    extras.dedup();
    ordered.extend(extras);
    ordered
}

fn display_category(category: &str) -> &str {
    PROJECT_FACT_CATEGORY_ORDER
        .iter()
        .find_map(|(slug, display)| (*slug == category).then_some(*display))
        .unwrap_or("General")
}

fn sort_project_facts(mut facts: Vec<ProjectFact>) -> Vec<ProjectFact> {
    facts.sort_by(|left, right| {
        project_fact_category_rank(&left.category)
            .cmp(&project_fact_category_rank(&right.category))
            .then_with(|| left.fact.cmp(&right.fact))
    });
    facts
}

fn project_fact_category_rank(category: &str) -> usize {
    PROJECT_FACT_CATEGORY_ORDER
        .iter()
        .position(|(slug, _)| *slug == category)
        .unwrap_or(PROJECT_FACT_CATEGORY_ORDER.len())
}

fn normalize_category(category: &str) -> String {
    let normalized = normalize_text(category).to_ascii_lowercase();
    match normalized.as_str() {
        "architecture" | "structure" | "layout" => "architecture",
        "tooling" | "tool" | "tools" => "tooling",
        "workflow" | "workflows" | "process" | "processes" => "workflow",
        "command" | "commands" => "commands",
        "testing" | "test" | "tests" => "testing",
        "convention" | "conventions" | "style" | "styles" => "conventions",
        "pitfall" | "pitfalls" | "gotcha" | "gotchas" | "failure" | "failures" => "pitfalls",
        "preference" | "preferences" => "preferences",
        "general" => "general",
        _ => "general",
    }
    .to_string()
}

fn normalize_text(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn normalize_details(details: Vec<String>) -> Vec<String> {
    let mut normalized = details
        .into_iter()
        .map(|detail| normalize_text(&detail))
        .filter(|detail| !detail.is_empty())
        .collect::<Vec<_>>();
    normalized.sort();
    normalized.dedup();
    normalized
}

fn sort_and_dedup_thread_ids(mut thread_ids: Vec<ThreadId>) -> Vec<ThreadId> {
    let mut seen = HashSet::new();
    thread_ids.retain(|thread_id| seen.insert(*thread_id));
    thread_ids.sort_unstable_by_key(|thread_id| thread_id.to_string());
    thread_ids
}

fn project_fact_key(fact: &str) -> String {
    normalize_text(fact).to_ascii_lowercase()
}

fn render_project_memory_auto_section(generated_body: Option<&str>) -> Option<String> {
    let generated_body = generated_body.unwrap_or_default().trim();
    if generated_body.is_empty() {
        return None;
    }

    Some(format!(
        "{PROJECT_MEMORY_AUTO_SECTION_BEGIN}\n{generated_body}\n{PROJECT_MEMORY_AUTO_SECTION_END}"
    ))
}

pub(super) fn merge_project_memory_contents(
    existing: Option<&str>,
    generated_body: Option<&str>,
) -> Option<String> {
    let rendered_section = render_project_memory_auto_section(generated_body);

    let (prefix, suffix) = match existing {
        Some(existing) => split_project_memory_contents(existing).unwrap_or((existing, "")),
        None => ("", ""),
    };

    let mut parts = Vec::new();
    let prefix = prefix.trim();
    let suffix = suffix.trim();
    if !prefix.is_empty() {
        parts.push(prefix.to_string());
    }
    if let Some(rendered_section) = rendered_section {
        parts.push(rendered_section);
    }
    if !suffix.is_empty() {
        parts.push(suffix.to_string());
    }

    if parts.is_empty() {
        None
    } else {
        Some(format!("{}\n", parts.join("\n\n")))
    }
}

async fn remove_if_exists(path: &Path) -> io::Result<()> {
    match tokio::fs::remove_file(path).await {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err),
    }
}

fn sanitize_project_root_label(label: &str) -> String {
    let mut normalized = String::with_capacity(label.len());
    for character in label.chars() {
        if character.is_ascii_alphanumeric() {
            normalized.push(character.to_ascii_lowercase());
        } else if !normalized.ends_with('-') {
            normalized.push('-');
        }
    }

    let normalized = normalized.trim_matches('-');
    if normalized.is_empty() {
        "project".to_string()
    } else {
        normalized.to_string()
    }
}

fn stable_hash_hex(input: &str) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in input.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

#[cfg(test)]
#[path = "project_memory_tests.rs"]
mod tests;
