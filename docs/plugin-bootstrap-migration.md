# Plugin Bootstrap Migration Guide

This document is for plugin-side migration planning. The server-side tools are available in this repository; no plugin code change is required in the same release.

## Target Contract

Use the server as the owner of common memory orchestration:

| Plugin need | Server tool | Notes |
|---|---|---|
| Startup context | `memory_bootstrap` | Preferred first call for session start, compact recovery, and manual continue flows. |
| Hook evidence capture | `memory_observation_create` | Replaces direct `store_memory` for automatic hook/session observations. |
| Debug lifecycle view | `memory_audit` | Read-only summary for viewer/debug UI and support bundles. |
| Explain retrieval | `memory_search_trace` | Read-only explanation path; delegates ranking to existing recall/search implementation. |

All four responses include additive `contract` and `summary`. Plugin code should ignore unknown fields and prefer `summary.partial.reason_code` over legacy free-form text.

## Recommended `memory_bootstrap` Timing

Call `memory_bootstrap` when the plugin would otherwise combine `recall`, `search_memory`, `list_memories`, and local recovery parsing:

1. New MCP session starts.
2. Context was compacted and the plugin has a compact summary.
3. User asks to continue/resume a task.
4. Plugin detects a stale local startup cache and needs a fresh server view.

Suggested arguments:

```json
{
  "prompt": "<current user request or recovered command>",
  "compact_summary": "<optional compact summary>",
  "namespace": "<project/workspace namespace>",
  "user_id": "<optional user id>",
  "agent_id": "<optional agent id>",
  "run_id": "<optional run id>",
  "project_id": "<optional code project id>",
  "limit": 10,
  "token_budget": 4000
}
```

Consume these fields first:

- `active_tasks`: candidate `TASK:` records, with in-progress/recent tasks prioritized.
- `stable_context`: grouped `DECISION:`, `USER:`, `RESEARCH:`, `PROJECT:`, `CONTEXT:`, and `EPIC:` records.
- `recovery`: operational details useful after compaction, filtered against `compact_summary` when supplied.
- `project`: code-index readiness summary.
- `memory_health`: GC backlog, learning memory readiness, and partial/degraded signals.
- `selection_summary`: returned counts, token estimate, and truncation reason.

## Hook Timeout Fallback Order

For Codex/OpenCode hooks with tight timeout budgets:

1. Try `memory_bootstrap` with a small `limit` and `token_budget`.
2. If the tool is missing or returns `summary.partial.reason_code="unsupported"`, fall back to existing plugin logic:
   - exact prefix BM25 search for `TASK:`;
   - `recall` for relevant stable context;
   - `project_info(action="binding_status")` or `project_info(action="status")` for project readiness.
3. If startup budget is nearly exhausted, skip non-critical stable context and only show active task candidates.
4. Cache only the rendered decision for the current hook cycle; do not treat plugin-side fallback output as a new server contract.

Timeout guidance:

- Short hook: call `memory_bootstrap` with `limit=5`, `token_budget=1500`.
- Normal session start: call `memory_bootstrap` with `limit=10`, `token_budget=4000`.
- Manual debug: increase `limit`, then inspect `selection_summary` for truncation.

## Replacing Auto-Capture Writes

Plugin auto-capture should move from direct `store_memory` to `memory_observation_create` for event evidence.

Old shape:

```json
{
  "tool": "store_memory",
  "arguments": {
    "content": "User resumed task from compact summary",
    "memory_type": "episodic",
    "namespace": "workspace"
  }
}
```

New shape:

```json
{
  "tool": "memory_observation_create",
  "arguments": {
    "content": "User resumed task from compact summary",
    "source": "codex-hook",
    "event_type": "compact_recovery",
    "namespace": "workspace",
    "run_id": "run-123",
    "confidence": 0.8,
    "redaction_state": "redacted",
    "metadata": {
      "hook": "session_start"
    }
  }
}
```

Server behavior:

- If `content` does not start with a legal prefix, the server stores it as `CONTEXT: <content>`.
- Legal prefixes remain: `PROJECT:`, `EPIC:`, `TASK:`, `RESEARCH:`, `DECISION:`, `CONTEXT:`, `USER:`.
- No `OBSERVATION:` prefix is created.
- `metadata.observation.created_from` is always `memory_observation_create`.
- The write does not promote to learning memory, consolidate duplicates, or replace existing memory.

Promotion remains a separate explicit step through `learning_memory_*`.

## Unsupported Old Server Strategy

Plugins should detect tool availability from `list_tools`.

If any new tool is missing:

- `memory_bootstrap` missing: fall back to existing startup composition.
- `memory_observation_create` missing: fall back to `store_memory`, but add a legal prefix in plugin code before writing.
- `memory_audit` missing: hide audit UI or show a degraded diagnostic instead of reconstructing purge state from raw lists.
- `memory_search_trace` missing: show regular `recall`/`search_memory` results without rank explanation.

Do not polyfill server response shapes in the plugin under the same field names unless the plugin marks them as local fallback data. The stable contract belongs to the server.

## Migration Checklist

1. Add `list_tools` capability detection for the four new tools.
2. Replace startup recall/list composition with `memory_bootstrap`.
3. Replace hook auto-capture `store_memory` calls with `memory_observation_create`.
4. Update debug/viewer surfaces to use `memory_audit`.
5. Add an optional "why this result" action backed by `memory_search_trace`.
6. Keep legacy fallback paths until the minimum supported server version includes these tools.
7. Treat `summary.partial.reason_code` as the machine-readable branch key.

## Non-Goals

- No plugin-side code changes are required by this server release.
- Observations are not learning conclusions.
- The server does not add a new memory table or `OBSERVATION:` prefix.
- The new read tools do not change `recall`, `search_memory`, `get_valid`, `learning_memory_*`, purge, or consolidation semantics.
