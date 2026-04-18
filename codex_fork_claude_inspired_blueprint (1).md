# Claude-Inspired Improvements for a Codex Fork

This document is a practical blueprint for making a Codex fork feel more like a powerful daily-driver coding harness. It combines the ideas we discussed earlier with the main architectural patterns highlighted in the Claude Code architecture article and current Claude Code and Codex documentation.

The goal is not to copy Claude Code feature-for-feature. The goal is to graft the best ideas onto Codex in a way that fits Codex's current architecture, config system, hook model, skills layer, approvals, sandboxing, and app-server.

---

## Executive summary

If I were prioritizing a Codex fork, I would build around these themes:

1. **A first-class memory system**
   - Add project, user, path-scoped, and auto memory.
   - Keep `AGENTS.md` for intentional guidance.
   - Add `.codex/MEMORY.md` plus per-project auto memory for learned patterns.

2. **A richer lifecycle hook system**
   - Expand Codex hooks beyond the current narrower surface.
   - Use hooks for deterministic automation, context injection, enforcement, and memory maintenance.

3. **A stronger core agent loop**
   - Move toward a single canonical turn/query loop that is re-entrant, stream-aware, and easier to instrument.
   - Make tool execution overlap with model streaming when safe.

4. **Better subagent and task orchestration**
   - Treat subagents and background work as first-class tasks with clear lifecycle, permissions, and memory boundaries.

5. **Better state separation**
   - Separate infrastructure/session state from highly reactive UI state.
   - Make app-server, CLI, and UI all consume the same turn/task primitives.

6. **A more cohesive extension system**
   - Unify hooks, skills, plugins, MCP tools, and app-server mentions under one extension model.

7. **Performance engineering as a product feature**
   - Prompt caching, compaction, speculative execution, stream overlap, cheaper subagent models, and latency-aware tooling.

8. **Stronger permission ergonomics**
   - Keep Codex sandboxing and approvals, but add more intent-aware approvals and subagent permission modes.

That combination would move a Codex fork from "good local agent" toward "agent operating system for coding workflows."

---

## Part 1. Memory system to add to Codex

### Why this matters

Claude Code treats memory as a persistent context layer that spans sessions. The official docs describe `CLAUDE.md` files as persistent instructions loaded at the start of every session, with more specific locations taking precedence over broader ones. The docs also recommend putting durable facts there, such as build commands, project conventions, and recurring corrections. The architecture article describes a dedicated memory subsystem and says Claude selects relevant memories at session start. Codex already has layered `AGENTS.md` discovery and config layering, which gives us a solid base to extend rather than replace.

### Design goal

Build a memory system with two kinds of persistence:

- **Instruction memory**: things humans intentionally want Codex to remember
- **Auto memory**: concise learned patterns extracted from repeated corrections and successful workflows

### Proposed file layout

```text
~/.codex/
  config.toml
  AGENTS.md
  MEMORY.md
  hooks/
  skills/
  projects/
    <project-hash>/
      memory/
        auto.md
        journal.jsonl

<repo>/
  AGENTS.md
  .codex/
    MEMORY.md
    rules/
      frontend.md
      backend.md
      tests.md
    memory/
      local-notes.md
```

### Recommended semantics

- `AGENTS.md`
  - Stable, intentional instructions
  - Repo setup, required commands, coding norms, file conventions
- `.codex/MEMORY.md`
  - Durable project knowledge that should survive sessions
  - Architecture notes, historical gotchas, recurring debugging patterns
- `.codex/rules/*.md`
  - Path-scoped or domain-scoped memory
  - Example: `frontend.md` only applies inside `web/`
- `~/.codex/MEMORY.md`
  - User-wide preferences and stable workflow habits
- `projects/<project-hash>/memory/auto.md`
  - Machine-maintained learned patterns
  - Must stay concise and auditable

### Load order

A good precedence order would be:

