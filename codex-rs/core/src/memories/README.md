# Memories Pipeline (Core)

This module runs two memory pipelines for eligible sessions:

- a startup pipeline for catch-up / background consolidation across prior threads
- an immediate current-thread pipeline after completed root turns

## Prompt Templates

Memory prompt templates live under `codex-rs/core/templates/memories/`.

- The undated template files are the canonical latest versions used at runtime:
  - `stage_one_system.md`
  - `stage_one_input.md`
  - `consolidation.md`
  - `read_path.md`
- In `codex`, edit those undated template files in place.
- The dated snapshot-copy workflow is used in the separate `openai/project/agent_memory/write` harness repo, not here.

## When it runs

Both pipelines are root-session only and run only if:

- the session is not ephemeral
- the memory feature is enabled
- the session is not a sub-agent session
- the state DB is available

Trigger points:

- session startup:
  - prune old stage-1 outputs
  - run Phase 1 startup selection
  - run Phase 2 consolidation
- completed root turn:
  - run Phase 1 immediately for the current thread
  - run Phase 2 consolidation

Both paths run asynchronously in the background and execute Phase 1 before Phase 2.

## Phase 1: Rollout Extraction (per-thread)

Phase 1 finds recent eligible rollouts and extracts a structured memory from each one.

Startup selection:

Eligible rollouts are selected from the state DB using startup claim rules. In practice this means
the startup path only considers rollouts that are:

- from allowed interactive session sources
- within the configured age window
- idle long enough (to avoid summarizing still-active/fresh rollouts)
- not already owned by another in-flight phase-1 worker
- within startup scan/claim limits (bounded work per startup)

What it does:

- on startup, claims a bounded set of rollout jobs from the state DB
- after a completed root turn, claims only the current thread immediately
- filters rollout content down to memory-relevant response items
- for immediate post-turn updates, truncates the rollout at the just-finished
  `TurnComplete` marker so extraction does not race with the next turn
- sends each rollout to a model (in parallel, with a concurrency cap)
- expects structured output containing:
  - a detailed `raw_memory`
  - a compact `rollout_summary`
  - an optional `rollout_slug`
- redacts secrets from the generated memory fields
- stores successful outputs back into the state DB as stage-1 outputs

Concurrency / coordination:

- Phase 1 runs multiple extraction jobs in parallel (with a fixed concurrency cap) so startup memory generation can process several rollouts at once.
- Each job is leased/claimed in the state DB before processing, which prevents duplicate work across concurrent workers/startups.
- Failed jobs are marked with retry backoff, so they are retried later instead of hot-looping.

Job outcomes:

- `succeeded` (memory produced)
- `succeeded_no_output` (valid run but nothing useful generated)
- `failed` (with retry backoff/lease handling in DB)

Phase 1 is the stage that turns individual rollouts into DB-backed memory records.

## Phase 2: Global Consolidation

Phase 2 consolidates the latest stage-1 outputs into the filesystem memory artifacts and then runs a dedicated consolidation agent.

What it does:

- claims a single global phase-2 job (so only one consolidation runs at a time)
- loads a bounded set of stage-1 outputs from the state DB using phase-2
  selection rules:
  - ignores memories whose `last_usage` falls outside the configured
    `max_unused_days` window
  - for memories with no `last_usage`, falls back to `generated_at` so fresh
    never-used memories can still be selected
  - ranks eligible memories by `usage_count` first, then by the most recent
    `last_usage` / `generated_at`
- computes a completion watermark from the claimed watermark + newest input timestamps
- syncs local memory artifacts under the memories root:
  - `raw_memories.md` (merged raw memories, latest first)
  - `rollout_summaries/` (one summary file per retained rollout)
- prepares repo-local structured memory artifacts under:
  - `project_facts/<project-slug>-<hash>.json` (accepted durable facts store)
  - `project_facts/candidates/<project-slug>-<hash>.json` (phase-2 candidate facts input)
- prunes stale rollout summaries that are no longer retained
- finds old resource files from memory extensions under
  `memories_extensions/<extension>/resources/` for extension directories that
  have an `instructions.md`, using the memory module retention window
- if there are no Phase 1 inputs or old extension resources, marks the job
  successful and exits

If there is input, it then:

- spawns an internal consolidation sub-agent
- builds the Phase 2 prompt with a diff of the current Phase 1 input
  selection versus the last successful Phase 2 selection (`added`,
  `retained`, `removed`)
- includes old extension resource paths in the prompt diff
- when repo-local memory targets exist, instructs the consolidation agent to write
  JSON fact candidates for each repo instead of editing repo files directly
- runs it with no approvals, no network, and local write access only
- disables collab for that agent (to prevent recursive delegation)
- watches the agent status and heartbeats the global job lease while it runs
- after a successful consolidation agent run:
  - validates and normalizes repo-local fact candidates in Rust
  - merges them into the per-project accepted-facts stores under
    `memories/project_facts/`
  - removes facts supported only by removed thread ids
  - renders repo-local `.codex/MEMORY.md` generated sections from accepted facts only
- marks the phase-2 job success/failure in the state DB when the agent finishes
- prunes old extension resource files after the consolidation agent completes
  and the successful Phase 2 job is recorded

Selection diff behavior:

- successful Phase 2 runs mark the exact stage-1 snapshots they consumed with
  `selected_for_phase2 = 1` and persist the matching
  `selected_for_phase2_source_updated_at`
- Phase 1 upserts preserve the previous `selected_for_phase2` baseline until
  the next successful Phase 2 run rewrites it
- the next Phase 2 run compares the current top-N stage-1 inputs against that
  prior snapshot selection to label inputs as `added` or `retained`; a
  refreshed thread stays `added` until Phase 2 successfully selects its newer
  snapshot
- rows that were previously selected but still exist outside the current top-N
  selection are surfaced as `removed`
- before the agent starts, local `rollout_summaries/` and `raw_memories.md`
  keep the union of the current selection and the previous successful
  selection, so removed-thread evidence stays available during forgetting

Watermark behavior:

- The global phase-2 job claim includes an input watermark representing the latest input timestamp known when the job was claimed.
- Phase 2 recomputes a `new_watermark` using the max of:
  - the claimed watermark
  - the newest `source_updated_at` timestamp in the stage-1 inputs it actually loaded
- On success, Phase 2 stores that completion watermark in the DB.
- This lets later phase-2 runs know whether new stage-1 data arrived since the last successful consolidation (dirty vs not dirty), while also avoiding moving the watermark backwards.

In practice, this phase is responsible for refreshing the on-disk memory workspace and producing/updating the higher-level consolidated memory outputs.

## Why it is split into two phases

- Phase 1 scales across many rollouts and produces normalized per-rollout memory records.
- Phase 2 serializes global consolidation so the shared memory artifacts are updated safely and consistently.
