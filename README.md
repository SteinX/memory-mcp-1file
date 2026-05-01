# 🧠 Memory MCP Server

[![Release](https://github.com/pomazanbohdan/memory-mcp-1file/actions/workflows/release.yml/badge.svg)](https://github.com/pomazanbohdan/memory-mcp-1file/actions/workflows/release.yml)
[![Docker](https://img.shields.io/badge/docker-ghcr.io-blue.svg)](https://github.com/pomazanbohdan/memory-mcp-1file/pkgs/container/memory-mcp-1file)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![Built with Rust](https://img.shields.io/badge/Built%20with-Rust-d64e25.svg)](https://www.rust-lang.org)
[![Architecture](https://img.shields.io/badge/Architecture-Single%20Binary-success.svg)](#)

A high-performance, **pure Rust** Model Context Protocol (MCP) server that provides persistent, semantic, and graph-based memory for AI agents.

Works perfectly with:
*   **Claude Desktop**
*   **Claude Code** (CLI)
*   **Gemini CLI**
*   **Cursor**
*   **OpenCode**
*   **Cline** / **Roo Code**
*   Any other MCP-compliant client.

### 🏆 The "All-in-One" Advantage

Unlike other memory solutions that require a complex stack (Python + Vector DB + Graph DB), this project is **a single, self-contained executable**.

*   ✅ **No External Database** (SurrealDB is embedded)
*   ✅ **No API Keys, No Cloud, No Python** — Everything runs **100% locally** via an embedded ONNX runtime. The embedding model is baked into the binary and runs on CPU. Nothing leaves your machine.
*   ✅ **Zero Setup** (Just run one Docker container or binary)

It combines:
1.  **Vector Search** (FastEmbed) for semantic similarity.
2.  **Knowledge Graph** (PetGraph) for entity relationships.
3.  **Code Indexing** with **symbol graph** (calls, extends, implements) for deep codebase understanding.
4.  **Hybrid Retrieval** (Reciprocal Rank Fusion) for best results.
5.  **Explicit consolidation** for exact duplicate memories via replacement links, without silently changing write semantics.
6.  **Preview / Apply Alignment** with `plan_fingerprint`, plus matched-summary and execution-summary fields so consolidation can be previewed, verified, and audited before and after execution.
7.  **Read-side Consolidation Traceability** so `get_memory`, `list_memories`, and `get_valid` expose a normalized `consolidation_trace` summary instead of forcing callers to reconstruct lifecycle state from raw fields.
8.  **Replacement Lineage Navigation** so read APIs also expose a compact `replacement_lineage` summary for following supersession chains without reconstructing them client-side.
9.  **Operator Attention Summaries** so preview/apply/read responses surface a compact `attention_summary` for multi-match, partial-supersede, lineage-cycle, truncation, and fingerprint-check signals without requiring callers to infer risk from raw fields.
10. **Retrieval/Read Truth Alignment** so `search_memory` and `recall` also surface consolidation truth summaries instead of requiring a second hop to `get_memory` before a caller can see lifecycle state.
11. **Plan Diagnostics Echo** so `preview_consolidate_memory` and `consolidate_memory` return a normalized `plan_diagnostics` view of the fingerprint inputs, making stale-plan mismatches explainable without reconstructing the plan by hand.
12. **Hash-First Duplicate Lookup** so exact-duplicate consolidation can narrow candidates by `content_hash` first, while still falling back to exact-content matching for older memories that predate create-time hashing.
13. **Lookup Diagnostics** so preview/apply responses explicitly show whether duplicate detection came from `hash-first` narrowing or `exact-content` fallback for legacy no-hash memories.
14. **Operator Summary** so preview/apply/read/retrieval responses expose one compact `operator_summary` entrypoint that tells clients which diagnostic section to inspect first.

### 🏗️ Architecture

```mermaid
graph TD
    User[AI Agent / IDE]
    
    subgraph "Memory MCP Server"
        MS[MCP Server]
        
        subgraph "Core Engines"
            ES[Embedding Service]
            GS[Graph Service]
            CS[Codebase Service]
        end
        
        MS -- "Store / Search" --> ES
        MS -- "Relate Entities" --> GS
        MS -- "Index" --> CS
        
        ES -- "Vectorize Text" --> SDB[(SurrealDB Embedded)]
        GS -- "Knowledge Graph" --> SDB
        CS -- "AST Chunks" --> SDB
    end

    User -- "MCP Protocol" --> MS
```

> **[Click here for the Detailed Architecture Documentation](./ARCHITECTURE.md)**

---

## 🤖 Agent Integration (System Prompt)

Memory is useless if your agent doesn't check it. To get the "Long-Term Memory" effect, you must instruct your agent to follow a strict protocol.

We provide a battle-tested **[Memory Protocol (AGENTS.md)](./AGENTS.md)** that you can adapt.

### 🛡️ Core Workflows (Context Protection)

The protocol implements specific flows to handle **Context Window Compaction** and **Session Restarts**:

1.  **🚀 Session Startup**: The agent *must* search for `TASK: in_progress` immediately. This restores the full context of what was happening before the last session ended or the context was compacted.
2.  **⏳ Auto-Continue**: A safety mechanism where the agent presents the found task to the user and waits (or auto-continues), ensuring it doesn't hallucinate a new task.
3.  **🔄 Triple Sync**: Updates **Memory**, **Todo List**, and **Files** simultaneously. If one fails (e.g., context lost), the others serve as backups.
4.  **🧱 Prefix System**: All memories use prefixes (`TASK:`, `DECISION:`, `RESEARCH:`) so semantic search can precisely target the right type of information, reducing noise.

These workflows turn the agent from a "stateless chatbot" into a "stateful worker" that survives restarts and context clearing.

### Recommended System Prompt Snippet

Instead of scattering instructions across IDE-specific files (like `.cursorrules`), establish `AGENTS.md` as the **Single Source of Truth**.

Instruct your agent (in its base system prompt) to:
1.  **Read `AGENTS.md`** at the start of every session.
2.  **Follow the protocols** defined therein.

Here is a minimal reference prompt to bootstrap this behavior:

```markdown
# 🧠 Memory & Protocol
You have access to a persistent memory server and a protocol definition file.

1.  **Protocol Adherence**:
    - READ `AGENTS.md` immediately upon starting.
    - Strictly follow the "Session Startup" and "Sync" protocols defined there.

2.  **Context Restoration**:
    - Run `search_text("TASK: in_progress")` to restore context.
    - Do NOT ask the user "what should I do?" if a task is already in progress.
```

### Why this matters?
Without this protocol, the agent loses context after compaction or session restarts. With this protocol, it maintains the **full context of the current task**, ensuring no steps or details are lost, even when the chat history is cleared.

---

## 🔌 Client Configuration

### Universal Docker Configuration (Any IDE/CLI)

To use this MCP server with any client (**Claude Code**, **OpenCode**, **Cline**, etc.), use the following Docker command structure.

**Key Requirements:**
1.  **Memory Volume**: `-v mcp-data:/data` (Persists your graph, embeddings, **and cached model weights**)
2.  **Project Volume**: `-v $(pwd):/project:ro` (Allows the server to read and index your code)
3.  **Init Process**: `--init` (Ensures the server shuts down cleanly)
4.  **HTTP Server-Visible Paths**: When running in HTTP mode (default), the server can only index paths that are mounted into the container or otherwise visible to the server process. Local paths on the client machine are not automatically accessible.

> [!TIP]
> **One volume persists everything**: The single `-v mcp-data:/data` mount covers both the SurrealDB database **and** the ~1.2 GB embedding model (stored under `/data/models/`). There is no need for a separate volume for `/data/models` — it is already a subdirectory of `/data` and is preserved automatically. Without a named volume, Docker creates a new anonymous volume on each `docker run`, causing the model to re-download (~1.2 GB) every time.

#### HTTP Mode (Default / Remote)

When using the server over HTTP (e.g., for a remote agent or a containerized backend), ensure your project files are mounted into the container so the server can index them.

**Important Security & Scope Notes:**
1. **Server-Visible Paths**: The server process must be able to see the paths you request to index. You must mount the project directory and specify its location using `PROJECT_PATH` or `--project-path`.
2. **No Remote Uploads**: The server does not support uploading client-local files or indexing client-side paths over the network. All indexed code must be visible to the server's local filesystem (or mounted volume).
3. **No Header/Query Binding**: Project binding is currently only supported via explicit `project_info` tool actions. Binding via HTTP headers or query parameters is not supported.

```bash
docker run -d \
  --name memory-mcp \
  --memory=3g \
  -p 8080:8080 \
  -v mcp-data:/data \
  -v /absolute/path/to/host/project:/project:ro \
  -e PROJECT_PATH=/project \
  ghcr.io/pomazanbohdan/memory-mcp-1file:latest
```

> [!IMPORTANT]
> **HTTP Server-Visible Paths**: The server process must be able to see the paths you request to index. When running in Docker, you must mount the project directory and specify its location using `PROJECT_PATH` or `--project-path`.

#### Code Intelligence Startup Behavior

The server uses a deterministic priority matrix to establish the primary project root for code intelligence:

1.  **Explicit Configuration**: If `--project-path` or `PROJECT_PATH` is set and the path exists, it is used as the primary root.
2.  **Missing Root**: If a path is explicitly configured but missing, the server continues startup but reports **missing-root** or **degraded** diagnostics. It will **not** silently fall back to other paths.
3.  **Default Fallback**: If no path is configured and `/project` exists, the server uses `/project` and preserves the legacy `project_id="project"` for compatibility.
4.  **Disabled**: If no path is configured and `/project` is missing, code intelligence is disabled (reported in diagnostics), but the server remains functional for other memory tools.

#### JSON Configuration (Claude Desktop, etc.)

Add this to your configuration file (e.g., `claude_desktop_config.json`):

```json
{
  "mcpServers": {
    "memory": {
      "command": "docker",
      "args": [
        "run",
        "--init",
        "-i",
        "--rm",
        "--memory=3g",
        "-v", "mcp-data:/data",
        "-v", "/absolute/path/to/your/project:/project:ro",
        "ghcr.io/pomazanbohdan/memory-mcp-1file:latest",
        "--stdio"
      ]
    }
  }
}
```

> **Note:** Replace `/absolute/path/to/your/project` with the actual path you want to index. In some environments (like Cursor or VSCode extensions), you might be able to use variables like `${workspaceFolder}`, but absolute paths are most reliable for Docker.

### Cursor (Specific Instructions)

1.  Go to **Cursor Settings** > **Features** > **MCP Servers**.
2.  Click **+ Add New MCP Server**.
3.  **Type**: `stdio`
4.  **Name**: `memory`
5.  **Command**:
    ```bash
    docker run --init -i --rm --memory=3g -v mcp-data:/data -v "/Users/yourname/projects/current:/project:ro" ghcr.io/pomazanbohdan/memory-mcp-1file:latest --stdio
    ```
    *(Remember to update the project path when switching workspaces if you need code indexing)*

### OpenCode / CLI

```bash
docker run --init -i --rm --memory=3g \
  -v mcp-data:/data \
  -v $(pwd):/project:ro \
  ghcr.io/pomazanbohdan/memory-mcp-1file:latest \
  --stdio
```

> [!NOTE]
> The published Docker image defaults to **HTTP SSE** mode for standalone/server use. When wiring it into MCP desktop or CLI clients, append `--stdio` as shown above so the container speaks the stdio transport the client expects.

### NPX / Bunx (No Docker required)

You can run the server directly via `npx` or `bunx`. The npm package automatically downloads the correct pre-compiled binary for your platform.

#### Claude Desktop

Add to `claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "memory": {
      "command": "npx",
      "args": ["-y", "memory-mcp-1file"]
    }
  }
}
```

#### Claude Code (CLI)

```bash
claude mcp add memory -- npx -y memory-mcp-1file
```

#### Cursor

1.  Go to **Cursor Settings** > **Features** > **MCP Servers**.
2.  Click **+ Add New MCP Server**.
3.  **Type**: `command`
4.  **Name**: `memory`
5.  **Command**: `npx -y memory-mcp-1file`

Or add to `.cursor/mcp.json`:

```json
{
  "mcpServers": {
    "memory": {
      "command": "npx",
      "args": ["-y", "memory-mcp-1file"]
    }
  }
}
```

#### Windsurf / VS Code

Add to your MCP settings:

```json
{
  "mcpServers": {
    "memory": {
      "command": "npx",
      "args": ["-y", "memory-mcp-1file"]
    }
  }
}
```

#### Bun

```json
{
  "mcpServers": {
    "memory": {
      "command": "bunx",
      "args": ["memory-mcp-1file"]
    }
  }
}
```

> **Note:** Unlike Docker, `npx`/`bunx` runs the binary **locally** — it already has access to your filesystem, so no directory mounting is needed. To customize the data storage path, pass `--data-dir` via args:
> ```json
> "args": ["-y", "memory-mcp-1file", "--", "--data-dir", "/path/to/data"]
> ```

### Gemini CLI

Add to your `~/.gemini/settings.json`:

```json
{
  "mcpServers": {
    "memory": {
      "command": "npx",
      "args": ["-y", "memory-mcp-1file"]
    }
  }
}
```

Or with Docker:

```json
{
  "mcpServers": {
    "memory": {
      "command": "docker",
      "args": [
        "run", "--init", "-i", "--rm", "--memory=3g",
        "-v", "mcp-data:/data",
        "-v", "${workspaceFolder}:/project:ro",
        "ghcr.io/pomazanbohdan/memory-mcp-1file:latest",
        "--stdio"
      ]
    }
  }
}
```

---

## ✨ Key Features

- **Session-Scoped Code Scoping**: The server now supports binding an HTTP MCP session to a specific project. Once bound, code-intelligence tools (`recall_code`, `search_symbols`) automatically scope their operations to that project unless an explicit `project_id` is provided. This state is stored in process memory only and does not survive server restarts.
- **Governed Memory Retrieval**: Memory APIs now share first-class optional filters for `user_id`, `agent_id`, `run_id`, `namespace`, `memory_type`, metadata, and time windows. `list_memories` uses the same governance path and returns a filtered `total`.
- **Memory Lexical Engine**: Memory BM25-style retrieval now uses a reusable in-memory lexical index that is warmed from DB at startup and kept in sync by memory CRUD / invalidation flows, instead of rebuilding the lexical model on every request.
- **Layered Diagnostics**: Memory search/recall diagnostics expose retrieved candidates, post-filter hits, and returned hits; `metadata_filter` is explicitly reported as post-query subset matching.
- **Importance-aware Recall**: `importance_score` participates in memory ranking, so promoted memories can outrank equally matching low-priority ones.
- **Replacement Links Preserved**: `invalidate(..., superseded_by=...)` now round-trips on reads, so replacement chains survive retrieval and inspection.
- **Consolidation Preview**: `preview_consolidate_memory` shows exact-duplicate matches, replacement scope, and supersede reason before any write occurs.
- **Graph Memory**: Tracks entities (`User`, `Project`, `Tech`) and their relations (`uses`, `likes`). Supports PageRank-based traversal.
- **Code Intelligence**: Indexes local project directories (AST-based chunking) for Rust, Python, TypeScript, JavaScript, Go, Java, and **Dart/Flutter**. Tracks **calls, imports, extends, implements, and mixin** relationships between symbols.
- **Deterministic Code Scoping**: Code intelligence tools use a strict project resolution order:
  1. **Explicit `project_id`**: Always takes highest priority if provided in the tool arguments.
  2. **Session Binding**: If no explicit `project_id` is provided, the server uses the project bound to the current HTTP MCP session.
  3. **Breadth Fallback**: If neither an explicit ID nor a session binding exists, the server performs a cross-project search (if `project_id=None` behavior is supported by the tool).
  *Note: Stale session bindings (e.g., project deleted) return empty success with `reason_code="stale"` and `binding_state="stale_binding"` without broadening to cross-project search.*
- **Plugin-Facing Contract Freeze**: Code/project read surfaces expose additive `contract` + `summary` metadata with a machine-readable `reason_code` taxonomy (`missing`, `stale`, `partial`, `degraded`, `invalid_locator`, `generation_mismatch`, `unsupported`) while preserving legacy string `reason` fields for compatibility.
- **Explicit Projection Locator Lifecycle**: `project_info(action="projection")` returns an ephemeral locator record with typed lifecycle and lookup metadata, and `project_info(action="projection_by_locator")` returns the same contract on resolve/miss without promoting locators to stable public IDs.
- **Temporal Validity**: Memories can have `valid_from` and `valid_until` dates.
- **SurrealDB Backend**: Fast, embedded, single-file database.

---

## 🛠️ Tools Available

The server exposes **21 tools** to the AI model, organized into logical categories.

### 🧠 Core Memory Management
| Tool | Description |
|------|-------------|
| `store_memory` | Store a new memory with content, optional scope fields, metadata, and optional `importance_score`. Read/list surfaces now also expose additive `contract` + `summary` metadata. |
| `update_memory` | Update memory fields, including scope and `importance_score`. |
| `delete_memory` | Delete memory by ID. |
| `consolidate_memory` | Store a new memory and explicitly supersede exact duplicates within the same optional scope/type boundary. |
| `preview_consolidate_memory` | Preview exact-duplicate consolidation within the same optional scope/type boundary without writing any changes. |
| `list_memories` | List memories (newest first) with optional scope/type/metadata/time filters; `total` is the filtered total. Also returns additive `contract` + normalized `summary` metadata. |
| `get_memory` | Get full memory by ID. Memory IDs are stable public identities; response includes additive contract and summary metadata. |
| `invalidate` | Soft-delete memory, optionally linking replacement via `superseded_by`. |
| `get_valid` | Get valid memories. Supports optional timestamp (ISO 8601), scope filters, memory_type, metadata_filter, and event/ingestion ranges. Response includes additive contract and summary metadata. |

### 🔎 Search & Retrieval
| Tool | Description |
|------|-------------|
| `recall` | Hybrid memory retrieval (vector+BM25+graph via RRF) with additive diagnostics and contract metadata. |
| `search_memory` | Memory search (`query`, optional `mode`: `vector` or `bm25`) with optional filters and additive contract/summary metadata. |

### 🕸️ Knowledge Graph
| Tool | Description |
|------|-------------|
| `knowledge_graph` | Knowledge graph ops. Actions: create_entity(name, entity_type?, description?) \| create_relation(from_entity, to_entity, relation_type, weight?) \| get_related(entity_id, depth?, direction?) \| detect_communities(). get_related returns preferred exported nodes/edges plus additive contract and summary metadata; raw entities/relations remain compatibility fields. |

### 💻 Codebase Intelligence
| Tool | Description |
|------|-------------|
| `index_project` | Index codebase directory for code search. |
| `delete_project` | Delete indexed project. |
| `recall_code` | Hybrid code retrieval (vector+BM25+graph) with additive `contract`/`summary` metadata. `results[].id` is a local chunk-record reference; stable refind locator is `project_id + file_path + start_line + end_line`. |
| `search_symbols` | Symbol lookup by name with additive contract/summary metadata. |
| `symbol_graph` | Symbol relationship traversal with additive contract/summary metadata; `frontier` is an unexpanded boundary hint, not a cursor. |
| `project_info` | Project indexing information. Actions: list() \| status(project_id) \| stats(project_id) \| projection(project_id) \| projection_by_locator() \| bind(project_id) \| unbind() \| binding_status(). bind/unbind/status actions manage session-scoped project binding for HTTP MCP clients. Responses include additive contract/summary metadata, including lifecycle, generation, and projection/materialization fields. |

### Contract compatibility notes for plugin / MCP integrators

- `contract` and `summary` remain **additive-first** surfaces. Clients must ignore unknown fields and unknown enum values.
- `summary.partial.reason_code` is the canonical machine-readable contract reason. Current Phase 5A values are: `missing`, `stale`, `partial`, `degraded`, `invalid_locator`, `generation_mismatch`, and `unsupported`.
- `summary.partial.reason` is retained as a legacy compatibility string. Existing values like `projection_stale`, `indexing_in_progress`, and `progress:NN.N` remain readable, but new integrations should key off `reason_code`.
- `project_info(action="list")` discovers projects from the union of index status metadata, code chunks, code symbols, and file manifests, so partially indexed or degraded projects remain operator-visible.
- `project_info(action="stats")` returns degraded diagnostics when code intelligence rows exist but `index_status` metadata is missing; it only returns `Project not found` when no status, chunks, symbols, or manifest entries exist for that project.
- `project_info(action="projection")` returns `locator.lookup.state = "created"`; `project_info(action="projection_by_locator")` returns `locator.lookup.state = "resolved"` on success and `"missing"` on miss.
- **Session-Bound Project Resolution**:
  - `project_info(action="bind", project_id="...")` binds the current HTTP MCP session to a project.
  - `project_info(action="unbind")` clears the binding.
  - `project_info(action="binding_status")` returns the current binding or `null`.
  - **Lifetime**: Binding is keyed by the HTTP MCP `mcp-session-id`, stored in process memory only, and does not survive server restart.
  - **Transport Support**: Stdio mode returns `reason_code="unsupported"` for binding actions as it lacks session context.
  - **Auto-binding**: `index_project` does not automatically bind the session; use an explicit `bind` action after indexing if session-scoping is desired.
  - **Diagnostics**: If a session-bound project is deleted or becomes unavailable, tools return a success JSON with empty results, `project_resolution.source="session_binding"`, `reason_code="stale"`, and `binding_state="stale_binding"`. No cross-project fallback occurs for stale bindings.
- Projection locators are **opaque, same-process, non-persistable, and not generation-stable**. They are convenience handles for immediate readback, not stable public identities.
- Stable identities remain unchanged:
  - memory read/list/search surfaces → public memory IDs
  - symbol graph/search surfaces → stable project-scoped symbol IDs
  - `recall_code` re-find contract → `project_id + file_path + start_line + end_line`

### ⚙️ System & Maintenance
| Tool | Description |
|------|-------------|
| `get_status` | Get system status and startup progress. |
| `reset_all_memory` | **DANGER**: Reset all database data (requires `confirm=true`). |
| `how_to_use` | Meta-help tool for concise MCP tool-surface guidance. |


---

## ⚙️ Configuration

Environment variables or CLI args:

| Arg | Env | Default | Description |
|-----|-----|---------|-------------|
| `--data-dir` | `DATA_DIR` | platform-local app data dir (`memory-mcp`) | DB location |
| `--model` | `EMBEDDING_MODEL` | `gemma` | Embedding model (`qwen3`, `gemma`, `bge_m3`, `nomic`, `e5_multi`, `e5_small`) |
| `--mrl-dim` | `MRL_DIM` | *(native)* | Output dimension for MRL-supported models (e.g. 64, 128, 256, 512, 1024 for Qwen3). Defaults to the model's native maximum dimension (1024 for Qwen3). |
| `--batch-size` | `BATCH_SIZE` | `8` | Maximum batch size for embedding inference |
| `--cache-size` | `CACHE_SIZE` | `1000` | LRU cache capacity for embeddings |
| `--timeout` | `TIMEOUT_MS` | `30000` | Timeout in milliseconds |
| `--idle-timeout` | `IDLE_TIMEOUT` | `0` | Idle timeout in minutes. 0 = disabled |
| `--log-level` | `LOG_LEVEL` | `info` | Verbosity |
| `--log-file` | `LOG_FILE` | *(None)* | Log file path. If specified, logs will be written to this file in addition to stderr. The file will be rotated when it reaches the maximum size. Rotated files are named with startup timestamp (e.g., `app.2026-04-09_14-30-00.log.1`). |
| `--log-file-max-size-mb` | `LOG_FILE_MAX_SIZE_MB` | `10` | Maximum log file size in MB before rotation. Only effective when `--log-file` is specified. |
| *(None)* | `HF_TOKEN` | *(None)* | Optional HuggingFace token for private/rate-limited model downloads |
| *(None)* | `EMBEDDING_QUEUE_CAPACITY` | `256` | Max size of the background embedding queue |
| *(None)* | `EMBEDDING_BATCH_SIZE` | `8` | How many files to process in one embedding chunk |
| *(None)* | `INDEX_BATCH_SIZE` | `20` | How many files to process in one incremental chunk |
| *(None)* | `INDEX_DEBOUNCE_MS` | `2000` | MS to wait before flushing index events (debounce) |
| *(None)* | `MANIFEST_DIFF_INTERVAL_MINS` | `10` | Minutes between periodic missing file checks |
| `--project-path` | `PROJECT_PATH` | *(None)* | Primary project root for code intelligence. Fallback is `/project`. |
| `--allowed-project-roots` | `ALLOWED_PROJECT_ROOTS` | *(None)* | Optional comma-delimited allowlist for server-visible project roots. When set, startup/manual registration rejects roots outside this allowlist with `reason_code=path_not_allowed`. |
| `--max-managed-projects` | `MAX_MANAGED_PROJECTS` | `5` | Maximum number of managed lifecycle projects in registry. Additional registrations are rejected with `reason_code=max_project_limit`. |

### 🧠 Available Models

You can switch the embedding model using the `--model` arg or `EMBEDDING_MODEL` env var.

| Argument Value | HuggingFace Repo | Dimensions | Size | Use Case |
| :--- | :--- | :--- | :--- | :--- |
| `qwen3` | `Qwen/Qwen3-Embedding-0.6B` | 1024 (MRL) | 1.2 GB | Highest-quality bundled option. Larger download and storage footprint. |
| `gemma` | `unsloth/embeddinggemma-300m-qat-q4_0-unquantized` | 768 (MRL) | ~195 MB | **Default**. Smaller download, lower RAM, good Docker-friendly baseline. |
| `bge_m3` | `BAAI/bge-m3` | 1024 | 2.3 GB | State-of-the-art multilingual hybrid retrieval. Heavy. |
| `nomic` | `nomic-ai/nomic-embed-text-v1.5` | 768 | 1.9 GB | High quality long-context BERT-compatible. |
| `e5_multi` | `intfloat/multilingual-e5-base` | 768 | 1.1 GB | Legacy; kept for backward compatibility. |
| `e5_small` | `intfloat/multilingual-e5-small` | 384 | 134 MB | Fastest, minimal RAM. Good for dev/testing. |

### 📉 Matryoshka Representation Learning (MRL)

Models marked with **(MRL)** support dynamically truncating the output embedding vector to a smaller dimension (e.g., 512, 256, 128) with minimal loss of accuracy. This saves database storage and speeds up vector search.

Use the `--mrl-dim` argument to specify the desired size. If omitted, the default is the model's native base dimension (e.g., 1024 for Qwen3).

**Warning:** Once your database is created with a specific dimension, you cannot change it without wiping the data directory.

### 📦 Model Selection Notes

By default, the server uses **Gemma** because it is the lightest bundled model and starts more comfortably in Docker-sized environments.

To use Gemma explicitly:

```bash
memory-mcp --model gemma
```

Gemma currently works out of the box with the bundled downloader. `HF_TOKEN` is still optional and can help with higher rate limits or private/rate-limited HuggingFace access, but the current server code does not require any separate Gemma-specific license-acceptance flow.

If you want the highest-quality bundled model instead, switch to **Qwen3** explicitly:

```bash
memory-mcp --model qwen3
```

When running in Docker, remember that changing models also changes embedding dimensions and storage requirements. Reuse the same `/data` volume only when the stored data was created with the same model/dimension settings.

### 🐳 Docker Image Notes

- The published image defaults to **HTTP SSE** on port `8080` and binds to `0.0.0.0`, so `-p 8080:8080` works as expected.
- MCP desktop/CLI integrations should append `--stdio`, because those clients speak stdio rather than HTTP.
- The release pipeline now publishes both **linux/amd64** (`x86_64-unknown-linux-musl`) and **linux/arm64** (`aarch64-unknown-linux-musl`) artifacts, and the published container image resolves the correct binary per target architecture.

> [!WARNING]
> **Changing Models & Data Compatibility**
>
> If you switch to a model with different dimensions (e.g., from `e5_small` to `e5_multi`), **your existing database will be incompatible**.
> You must delete the data directory (volume) and re-index your data.
>
> Switching between models with the same dimensions (e.g., `e5_multi` <-> `nomic`) is theoretically possible but not recommended as semantic spaces differ.

## 🔮 Future Roadmap (Research & Ideas)

### Current roadmap status
- ✅ **Phase 0 — Baseline Foundations** complete
- ✅ **Phase 1 — Canonical Contract Foundation** complete
- ✅ **Phase 2 — Public Surface Normalization** complete
- ✅ **Phase 3 — Later-Phase Contract Freeze + MVP Preparation** effectively complete for the MCP server repo
- ✅ **Phase 4 — Projection Builder (non-plugin scope)** effectively complete for the MCP server repo, including:
  - export-only on-demand projection build via `project_info(action="projection")`
  - deterministic builder flow and request/options contract
  - shaping semantics
  - ephemeral same-process locator + read-back path
- ⏸️ **Phase 5 — Plugin-facing Workflow Integration** is intentionally **out of scope for this repository** unless future work explicitly chooses to implement plugin-side workflow assets here.

### Repository closure status

From the MCP server repository perspective, the remaining work is now **closure and handoff**, not major new server capability work:

- the public contract layer (`contract` + `summary`) is already in place across memory, graph, code search, symbol, and project surfaces;
- projection/materialization semantics are explicit, but still truthfully non-persistent and non-addressable beyond same-process ephemeral locator read-back;
- stable vs transient identity rules are already frozen and documented;
- plugin orchestration, cache policy, stale UX, retry policy, and workflow commands are expected to live **outside this repo**.

See also:
- [`ARCHITECTURE.md`](./ARCHITECTURE.md) — plugin-facing MCP contract notes
- `SERVER_PLUGIN_BOUNDARY_STATUS.md` — final repo-side closure and handoff status
- `PLUGIN_IMPLEMENTATION_PLAN.md` — detailed plugin-side implementation plan

Based on analysis of advanced memory systems like [Hindsight](https://hindsight.vectorize.io/) (see their documentation for details on these mechanisms), we are exploring these "Cognitive Architecture" features for future releases:

### 1. Meta-Cognitive Reflection (Consolidation)
*   **Problem:** Raw memories accumulate noise over time (e.g., 10 separate memories about fixing the same bug).
*   **Solution:** Implement a `reflect` background process (or tool) that periodicallly scans recent memories to:
    *   **De-duplicate** redundant entries.
    *   **Resolve conflicts** (if two memories contradict, keep the newer one or flag for review).
    *   **Synthesize** low-level facts into high-level "Insights" (e.g., "User prefers Rust over Python" derived from 5 code choices).

### 2. Temporal Decay & "Presence"
*   **Problem:** Old memories can sometimes drown out current context in semantic search.
*   **Solution:** Integrate **Time Decay** into the Reciprocal Rank Fusion (RRF) algorithm.
    *   Give a calculated boost to recent memories for queries implying "current state".
    *   Allow the agent to prioritize "working memory" over "historical archives" dynamically.

### 3. Namespaced Memory Banks
*   **Problem:** Running one docker container per project is resource-heavy.
*   **Solution:** Add support for `namespace` or `project_id` scoping.
    *   Allows a single server instance to host isolated "Memory Banks" for different projects or agent personas.
    *   Enables "Switching Context" without restarting the container.

### 4. Epistemic Confidence Scoring
*   **Problem:** The agent treats a guess the same as a verified fact.
*   **Solution:** Add a `confidence` score (0.0 - 1.0) to memory schemas.
    *   Allows storing hypotheses ("I think the bug is in auth.rs", confidence: 0.3).
    *   Retrieval tools can filter out low-confidence memories when answering factual questions.

---

## 🔍 Troubleshooting

### Empty `recall_code` or `search_symbols` results
If you receive empty results when searching code, check the following:

1.  **Mount Path**: Ensure your project directory is correctly mounted into the container (e.g., `-v /host/path:/project:ro`).
2.  **Project Root Configuration**: Verify `PROJECT_PATH` or `--project-path` matches the mounted path inside the container.
3.  **Indexing Status**: Run `project_info(action="stats", project_id="...")` to check if indexing is still in progress or if there are errors.
4.  **Diagnostic Info**: Use `project_info(action="list")` to see which projects the server has discovered and their current state (e.g., `degraded`, `missing`).
5.  **Server-Visible Paths**: Remember that HTTP clients must provide paths that are **visible to the server**. The server cannot access paths that exist only on your local client machine unless they are mounted.

## License

MIT