1. built-in system instructions
2. user config
3. user `AGENTS.md`
4. user `MEMORY.md`
5. project config
6. repo `AGENTS.md`
7. project `.codex/MEMORY.md`
8. path-scoped rules that match current work
9. project auto memory
10. session or CLI overrides

This keeps intentional human rules ahead of inferred memory and keeps local context ahead of broad defaults.

### What should go into memory

Good memory candidates:

- recurring build and test commands
- non-obvious setup requirements
- codebase conventions
- recurring bugfix patterns
- generated-file workflows
- user style preferences that matter repeatedly
- architecture constraints that affect many tasks

Bad memory candidates:

- secrets
- one-off branch names
- ephemeral tickets
- raw transcripts
- detailed chain-of-thought or long chat summaries
- anything likely to be stale in a few days unless clearly marked temporary

### Auto memory pipeline

Do not dump full transcripts into memory. Instead:

1. collect candidate events at the end of a turn, task, or session
2. score them for durability and future usefulness
3. dedupe against existing memory
4. compact them into short bullet entries
5. write only high-confidence items

Suggested candidate triggers:

- explicit user correction repeated more than once
- repeated successful command sequence
- repeated lint/test failure with same fix
- explicit instruction like "always do X in this repo"
- recurring file pattern or directory-specific convention

### Memory maintenance hooks

Use hooks to keep memory quality high:

- `SessionStart`: load the most relevant memory into developer context
- `PostToolUse`: collect possible memory candidates after successful tests/builds
- `TaskCompleted`: summarize useful subagent discoveries into candidates
- `SessionEnd`: run compaction and dedupe
- `MemoryUpdated`: lint, normalize, and warn about low-value additions

### Minimal config sketch

```toml
[memory]
enabled = true
project_memory_file = ".codex/MEMORY.md"
user_memory_file = "~/.codex/MEMORY.md"
auto_memory_enabled = true
auto_memory_dir = "~/.codex/projects/{project_hash}/memory"
auto_memory_max_bytes = 32768

[memory.extraction]
enabled = true
min_confidence = 0.80
dedupe = true
require_confirmation = false
```

---

## Part 2. Hook system to add or expand in Codex

### Why this matters

Codex already has hooks, including `SessionStart`, `PreToolUse`, and others, but the current docs note that `PreToolUse` is still a work in progress and currently only supports Bash interception. Claude Code's hook system is much broader and can add context, block actions, and influence multiple lifecycle phases. That broader lifecycle is worth borrowing.

### Design goal

Turn hooks into a unified automation and policy system with a broader event surface.

### Recommended event model

#### Session lifecycle
- `SessionStart`
- `SessionResume`
- `SessionEnd`
- `SessionInterrupted`

#### Turn lifecycle
- `UserPromptSubmit`
- `TurnStart`
- `TurnComplete`
- `TurnAbort`
- `TurnError`

#### Tool lifecycle
- `PreToolUse`
- `PostToolUse`
- `ToolError`
- `PermissionRequest`
- `PermissionDenied`
- `ApprovalGranted`

#### Task and subagent lifecycle
- `TaskCreated`
- `TaskStarted`
- `TaskCompleted`
- `TaskFailed`
- `SubagentStart`
- `SubagentComplete`
- `SubagentEscalation`

#### State and file lifecycle
- `FileChanged`
- `CwdChanged`
- `ConfigChanged`
- `MemoryUpdated`
- `SkillChanged`

#### Context lifecycle
- `CompactionStart`
- `CompactionComplete`
- `ContextTruncated`
- `PromptCacheHit`
- `PromptCacheMiss`

### What hooks should do

Hooks should be able to:

- inject developer context
- deny or allow actions
- attach rationale to approval prompts
- run deterministic validators
- suggest or write memory candidates
- trigger external systems like notifications or webhooks
- log telemetry for profiling and debugging

### High-value hook patterns

