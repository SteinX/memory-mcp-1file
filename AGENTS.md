# 🤖 AGENTS.md — AI Agent Master Protocol (MCP + VIDA Thinking)

<identity>
You are an AI agent operating with the **Memory MCP** and **VIDA Thinking Framework**.
You must adhere to strict workflows, utilize specialized tools, and maintain context across sessions.
Communication language with the user: Ukrainian.
</identity>

---

## ⛔ L0 INVARIANTS (Never violate under any circumstance)

<!-- SURVIVE_COMPRESSION: These rules MUST survive context window clearing -->
1. **[MUST]** After ANY context compression/clearing, your FIRST action must be to read `AGENTS.md`.
2. **[MUST NOT]** Never perform auto-commits without explicit user permission.
3. **[MUST NOT]** Never propose or implement "hotfixes". Demand root-cause solutions.
4. **[MUST NOT]** Never delete files or code blocks without explicit discussion.
5. **[MUST]** Always use `/vida-think` for architectural decisions, debugging, or complex refactoring.

---

## 🔄 POST-COMPRESSION BOOT SEQUENCE (⛔ CRITICAL BLOCKER — NO EXCEPTIONS)

<!-- SURVIVE_COMPRESSION: This entire section MUST be executed after every compression event -->

### ⛔ THIS IS A HARD STOP. DO NOT RESPOND TO THE USER, DO NOT CONTINUE ANY TASK, DO NOT ANSWER ANY QUESTION UNTIL ALL STEPS BELOW ARE FULLY EXECUTED WITH `Read` TOOL CALLS.

When context is compressed or cleared, you MUST execute this sequence **before ANY other action**.
**Violation = immediate protocol failure. No shortcuts. No "I already know". No skipping.**

1. **Read `AGENTS.md`** — this file (full, not partial). Re-internalize all L0 invariants and protocols.
2. **Read all `/vida-think` algorithm files** — required for architectural decisions:
   - `_vida/algorithms/meta-analysis.md`
   - `_vida/algorithms/pr-cot.md`
   - `_vida/algorithms/mar.md`
   - `_vida/algorithms/5-solutions.md`
   - `_vida/shared/reasoning-modules.md`
3. **Read `_vida/shared/beads-protocol.md`** — state management protocol.
4. **Read the context summary** — if one exists in the compressed context, use it to orient.
5. **Do NOT continue work** until steps 1-3 are complete.

### Detection heuristic
If your first tool call after compression is NOT `Read AGENTS.md` — you are violating this protocol.
If you skip ANY file from step 2 — you are violating this protocol.
If you respond to the user before completing step 3 — you are violating this protocol.

**Why**: Algorithms define HOW to think. Without them, `/vida-think` produces shallow analysis
that violates L0 #5. Compression strips algorithm knowledge — it must be reloaded explicitly.

**Anti-pattern**: Proceeding with tasks using "remembered" algorithm steps from pre-compression.
This leads to incomplete META-analysis, skipped PR-CoT passes, or fabricated MAR scores.

---

## 🔀 PHASE TRANSITION ROUTING (⛔ BLOCKING)

> ⛔ STOP. Before starting ANY new phase of work, you MUST execute the MANDATORY BOOT PROTOCOL.
> Your output is VOID if this gate is not passed. "I already know how to code" is NOT an excuse.

### GATE-0: Phase Identification & Loading

Before ANY output, when a user requests a phase transition, you MUST:

1. **Classify** the user's message against the `PHASE_ROUTING_TABLE` (see below).
2. **IF** a phase is identified:
   - State explicitly: `[ROUTER] Phase detected: vida_{phase_name}`
   - State explicitly: `[ROUTER] Loading: _vida/commands/vida-{phase}.md`
   - Call the `Read` tool on `_vida/commands/vida-{phase}.md` as your FIRST action.
   - **WAIT** for the tool result.
   - Proceed ONLY after executing the `PROOF-OF-LOAD` requirement.
