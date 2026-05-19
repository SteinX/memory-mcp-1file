# Tools Reference

This page groups the MCP tool surface by job. For exact request and response
schemas, inspect `list_tools` from your MCP client.

## Memory

| Tool | Purpose |
|---|---|
| `memory_bootstrap` | Read-only session bootstrap: active tasks, stable context, recovery details, project readiness, memory health, and diagnostics. |
| `store_memory` | Store a new memory. |
| `memory_observation_create` | Store observation evidence from plugin/session hooks without auto-promotion or silent overwrite. |
| `get_memory` | Fetch one memory by stable public ID. |
| `update_memory` | Update memory fields. |
| `list_memories` | List memories newest-first with optional filters. |
| `get_valid` | List currently valid memories, optionally at a timestamp. |
| `invalidate` | Soft-delete a memory and optionally link a replacement. |
| `export_memory` | Export memories as JSONL using the migration contract. |
| `import_memory` | Import memories from inline JSONL. |

Memory IDs are stable public identities. Responses include additive `contract`
and `summary` metadata for compatibility and lifecycle diagnostics.

## Search

| Tool | Purpose |
|---|---|
| `recall` | Hybrid memory retrieval through vector, BM25, and graph RRF fusion. |
| `search_memory` | Memory search with `mode: vector` or `mode: bm25`. |
| `memory_search_trace` | Read-only explanation for effective query, result IDs, match channels, lifecycle visibility, and rank reasoning. |
| `recall_code` | Hybrid code retrieval. |
| `search_symbols` | Fast by-name code symbol lookup. |
| `symbol_graph` | Traverse code relationships for a symbol. |

## Lifecycle And Cleanup

Routine cleanup should be explicit and auditable:

```text
invalidate -> preview_purge_memory -> purge_memory
```

| Tool | Purpose |
|---|---|
| `consolidate_memory` | Store a new memory and explicitly supersede exact duplicates. |
| `preview_consolidate_memory` | Preview exact-duplicate consolidation without writing. |
| `preview_purge_memory` | Preview invalidated memories eligible for physical purge. |
| `purge_memory` | Physically purge invalidated memories selected by a prior preview fingerprint. |
| `delete_memory` | Emergency/admin single-record hard deletion. Requires explicit human confirmation. |

`purge_memory` requires the `plan_fingerprint` returned by
`preview_purge_memory`. This prevents stale or broadened cleanup plans from
silently deleting data.

## Learning Memory

Learning memory is a typed wrapper over the regular Memory protocol. It stores
structured learning metadata such as kind, status, confidence, scope, and
evidence.

| Tool | Purpose |
|---|---|
| `learning_memory_create` | Create a learning memory record. |
| `learning_memory_get` | Fetch a learning memory by ID. |
| `learning_memory_list` | List learning memories with status/scope filters. |
| `learning_memory_search` | Search learning memories, defaulting to confirmed/rule records. |
| `learning_memory_update` | Update content or metadata. |
| `learning_memory_promote` | Promote candidate to confirmed or confirmed to rule. |
| `learning_memory_reject` | Reject and invalidate a learning record. |
| `learning_memory_archive` | Archive and invalidate a learning record. |
| `learning_memory_supersede` | Supersede one learning record with a replacement. |
| `learning_memory_migrate_legacy` | Convert prefix-based legacy records to learning metadata. |
| `learning_memory_delete` | Compatibility shim; performs soft reject/archive/invalidate by default. |

## Knowledge Graph

The `knowledge_graph` tool multiplexes these actions:

| Action | Purpose |
|---|---|
| `create_entity` | Create a graph entity. |
| `create_relation` | Link two entity IDs. |
| `get_related` | Traverse related nodes and edges. |
| `detect_communities` | Detect graph communities. |

`create_relation` expects entity IDs returned by `create_entity`, not display
names.

## Code Project

| Tool | Purpose |
|---|---|
| `index_project` | Index a codebase directory. |
| `project_info` | Project list/index/status/stats/projection/binding actions. |
| `delete_project` | Delete an indexed project. |

`project_info` is the preferred project lifecycle surface because it returns
normalized contract and summary metadata.

## System

| Tool | Purpose |
|---|---|
| `get_status` | System status and startup progress. |
| `memory_audit` | Read-only lifecycle, purge-readiness, observation, and operator attention summary. |
| `how_to_use` | Concise server-provided usage guide. |
| `reset_all_memory` | Destructive admin reset. Requires `confirm=true` and explicit operator approval. |

## Migration Contract

`export_memory` emits JSONL records under a stable migration contract. The
contract preserves stable memory identity, content, metadata, scope, lifecycle
state, and timestamps.

`import_memory` supports conflict-safe import. Unsupported schema versions are
reported as structured errors instead of partial silent import.

Use migration tools for backup, handoff, and cross-instance movement. Do not use
hard deletion as a migration cleanup step; invalidate or purge through the
preview/apply flow instead.

## Contract Notes For Integrators

- Treat `contract` and `summary` as additive stable metadata.
- Inspect `summary.partial.reason_code` before deciding an operation failed.
- Prefer stable public IDs for memories.
- For code chunks, `results[].id` is a local chunk-record reference. Stable
  refind locator is `project_id + file_path + start_line + end_line`.
- For graph and projection responses, exported nodes/edges are preferred over
  raw compatibility fields.
- `frontier` is an unexpanded boundary hint, not a cursor.