#### 1. Project bootstrapping
On `SessionStart`, inject:
- project memory summary
- recent failing tests
- current branch and worktree status
- repo-specific setup reminders

#### 2. Safe editing pipeline
On `PostToolUse` for file edits:
- run formatter
- run lint on touched files
- run focused tests if applicable
- suggest memory candidate if failure/fix pattern repeats

#### 3. Dangerous action gating
On `PreToolUse`:
- block edits to secrets or protected paths
- block `git push` on protected branches unless explicitly approved
- require explanation metadata for destructive commands

#### 4. Better approvals
On `PermissionRequest`:
- attach why the action is needed
- show which user request or subagent recommendation led to it
- show risk level and expected side effects

#### 5. Memory upkeep
On `SessionEnd` or `TaskCompleted`:
- extract durable lessons
- compact them
- propose or apply memory updates

### Hook execution constraints

Hooks must respect sandbox and approval boundaries. They should not become a backdoor that bypasses Codex safety. For example:

- project hooks only run in trusted projects
- networked hooks obey network policy
- file-writing hooks obey writable roots
- hooks can suggest approval escalation but not silently override it

### Minimal config sketch

```toml
[hooks]
enabled = true
allow_networked_hooks = false
allow_project_hooks_in_untrusted_repos = false

[hooks.events]
session = true
turn = true
tool = true
task = true
memory = true
context = true
```

---

## Part 3. A stronger query loop and turn model

### Why this matters

The article's biggest architectural lesson is the value of a single canonical query loop. It describes `query.ts` as the system heartbeat: stream model output, collect tool calls, execute them, append results, and loop until completion. That consistency lets REPL, SDK, subagents, and headless modes share one core execution path.

Codex already has app-server turn and thread concepts, which is a good base. The opportunity is to make the turn loop more canonical across CLI, app-server, and future UIs.

### Design goal

Create one authoritative turn runtime used by:

- CLI
- app-server
- embedded UI
- headless automation
- subagents

### Desired properties

- re-entrant after tool results
- stream-aware
- interruption-safe
- typed terminal states
- easy to instrument
- easy to replay for debugging

### Recommended terminal states

Give turns explicit stop reasons such as:

- completed
- user_aborted
- approval_denied
- max_turns
- token_budget_exhausted
- hook_blocked
- unrecoverable_error
- delegated_to_background

That makes observability and debugging much easier.

### Recommendation

If Codex's loop logic is currently spread across CLI glue, app-server glue, and tool dispatch layers, collapse that toward a single shared turn engine with thin adapters above it.

---

## Part 4. Concurrent and speculative tool execution

### Why this matters

The article highlights one of Claude Code's strongest ideas: concurrency-safe tools can start before the model finishes streaming. That cuts idle time and makes the system feel much faster.

### Design goal

Allow read-only or low-risk tools to execute speculatively while the model is still streaming, then discard or keep results depending on the final structured tool call sequence.

### Suggested rollout

#### Phase 1
Allow only safe read-like tools to overlap:
- file reads
- grep/search
- directory listing
- symbol index lookups
- git status or read-only git inspection

#### Phase 2
Allow limited concurrency batches:
- multiple reads in parallel
- read plus search in parallel
- read plus metadata indexing

#### Phase 3
Support speculative launch based on partial stream output:
- if the model begins a clearly valid read/search request, launch immediately
- if later generation invalidates it, discard result

### Guardrails

- never speculate destructive tools
- never speculate tools that create approvals
- tag speculative results clearly in telemetry
- make cancellation cheap

This should be one of the highest-value additions because users feel latency immediately.

---

## Part 5. Better task and subagent orchestration

### Why this matters

The article treats tasks and subagents as a first-class abstraction, not just a cute extra. Claude Code uses them for recursive delegation and task lifecycle control. Claude's docs also describe built-in subagents with tool restrictions and inherited permissions.

Codex appears to support subagent-like behavior, but making it more explicit would improve both developer ergonomics and safety.

