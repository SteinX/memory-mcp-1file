# Code Intelligence

Memory MCP indexes code with static tree-sitter parsing, BM25, vector
embeddings, and a symbol graph. It is designed to remain usable while indexing
is active.

## Project Root Selection

Startup chooses the primary code root with this priority:

1. If `--project-path` or `PROJECT_PATH` is set and exists, use it.
2. If an explicit path is missing, start the server but report degraded
   diagnostics. Do not silently fall back to another root.
3. If no path is configured and `/project` exists, use `/project` with the
   legacy `project_id="project"`.
4. If no root is available, code intelligence is disabled while memory tools
   continue working.

Startup recovery for interrupted indexing jobs runs after the HTTP/SSE process
starts accepting requests. This keeps MCP initialization and `/health`
responsive even when a large database must be inspected.

## Language Contract

This path is static and tree-sitter-based. It does not require SourceKit,
clangd, Kotlin LSP, Gradle model parsing, Xcode project parsing, or compiler
invocation.

It is syntactic rather than compiler-accurate. It does not provide overload
resolution, macro expansion, dynamic dispatch resolution, build-configuration
awareness, type inference, or compiler-accurate call graphs.

| Language | Status | Contract |
|---|---|---|
| Rust | Supported | AST-based symbol and relation extraction. |
| Python | Supported | AST-based symbol and relation extraction. |
| JavaScript | Supported | AST-based symbol and relation extraction. |
| TypeScript | Supported | AST-based symbol and relation extraction. |
| Go | Supported | AST-based symbol and relation extraction. |
| Java | Supported | AST-based symbol and relation extraction. |
| Dart | Supported | AST-based symbol and relation extraction. |
| C | Supported | Syntactic functions, structs/enums/typedefs, includes, and obvious direct calls. |
| C++ | Supported | Syntactic namespaces, classes/methods/functions, includes, and obvious direct calls. |
| Swift | Supported | Syntactic imports, types, methods/functions, and obvious direct calls. |
| Kotlin | Supported | Syntactic package/imports, classes/interfaces/objects, and methods/functions. |
| Objective-C | Supported | Syntactic imports, classes/protocols, methods, C calls, and obvious message sends. |

For `.h` files, detection is heuristic and ordered as Objective-C markers,
C++ markers, then C fallback.

## Tools

| Tool | Purpose |
|---|---|
| `index_project` | Start or retry code indexing for a project root. |
| `project_info` | List/index/status/stats/projection/bind/unbind project state. |
| `recall_code` | Hybrid code retrieval via vector, BM25, and graph signals. |
| `search_symbols` | Fast by-name symbol lookup. |
| `symbol_graph` | Traverse relationships for a `symbol_id`. |
| `delete_project` | Delete indexed project state. |

`project_info` actions:

```text
list
index
status
stats
projection
projection_by_locator
bind
unbind
binding_status
cancel_index
cleanup_abandoned_index_jobs
```

## During Indexing

Large projects are indexed in the background. Code tools serve the last
successfully promoted generation while a newer generation is building.

| Tool | Indexing-time behavior | Fallback | No serving generation |
|---|---|---|---|
| `project_info` | Always available; reports serving and indexing generation. | N/A | Returns status with `reason_code=missing`. |
| `recall_code` | Serves stale generation. | BM25 / symbol-derived results. | Returns `reason_code=missing`, zero results. |
| `search_symbols` | Serves stale symbol table. | N/A | Returns `reason_code=missing`, zero results. |
| `symbol_graph` | Serves stale graph. | Symbol frontier. | Returns partial metadata. |

For single identifier queries such as `RecallCodeParams`, `recall_code` can use
a BM25 lexical fast path:

```text
summary.fallback_path = "bm25_lexical_fast_path"
```

This intentionally skips embedding, symbol probing, and PPR graph expansion so
exact symbol lookups stay responsive on large persisted indexes.

## Degradation Metadata

Code responses include `summary.partial`:

| Field | Meaning |
|---|---|
| `is_partial` | `true` when data is stale, missing, or degraded. |
| `reason_code` | Machine-readable state: `missing`, `stale`, `partial`, `degraded`, `unsupported`, etc. |
| `reason` | Human-readable explanation. |

Use `reason_code` for client control flow:

- `missing`: no serving data exists yet. Show an empty result and inspect
  `project_info`.
- `stale`: results are from the last promoted generation.
- `degraded`: a fallback served the request. Inspect `capability_readiness`.
- `unsupported`: the requested action is not available in the current transport
  or context.

Returned chunks and symbols may also include `freshness.state`:

| State | Meaning |
|---|---|
| `fresh` | File unchanged since the serving generation. |
| `stale` | File changed since the serving generation. |
| `unknown` | No checkpoint data is available. |