3. **IF** no phase is identified but a transition is suspected:
   - Default to `/vida-status` to orient.

### 🧾 PROOF-OF-LOAD REQUIREMENT (CRITICAL)

After the `Read` tool call, your NEXT output line MUST be:
```
[PROOF-OF-LOAD] Loaded: _vida/commands/vida-{phase}.md
Summary of first 3 lines: {actual content extracted directly from the tool result}
```
**⛔ If you cannot produce this exact summary from the tool result — you did NOT load the file. STOP. Do not generate code. Retry the tool call.**

---



---

---

# 🧠 Memory Protocol (Memory MCP)

<critical>
This protocol is MANDATORY. Violation = loss of context between sessions.
Goal: any agent can continue another agent's work without losing context.
</critical>

---

## ⚡ Quick Reference

<quick_reference>

| Situation | Action | Section |
|-----------|--------|---------|
| 🚀 Session start | `search_text` → show TASK → AUTO_CONTINUE | [SESSION_START](#-session_start-session-startup-algorithm) |
| 🔍 Found TASK | Show to user → wait 30 sec | [AUTO_CONTINUE](#-auto_continue-confirmation-protocol-with-timer) |
| 🆕 Ad-hoc Task | Create TASK (ad_hoc) → SYNC | [AD_HOC_TASK](#-ad_hoc_task-user--external-tasks) |
| 🧪 Research | Create RESEARCH → Cycle → SYNC | [RESEARCH_PROTOCOL](#-research_protocol-investigation--architecture) |
| ✏️ Changed subtask | `update_memory` → SYNC | [SYNC_PROTOCOL](#-sync_protocol-status-synchronization) |
| ✅ Completed WP | `invalidate` → update EPIC → SYNC | [TASK_COMPLETE](#-task_complete-completing-work-package) |

</quick_reference>

<critical_reminder>
🔴 MOST COMMON MISTAKE: Continuing work WITHOUT showing task state to user.
User message BEFORE showing TASK — is NOT a confirmation!
</critical_reminder>

---

## 📋 Mandatory Prefix System

<prefixes>

**EVERY memory entry MUST start with a prefix.**

| Prefix | memory_type | Purpose | Priority |
|--------|-------------|---------|----------|
| `PROJECT:` | semantic | Overall project state | 🟢 Low |
| `EPIC:` | procedural | WP group, feature progress | 🟡 Medium |
| `TASK:` | episodic | Active Work Package | 🔴 **Highest** |
| `RESEARCH:` | semantic | Investigation & Findings | 🔵 High |
| `DECISION:` | semantic | Architectural decision with reason | 🟢 Low |
| `CONTEXT:` | semantic | Technical context (stack, architecture) | 🟢 Low |
| `USER:` | semantic | User preferences | 🟢 Low |

</prefixes>

<constraints type="prefixes">
- FORBIDDEN to store entries WITHOUT prefix
- FORBIDDEN to use other prefixes
- FORBIDDEN to store TASK/EPIC without `Updated:` field
</constraints>

---



---



---

## ⏳ AUTO_CONTINUE: Confirmation Protocol with Timer

<auto_continue priority="BLOCKING">
MANDATORY when finding an active task.
Show state → Wait for confirmation OR 30 sec timer.
</auto_continue>

### ⚠️ CRITICAL: What is NOT a confirmation

<critical_rule>
User message BEFORE showing task state — is NOT a confirmation!
User cannot confirm what they haven't seen yet.
</critical_rule>

| Scenario | Example | Is this confirmation? |
|----------|---------|----------------------|
| User wrote something → you found TASK | "Continue" before search | ❌ **NO** — they haven't seen the task |
| You showed TASK → user responded | "Yes/go ahead" after showing | ✅ **YES** |
| You showed TASK → 30 sec timer | Silence | ✅ **YES** (auto-continue) |

<checklist id="auto_continue">
- [ ] Showed task state to user (table)
- [ ] Asked "Continue this task?"
- [ ] Started timer `sleep 30`
- [ ] Received confirmation OR timer triggered
- [ ] ONLY AFTER this continued work
</checklist>

### Algorithm

```
┌─────────────────────────────────────────────────────────────┐
│                    AUTO_CONTINUE                            │
├─────────────────────────────────────────────────────────────┤
│ 1. Show user the found task:                                │
│                                                             │
│    ╔══════════════════════════════════════════════════════╗ │
│    ║ 🔍 Found unfinished task in memory:                  ║ │
│    ║                                                      ║ │
│    ║ TASK: {WP-id} - {name}                               ║ │
│    ║ Status: {status}                                     ║ │
│    ║ Current: {current subtask}                           ║ │
│    ║ Progress: {N}/{total} subtasks                       ║ │
│    ║ Command: {continuation command}                      ║ │
│    ║                                                      ║ │
│    ║ Continue this task?                                  ║ │
│    ║ (auto-continue in 30 sec)                            ║ │
│    ╚══════════════════════════════════════════════════════╝ │
│                                                             │
├─────────────────────────────────────────────────────────────┤
│ 2. SIMULTANEOUSLY start timer:                              │
│                                                             │
│    bash: sleep 30 && echo "AUTO_CONTINUE_TRIGGER"           │
│    timeout: 35000ms                                         │
│                                                             │
├─────────────────────────────────────────────────────────────┤
│ 3. Handle result:                                           │
│                                                             │
│    IF user responded BEFORE timer:                          │
│       → "yes/continue/go ahead" → continue                  │
│       → "no/stop/other" → ask what to do                    │
│       → new task → switch to it                             │
│                                                             │
│    ELSE IF timer triggered (no response):                   │
│       → Automatically continue task                         │
│       → Notify: "⏳ Continuing automatically..."            │
│                                                             │
├─────────────────────────────────────────────────────────────┤
│ 4. Launch recovery command:                                 │
│                                                             │
│    IF TASK has Command field (e.g. /spec-kitty.implement):  │
│       → Execute slashcommand (see below)                    │
│                                                             │
│    ELSE:                                                    │
│       → Continue work manually using Context                │
│                                                             │
└─────────────────────────────────────────────────────────────┘
```

### 🔧 What is Command (slashcommand)

<slashcommand>
**Command** — is NOT a bash command, but a reference to an .md file with agent instructions.

**Format**: `/{prefix}.{action} {arguments}`
- Example: `/spec-kitty.implement WP01`
</slashcommand>

**Execution algorithm:**

1. **Parse the command:**
   - `/spec-kitty.implement WP01` → command=`spec-kitty.implement`, args=`WP01`
   
2. **Find instruction file in IDE/CLI directory:**
   
   | IDE/CLI | Path |
   |---------|------|
   | OpenCode | `.opencode/command/{command}.md` |
   | Cursor | `.cursor/command/{command}.md` |
   | Claude Code | `.claude/command/{command}.md` |
   | Windsurf | `.windsurf/command/{command}.md` |
   
3. **Read the ENTIRE file and execute instructions:**
   - `$ARGUMENTS` → substitute args (e.g. `WP01`)
   - File contains FULL workflow with all steps
   - Execute step by step

<warning>
`.opencode/command/spec-kitty.implement.md` = **276 lines** of full workflow
`.kittify/.../implement.md` = **12 lines** just bash command

If you only read the short file — you're missing 90% of instructions!
</warning>

### Output Format (MANDATORY)

<output_format>
Start your response EXACTLY like this:

```
🔍 **Found unfinished task in memory:**

┌────────────────────────────────────────────┐
│ TASK: WP01-poc-validation                  │
│ Status: in_progress                        │
│ Current: T002 (rmcp PoC)                   │
│ Progress: 1/3 subtasks done                │
│ Command: /spec-kitty.implement WP01        │
│                                            │
│ Subtasks:                                  │
│   [x] T001: Candle PoC - DONE              │
│   [ ] T002: rmcp PoC ← current             │
│   [ ] T003: SurrealDB PoC                  │
└────────────────────────────────────────────┘

**Continue this task?**
_(auto-continue in 30 seconds if no response)_
```
</output_format>

<constraints type="auto_continue">
- FORBIDDEN to continue WITHOUT showing information to user
- FORBIDDEN to wait longer than 30 seconds
- FORBIDDEN to ignore user response if it arrived
</constraints>

---



---

## 🔄 TASK_UPDATE: When to Update Memory

<task_update>
Update TASK on EVERY significant state change.
DO NOT update on every tool call — that's too frequent.
</task_update>

| Trigger | Action |
|---------|--------|
| Completed subtask (T001 → T002) | `update_memory` → **EXECUTE SYNC_PROTOCOL** |
| Encountered blocker | `update_memory` (blocked) → **EXECUTE SYNC_PROTOCOL** |
| Made a decision | + `store_memory` DECISION |
| User says "stop/pause" | `update_memory` (paused) → **EXECUTE SYNC_PROTOCOL** |
| Created/modified files | Add to Context |
| Fully completed WP | `invalidate` + new TASK → **EXECUTE SYNC_PROTOCOL** |

<checklist id="task_update">
- [ ] Updating TASK when Current subtask changes
- [ ] Adding changed files to Context
- [ ] Creating DECISION for important decisions
- [ ] Updating Status on blockers
- [ ] **EXECUTE SYNC_PROTOCOL** (Memory + Task Tool)
</checklist>

<constraints type="task_update">
- FORBIDDEN to update on every tool call (too frequent)
- FORBIDDEN to NOT update on subtask change (too rare)
- FORBIDDEN to leave Status=in_progress when blocked
- FORBIDDEN to have conflicting status between Memory, Task Tools, and Documents
</constraints>

---

## ✅ TASK_COMPLETE: Completing Work Package

<task_complete>
EXECUTE BEFORE moving to next WP.
Step order is important!
</task_complete>

<checklist id="task_complete">
- [ ] `invalidate(id="{task_memory_id}", reason="WP completed")`
- [ ] `update_memory(id="{epic_id}")` with Progress: {N+1}/{total}
- [ ] `store_memory("DECISION: ...")` for important decisions
- [ ] `store_memory("TASK: ...")` for new WP
- [ ] **EXECUTE SYNC_PROTOCOL** (Triple Sync)
</checklist>

### Algorithm

```
1. invalidate(
     id="{task_memory_id}",
     reason="WP completed successfully"
   )

2. update_memory(id="{epic_id}") with:
   - Progress: {N+1}/{total}
   - Current WP: {next WP}
   
3. If there were important decisions:
   store_memory(content="DECISION: ...", memory_type="semantic")

4. store_memory for new TASK:
   - Type: standard
   - Status: in_progress
   - Current: first subtask
   - Path: path to new WP file
   
5. EXECUTE SYNC_PROTOCOL (Update Task Tool + Docs)
```

<constraints type="task_complete">
- FORBIDDEN to move to new WP WITHOUT invalidating old TASK
- FORBIDDEN to forget updating EPIC Progress
- FORBIDDEN to use delete_memory — ONLY invalidate
- FORBIDDEN to skip SYNC_PROTOCOL
</constraints>

---

## ⚡ AD_HOC_TASK: User & External Tasks

<ad_hoc_task>
Protocol for tasks NOT defined in the standard Roadmap/Epic structure.
Includes: User requests, Bug fixes outside sprints, One-off maintenance.
</ad_hoc_task>

### Algorithm

```
┌─────────────────────────────────────────────────────────────┐
│                    AD_HOC_TASK                              │
├─────────────────────────────────────────────────────────────┤
│ 1. Creation:                                                │
│    store_memory("TASK: ...")                                │
│    - ID: {generated_id} (e.g. USER-20240101)                │
│    - Type: ad_hoc                                           │
│    - Status: in_progress                                    │
│    - Description: {user request}                            │
│                                                             │
│ 2. Sync Start:                                              │
│    → Add to Task Tool (IDE/CLI) under "Ad-hoc" or similar   │
│                                                             │
│ 3. Execution:                                               │
│    → Execute subtasks                                       │
│    → SYNC_PROTOCOL after EACH step/subtask                  │
│                                                             │
│ 4. Completion:                                              │
│    → invalidate(id="{task_id}", reason="Completed")         │
│    → Mark Done in Task Tool                                 │
│    → Notify User                                            │
└─────────────────────────────────────────────────────────────┘
```

<constraints type="ad_hoc_task">
- FORBIDDEN to execute "just a quick task" without recording in Memory
- FORBIDDEN to skip Task Tool entry for ad-hoc tasks
- **MANDATORY** to follow SYNC_PROTOCOL (Memory + Tool)
</constraints>

---

## 🧪 RESEARCH_PROTOCOL: Investigation & Architecture

<research_protocol>
Protocol for investigations, selecting libraries, and designing architecture.
Balances Memory limits by storing details in files and summaries in Memory.
</research_protocol>

### ⚖️ Memory vs File Strategy

| Type | Where to store | Content |
|------|----------------|---------|
| **Meta-data** | **Memory (MCP)** | Status, Goal, *Key* Open Questions, *Key* Findings. <br/> **Limit:** ~1000-2000 chars per record. |
| **Details** | **File (.md)** | Full benchmarks, long descriptions, code examples, logs. |

### Algorithm

```
┌─────────────────────────────────────────────────────────────┐
│                  RESEARCH_PROTOCOL                          │
├─────────────────────────────────────────────────────────────┤
│ 1. Initialization:                                          │
│    Create file: doc/research/{topic}.md                     │
│    store_memory("RESEARCH: ...")                            │
│    - Path: {path to file}                                   │
│    - Goal: {objective}                                      │
│    - Open Questions: {list of questions}                    │
│    → EXECUTE SYNC_PROTOCOL                                  │
│                                                             │
│ 2. Research Cycle (Iterative):                              │
│    → Investigate / Experiment                               │
│    → Write details to File (.md)                            │
│    → Update Memory ("RESEARCH: ...")                        │
│         - Remove answered questions from Open Questions     │
│         - Add answer to Conclusions                         │
│    → EXECUTE SYNC_PROTOCOL                                  │
│                                                             │
│ 3. Completion:                                              │
│    → Formulate final Decisions                              │
│    → store_memory("DECISION: ...") (for approved choices)   │
│    → invalidate(id="{research_id}", reason="Completed")     │
│    → Update PROJECT/EPIC with results                       │
│    → EXECUTE SYNC_PROTOCOL                                  │
└─────────────────────────────────────────────────────────────┘
```

<constraints type="research_protocol">
- FORBIDDEN to dump huge texts into Memory (use linked File)
- FORBIDDEN to conduct research without defining "Goal" and "Open Questions"
- **MANDATORY** to fix Approved Decisions as separate DECISION records upon completion
</constraints>

---

## 🏁 EPIC_COMPLETE: Completing Feature

<epic_complete>
EXECUTE when closing all WPs of a feature.
</epic_complete>

<checklist id="epic_complete">
- [ ] `invalidate(id="{epic_id}", reason="feature completed")`
- [ ] `store_memory("PROJECT: ...")` with Last Completed
- [ ] `store_memory("DECISION: ...")` for each important decision
- [ ] **TRIPLE SYNC:** Update active Task Management Tool (CLI/IDE) status
- [ ] **TRIPLE SYNC:** Mark Epic as Done in active Task Management Tools (CLI/IDE)
- [ ] **GIT COMMIT (MANDATORY):** Commit all changes for the completed feature
</checklist>

### Algorithm

```
1. invalidate(id="{epic_id}", reason="feature completed")

2. store_memory(content="PROJECT: ...") with:
   - Last Completed: {feature-id}
   - Current Epic: None | {next feature}
   
3. For EACH important decision of the feature:
   store_memory(content="DECISION: ...", memory_type="semantic")

4. GIT COMMIT (MANDATORY):
   git add -A
   git commit -m "feat({feature-id}): complete {feature description}"
   
   Commit message format:
   - feat({id}): for new features
   - fix({id}): for bug fix features
   - refactor({id}): for refactoring features
   
   Include in commit body (optional):
   - List of completed WPs
   - Key decisions made
```

<constraints type="epic_complete">
- FORBIDDEN to complete epic WITHOUT updating PROJECT
- FORBIDDEN to lose DECISION records
- FORBIDDEN to complete epic WITHOUT git commit of all changes
</constraints>

---

## 🔍 Search Method Selection

| Situation | Method | Why |
|-----------|--------|-----|
| **Session start** | `search_text` | BM25 accurately finds prefixes |
| Search by ID | `get_memory` | Direct retrieval |
| Search decisions | `search_text("DECISION:")` | Exact prefix match |
| Semantic search | `search` or `recall` | When exact words unknown |
| Change history | `get_valid_at` | State at point in time |
| All current | `get_valid` | Filters by valid_until |

<important>
`recall` uses hybrid search (vector + BM25 + PPR), 
but for prefixes `search_text` is more reliable.
</important>

---

## 📊 Knowledge Graph (optional)

<knowledge_graph>
Use for complex projects with dependencies.
</knowledge_graph>

```
# Creating hierarchy
create_entity(name="Feature:001-memory-mcp", entity_type="feature")
create_entity(name="WP:WP01", entity_type="work_package")
create_entity(name="Task:T001", entity_type="task")

# Relations
create_relation(from="WP:WP01", to="Feature:001", relation_type="belongs_to")
create_relation(from="Task:T001", to="WP:WP01", relation_type="part_of")
create_relation(from="WP:WP02", to="WP:WP01", relation_type="depends_on")

# Navigation
get_related(entity_id="WP:WP01", depth=2, direction="both")
```

---

## ⚠️ Critical Rules

### MUST (REQUIRED)

<must_do>
- ✅ Call `search_text` at the start of EVERY session
- ✅ Show task state to user BEFORE continuing (AUTO_CONTINUE)
- ✅ Every entry starts with prefix (PROJECT:/EPIC:/TASK:/DECISION:)
- ✅ Every TASK/EPIC has `Updated:` field with ISO timestamp
- ✅ TASK has fields: Status, Current, Path, Command, Agent
- ✅ Use `invalidate` instead of `delete_memory`
- ✅ Update TASK on subtask change
- ✅ Update EPIC on WP completion
- ✅ Store DECISION with REASON
</must_do>

### MUST NOT (FORBIDDEN)

<must_not>
- ❌ Store entries without prefix
- ❌ Start work without searching memory
- ❌ Continue work WITHOUT showing task state to user
- ❌ Consider user message BEFORE showing task as confirmation
- ❌ Move to new WP without invalidating old TASK
- ❌ Use `delete_memory` (only invalidate)
- ❌ Ignore found active TASK records
- ❌ Store duplicates — use `update_memory`
</must_not>

---

---

## 📋 Rules Summary

| Rule | Description |
|------|-------------|
| **Communication language** | Ukrainian only |
| **Memory: start** | REQUIRED `search_text` + show to user |
| **Memory: completion** | REQUIRED `invalidate` + `store_memory` |
| **Memory: deletion** | FORBIDDEN `delete_memory`, only `invalidate` |
| **Thinking: Boot** | REQUIRED to read algorithms after context wipe |
| **Thinking: Routing** | REQUIRED to identify phases and load specific commands |

---

## 🐳 Docker Local Debug Protocol

<docker_debug>
Інструкція для локального тестування Memory MCP через Docker.
</docker_debug>

### Quick Start

```bash
# 1. Build binary (fast profile = debug-friendly, ~30s)
cargo build --profile fast

# 2. Build Docker image
docker build -f Dockerfile.fast -t memory-mcp:dev .

# 3. Run container (4GB limit, persistent data volume)
docker run -d \
  --name memory-mcp-dev \
  --memory 4g \
  -v /home/unnamed/project:/project \
  -v memory-mcp-data:/data \
  -e RUST_LOG=info \
  -e RUST_BACKTRACE=1 \
  memory-mcp:dev \
  sh -c 'tail -f /dev/null | /usr/local/bin/memory-mcp 2>&1'
```

### Важливі нюанси

| Аспект | Правильно | Неправильно |
|--------|-----------|-------------|
| **stdin** | `tail -f /dev/null \| memory-mcp` | `cat /dev/zero \| memory-mcp` (**OOM!** нескінченний бінарний потік) |
| **Volume для /data** | `-v memory-mcp-data:/data` (named volume, persist між rm/run) | без volume (модель ~800MB качається з мережі щоразу) |
| **Memory limit** | `--memory 4g` (мінімум для Qwen3 1024d) | без ліміту (SurrealKV block cache з'їсть RAM/2-1GB) |

### Моніторинг

```bash
# Статус контейнера (OOM check)
docker inspect memory-mcp-dev --format '{{.State.Status}} OOM:{{.State.OOMKilled}}'

# RAM + CPU в реальному часі
docker stats memory-mcp-dev --no-stream --format "MEM: {{.MemUsage}} ({{.MemPerc}}) | CPU: {{.CPUPerc}}"

# Прогрес індексації
docker logs memory-mcp-dev 2>&1 | grep -c "Indexing file"

# Помилки
docker logs memory-mcp-dev 2>&1 | grep -iE "panic|error|OOM|failed" | tail -10

# Останні логи
docker logs memory-mcp-dev 2>&1 | tail -20

# Kernel OOM killer (якщо контейнер зник)
dmesg | grep -iE "oom|killed|memory-mcp" | tail -10
```

### Rebuild цикл

```bash
# Повний цикл: build → image → restart (зберігає /data volume!)
cargo build --profile fast && \
docker rm -f memory-mcp-dev && \
docker build -f Dockerfile.fast -t memory-mcp:dev . && \
docker run -d \
  --name memory-mcp-dev \
  --memory 4g \
  -v /home/unnamed/project:/project \
  -v memory-mcp-data:/data \
  -e RUST_LOG=info \
  -e RUST_BACKTRACE=1 \
  memory-mcp:dev \
  sh -c 'tail -f /dev/null | /usr/local/bin/memory-mcp 2>&1'
```

### Memory Budget (Qwen3 1024d, 4GB limit)

```
SurrealKV block cache:  256MB  (env SURREAL_SURREALKV_BLOCK_CACHE_CAPACITY)
Qwen3 model (mmap):   1200MB  (поступово page-in)
HNSW indexes (4×):      60MB  (~10K vectors)
BM25 engine:            40MB  (streaming rebuild)
Runtime + stacks:      100MB  (8MB per thread)
─────────────────────────────
Steady state:         ~1700MB
+ Indexing peak:       +400MB
─────────────────────────────
Peak:                 ~2100MB  (headroom ~1900MB при 4GB)
```

### Відомі проблеми

| Проблема | Причина | Рішення |
|----------|---------|---------|
| "Previous indexing interrupted" | Попередній контейнер був OOM-killed під час індексації | Нормально — перезапуск індексації |
| Модель качається з мережі | `/data` volume не збережений | Використовувати named volume `-v memory-mcp-data:/data` |
| OOM при 4GB | SurrealKV block cache авто = RAM/2-1GB | ENV `SURREAL_SURREALKV_BLOCK_CACHE_CAPACITY=268435456` (256MB) |
| Індексація "failed" після 300с | completion_monitor stall timeout занадто короткий | Збільшено до 1800с (30 хв) для Qwen3 CPU |
| CPU лише 130% | `RAYON_NUM_THREADS` не встановлено | ENV `RAYON_NUM_THREADS=0` (авто-detect всі ядра) |

---

*Last updated: 2026-03-01*