### Design goal

Promote tasks and subagents into explicit runtime objects.

### Recommended task model

Each task should have:
- `task_id`
- parent task or turn id
- state: `pending`, `running`, `completed`, `failed`, `cancelled`
- permission mode
- memory scope
- tool set
- cost accounting
- logs and events

### Recommended built-in subagents

#### Explore
- fast, read-only
- optimized for file discovery and analysis
- can summarize repo structure, find implementations, and gather evidence

#### Plan
- read-only or nearly read-only
- generates execution plans and validation steps
- especially good before broad code changes

#### Fix
- limited write access inside narrowed roots
- good for tightly scoped bugfixes

#### Verify
- no code edits by default
- runs tests, validates diffs, checks assumptions

### Recommended subagent features

- inherit parent memory, but optionally receive a compacted subset
- inherit permissions, but default to a more restrictive mode
- be able to bubble risky actions back to parent
- write task summaries that can feed memory candidates

### Why this helps

Large tasks split naturally into explore, plan, edit, and verify. First-class task orchestration makes that reliable instead of improvised.

---

## Part 6. Better state architecture

### Why this matters

The article argues for separating session/infrastructure state from UI-reactive state. That reduces accidental complexity and avoids turning everything into one giant reactive blob.

### Design goal

Split state into two tiers:

#### Infrastructure/session state
Rarely changing, authoritative runtime state:
- cwd
- worktree info
- model config
- approval mode
- sandbox mode
- telemetry counters
- cost totals
- task registry
- plugin registry
- session id

#### Reactive/UI state
High-frequency presentation state:
- streamed messages
- tool status indicators
- pending approvals
- progress bars
- notifications
- focus state

### Recommendation

Keep app-server and CLI reading from a shared infrastructure runtime while their UIs subscribe to derived reactive state. This also makes remote control and embedded clients easier.

---

## Part 7. Stronger permissions and approval ergonomics

### Why this matters

Codex already has real sandbox and approval controls, including granular approval policy settings and writable-root configuration. Claude adds a more user-friendly and intent-aware permission story, including explicit modes and fine-grained rules.

### Design goal

Preserve Codex's safety model while making it more expressive and more ergonomic.

### Recommended additions

#### 1. Named permission modes
Expose mode presets such as:
- `read-only`
- `default`
- `accept-edits`
- `auto-low-risk`
- `subagent-bubble`
- `danger-full-access`

These can compile down to existing Codex config and approval policy behavior.

#### 2. Intent-aware approvals
Before prompting, add context such as:
- why this action is needed
- what user request it satisfies
- whether a safer alternative exists
- whether a subagent recommended it

#### 3. Tool-specifier rules
Claude's permission docs show fine-grained rule syntax such as `Bash(npm run build)` or `Read(./.env)`. Codex should expose similarly ergonomic user-facing allow/deny rules, even if internally they map to existing approval and exec policy primitives.

#### 4. Subagent permission defaults
Subagents should default to a stricter inherited mode and escalate risky actions.

#### 5. Approval bundles
If the model needs a short sequence of tightly related safe actions, let the user approve a bounded bundle instead of approving each micro-step.

---

## Part 8. Skills improvements

### Why this matters

Both Claude Code and Codex support skills. Claude's docs make skills feel more integrated, especially through discovery, nested directories, and supporting files.

### Design goal

Make Codex skills easier to discover, easier to scope, and more tightly integrated with memory and hooks.

### Recommended additions

#### 1. Nested skill discovery
Auto-discover skills not just globally but near the working path. For example, if the user is editing `packages/frontend/`, also scan `packages/frontend/.codex/skills/`.

#### 2. Supporting file patterns
Encourage each skill directory to optionally include:
- `SKILL.md`
- `examples.md`
- `reference.md`
- `scripts/`
- `templates/`