A project-level partial response does not mean every item is stale. Inspect
item-level freshness for freshness-sensitive decisions.

## Indexing Pipeline Configuration

The default pipeline mode is `legacy`. The staged pipeline is available for
validation, but it is not the default.

| Env | Default | Accepted values | Description |
|---|---|---|---|
| `CODE_INDEX_PIPELINE_MODE` | `legacy` | `legacy`, `staged` | Pipeline mode. |
| `CODE_INDEX_READ_WORKERS` | `2` | integer >= 1 | Concurrent file-read workers. |
| `CODE_INDEX_PARSE_WORKERS` | `max(2, min(cpu_count/2, 4))` | integer >= 2 | Concurrent parse workers. |
| `CODE_INDEX_COMMIT_BATCH_SIZE` | `100` | integer >= 1 | Parsed chunks committed per batch. |
| `CODE_INDEX_MAX_INFLIGHT_FILES` | `64` | integer >= 1 | Files allowed in the pipeline at once. |
| `CODE_INDEX_MAX_INFLIGHT_BYTES` | `134217728` | integer >= 1 | Approximate bytes allowed in flight. |
| `CODE_INDEX_STATUS_FLUSH_MS` | `1000` | integer >= 1 | Minimum progress status flush interval. |
| `CODE_INDEX_RELATION_BATCH_SIZE` | `5000` | integer >= 1 | Symbol relations written per batch. |
| `CODE_INDEX_BM25_MODE` | `final_rebuild` | `final_rebuild`, `incremental` | `incremental` is parsed but not production-active. |
| `CODE_INDEX_INCLUDE_PATTERNS` | none | comma-delimited globs | Optional project-relative include patterns. |
| `CODE_INDEX_EXCLUDE_PATTERNS` | none | comma-delimited globs | Optional project-relative exclude patterns. |

Try staged mode:

```bash
CODE_INDEX_PIPELINE_MODE=staged memory-mcp
```

Return to the default:

```bash
CODE_INDEX_PIPELINE_MODE=legacy memory-mcp
```

## Index Filters

Patterns are project-relative and use `/` as the separator on every platform.
Excludes override includes.

Built-in skipped directories include `node_modules`, `target`, `.git`, `build`,
and `dist`.

Generated source skips include common generated directories, Dart generated
suffixes, Swift protobuf files (`*.pb.swift`), and source files whose header
marks them as generated and not intended for editing.

Environment example:

```bash
CODE_INDEX_INCLUDE_PATTERNS=src/**,lib/**
CODE_INDEX_EXCLUDE_PATTERNS=**/*.test.rs,**/generated/**
```

Tool-call example:

```json
{
  "path": "/my/project",
  "include_patterns": ["src/**"],
  "exclude_patterns": ["**/*_test.rs"]
}
```

Invalid glob patterns, such as absolute paths or backslash separators, are
rejected before indexing begins.

## Embedding Backfill

Full indexing publishes BM25, symbols, and graph generations before all chunk
and symbol embeddings have finished. Unembedded rows remain queryable through
lexical and symbol search while the embedding worker catches up.

On startup, persisted `embedding_pending` projects are scanned and unembedded
chunks/symbols are re-queued even when no `--project-path` lifecycle manager is
active. Vector and semantic serving generations are published after HNSW
rebuild succeeds.

## Durable Resume

Manual full-index jobs can be cancelled, interrupted, resumed, or restarted.
`project_info(action="status")` reports:

- current job state;
- `resume_token` when resume is possible;
- checkpoint generation metadata;
- whether full restart fallback is allowed.

When `can_resume: true`, read a fresh `resume_token` immediately before
resuming. Do not cache resume tokens across restarts.

If resume fails with `checkpoint_generation_missing`, common causes are:

1. Token mismatch.
2. Checkpoints were cleaned up between status and resume.
3. A newer full index promoted a newer generation.

Fallback:

```json
{
  "action": "index",
  "path": "/project",
  "force": true,
  "confirm_failed_restart": true
}
```

## Verification Commands

Parser and scanner checks:

```bash
cargo check
cargo test codebase::scanner -- --nocapture
cargo test codebase::parser -- --nocapture
cargo test
```

Indexing subsystem:

```bash
cargo test --lib codebase
```

MCP regression harness:

```bash
python3 scripts/task14_mcp_regression_harness.py
```

Small-tier code retrieval benchmark:

```bash
CODE_INDEX_PIPELINE_MODE=legacy python3 evals/code_retrieval_benchmark.py --tier small
CODE_INDEX_PIPELINE_MODE=staged python3 evals/code_retrieval_benchmark.py --tier small
```

Small-tier evidence is not enough to change the default pipeline mode. Collect
medium/large tier and Docker RSS evidence before making default-change claims.