#### 3. Memory-aware skill suggestion
If project memory says a certain workflow is common, recommend relevant skills.

#### 4. Hook-triggered skill activation
On `UserPromptSubmit`, if the prompt matches a known workflow, attach a suggested skill before the model wastes tokens rediscovering it.

#### 5. Better skill UI
Surface skill scope, last modified time, enabled status, and source path.

---

## Part 9. Plugins and extension unification

### Why this matters

Claude presents plugins as part of a broader extension story. Codex has app-server, skills, hooks, mentions, MCP, and plugin-like components, but the experience can feel more distributed.

### Design goal

Create a unified extension model so users understand one mental model:

- **skills** = reusable procedures
- **hooks** = lifecycle automation
- **plugins** = packaged capability bundles
- **MCP servers** = remote tool providers
- **mentions** = discoverable app or plugin references inside turns

### Recommended additions

#### 1. Standard plugin manifest
Define a plugin manifest that can declare:
- hooks
- skills
- MCP dependencies
- required permissions
- config schema
- persistent data dirs

#### 2. Scope handling
Support plugin installation at:
- user scope
- project scope
- local scope

#### 3. Plugin event bridge
Allow plugins to subscribe to task, turn, tool, and memory events.

#### 4. Skill/plugin bridge
Let plugins ship skills and let skills depend on plugin-provided tools.

This would make the ecosystem feel much more coherent.

---

## Part 10. Context management, compaction, and prompt caching

### Why this matters

Both systems care about context limits and performance. The article explicitly includes fork agents and prompt cache material later in the book. Codex docs also surface conversation state, compaction, and prompt caching in the run-and-scale section.

### Design goal

Make context management deliberate rather than reactive.

### Recommended additions

#### 1. Structured compaction
When context grows too large:
- preserve task state
- preserve active approvals
- preserve important memory candidates
- preserve current plan
- preserve tool results likely to matter again
- compress stale chatter aggressively

#### 2. Compaction hooks
Add:
- `CompactionStart`
- `CompactionComplete`
- `ContextTruncated`

That lets plugins or internal features preserve state before it is lost.

#### 3. Prompt caching awareness
Track cache hits and misses for:
- root instructions
- project memory
- repeated subagent prompts
- repeated skills

#### 4. Forked context branches
For exploration-heavy tasks, support cheap branch/fork contexts that can try alternate plans without polluting the main thread until selected.

This is especially useful for debugging, large refactors, and compare-and-choose workflows.

---

## Part 11. Remote control, app-server, and observability

### Why this matters

Codex already has an app-server and turn lifecycle notifications. That is a major asset. The Claude architecture emphasizes having one system power REPL, SDK, subagents, and headless modes.

### Design goal

Make the app-server the clean remote facade over the same core runtime the CLI uses.

### Recommended additions

#### 1. Canonical event stream
Standardize a clean stream of events such as:
- thread started
- turn started
- turn item delta
- tool requested
- approval requested
- tool completed
- task created
- task completed
- memory updated
- compaction occurred

#### 2. Structured telemetry
Collect:
- latency by stage
- model streaming time
- tool execution time
- speculative execution stats
- approval delays
- memory load/selection stats
- prompt cache hit rates

#### 3. Replay and debugging tools
Store enough structured turn data to replay a turn deterministically for debugging.

#### 4. Better notifications
Codex already supports notification options. Extend that to important lifecycle events for long-running jobs, subagent completion, or approval timeout.

---

## Part 12. Performance engineering changes worth copying

### Why this matters

The article keeps returning to real-world scale, latency, and throughput. This should not be treated as polish. It is part of what makes a harness feel like a workhorse.

### Recommended additions

#### 1. Concurrency-safe tool overlap
Covered earlier, but worth restating because it is high impact.

#### 2. Cheap models for explore agents
Use cheaper, faster models for read-only exploration subagents and reserve the expensive model for synthesis or edits.

#### 3. Incremental indexing
Build or reuse searchable indexes for:
- files
- symbols
- tests
- dependencies
- recent tool outputs

#### 4. Smart context inclusion
Do not repeatedly inject full repo instructions, memory, and skill payloads when a compact summary will do.

#### 5. Result reuse
Cache safe read/search results inside a turn so repeated asks do not hit disk or spawn duplicate processes.

#### 6. Backpressure-aware streaming
Make sure the UI, CLI, and app-server consumers cannot force the runtime into awkward buffering behavior.

---

## Part 13. A practical phased roadmap

### Phase 1: high-leverage and low-risk
- add `.codex/MEMORY.md`
- add user `MEMORY.md`
- load memory alongside `AGENTS.md`
- add `SessionEnd` memory extraction
- broaden `SessionStart` context injection
- improve approvals with rationale text
- add nested skill discovery

### Phase 2: make it feel materially smarter
- path-scoped rules
- auto memory with dedupe and compaction
- task and subagent registry
- `TaskCompleted` hooks
- focused verify agent
- structured compaction hooks
- better telemetry

### Phase 3: make it feel like a full harness
- speculative tool execution
- forked context branches
- richer permission modes
- plugin manifest and lifecycle bridge
- task-aware UI and remote dashboards
- memory-aware skill and subagent routing

---

## Part 14. Recommended architecture boundaries

If you are forking Codex, I would roughly separate responsibilities like this:

### `core/runtime`
- canonical turn/query loop
- terminal states
- turn replay support

### `core/tasks`
- task and subagent lifecycle
- task registry
- escalation rules

### `core/memory`
- memory discovery
- relevance selection
- extraction
- dedupe
- compaction

### `core/hooks`
- hook registry
- event schemas
- hook execution policy
- safety boundaries

### `core/permissions`
- user-facing permission modes
- approval explanation generation
- subagent permission inheritance

### `core/context`
- compaction
- cache awareness
- inclusion strategy

### `core/extensions`
- skills discovery
- plugin manifests
- MCP bridges
- mention resolution

### `server/app`
- thread and turn endpoints
- canonical event stream
- client-facing notifications

### `ui/cli`
- rendering only
- pending approvals UI
- task tree visualization
- memory and hook inspection commands

---

## Part 15. What I would prioritize first if the goal is "better than stock Codex"

If you want maximum payoff quickly, I would do these in order:

1. **Memory layer**
2. **Better hooks and lifecycle coverage**
3. **Better task/subagent model**
4. **Approval ergonomics and permission modes**
5. **Nested skills and extension unification**
6. **Speculative/concurrent tool execution**
7. **Compaction and prompt-cache-aware context management**
8. **Remote observability and replay tooling**

Why this order:
- memory and hooks improve day-to-day usefulness immediately
- task orchestration makes large jobs more reliable
- approval ergonomics improve trust
- concurrency and advanced context work are huge wins, but they are trickier to get right safely

---

## Closing view

The most important lesson from the Claude architecture article is not one specific feature. It is the idea that an agentic coding harness becomes powerful when the model is wrapped in a coherent runtime:

- one core loop
- one consistent tool model
- explicit task orchestration
- persistent memory
- lifecycle hooks
- strong permissions
- thoughtful context management
- performance engineering everywhere

Codex already has a lot of the raw ingredients: layered instructions, config, hooks, approvals, sandboxing, app-server turns, skills, and an extension surface. A strong fork would make those pieces feel like one product instead of several adjacent subsystems.

That is the real opportunity.

---

## Source notes

This blueprint was informed by:

- the Claude Code architecture article's six abstractions, golden-path query loop, task model, two-tier state split, memory layer, hook system, streaming/concurrent tool execution, and permission model
- Claude Code docs on memory, hooks, permissions, skills, subagents, and plugins
- Codex docs on layered `AGENTS.md`, configuration, hooks, approvals and sandboxing, and app-server turn/skills support

