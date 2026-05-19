# Troubleshooting

## Empty `recall_code` Or `search_symbols` Results

Check these first:

1. Project files are mounted or otherwise visible to the server.
2. `PROJECT_PATH` or `--project-path` points to the server-visible path.
3. `project_info(action="stats", project_id="...")` shows indexing progress or
   errors.
4. `project_info(action="list")` shows the discovered projects and lifecycle
   state.
5. HTTP clients are not passing client-local paths that the server cannot see.

## `already_running` From `index_project`

A same-project full-index request returns `already_running` when another task
for the same project is still active. This prevents a second run from clearing
chunks and symbols while the first run is writing them.

Wait for the active task:

```json
{
  "action": "status",
  "project_id": "project"
}
```

Retry after `state` is no longer `indexing` or `in_progress`.

If status is stuck and you are certain no task is running, override with:

```json
{
  "path": "/project",
  "force": true,
  "confirm_failed_restart": true
}
```

## Interrupted Or Lost One-Shot Indexing Task

If the process restarts during a full-index task, the task itself is lost. On
the next `project_info(action="status")`, you may see:

- `background_task.state = "unknown_after_restart"` when restart happened after
  initial status persisted.
- `background_task.state = "failed"`, `retryable = true`,
  `reason_code = "lost_one_shot_indexing_task_after_restart"`, and
  `background_task.phase = "before_file_enumeration"` when restart happened
  before file enumeration.

Recovery:

```json
{
  "action": "index",
  "path": "/project",
  "force": true,
  "confirm_failed_restart": true
}
```

## Durable Resume Fails

If `project_info(action="status")` returns `can_resume: true` and a
`resume_token`, but resume fails with `checkpoint_generation_missing`, common
causes are:

1. The token was cached and no longer matches the stored checkpoint.
2. `cleanup_abandoned_index_jobs` removed checkpoints between status and resume.
3. A newer full index promoted a new generation.

Read a fresh status before resuming. If resume is no longer possible, force a
fresh full index.

## Job Stuck In `cancel_requested`

Cancellation is cooperative. The indexer polls for cancellation every 10 files.
If the server is killed before that poll runs, the job can remain in
`cancel_requested`.

On startup, recovery transitions it to failed or resumable. Then resume or
force-restart as appropriate.

Cleanup terminal jobs:

```json
{
  "action": "cleanup_abandoned_index_jobs",
  "project_id": "project"
}
```

## BM25 Finalization Failure

If `project_info(action="stats")` reports a BM25 failure after indexing, lexical
code search may be degraded while vector search still works.

Rebuild by running a fresh full index with:

```json
{
  "action": "index",
  "path": "/project",
  "force": true,
  "confirm_failed_restart": true
}
```

The active production strategy is `final_rebuild`, which reconstructs BM25 from
committed chunks.

## Memory Pressure During Indexing

Reduce in-flight bounds:

```bash
CODE_INDEX_MAX_INFLIGHT_FILES=32 CODE_INDEX_MAX_INFLIGHT_BYTES=67108864 memory-mcp
```

The defaults are conservative for a 3 GB Docker container, but the embedding
model adds roughly 200 MB to 1.2 GB depending on model choice. If using `qwen3`
inside a small container, lower `CODE_INDEX_MAX_INFLIGHT_BYTES` or switch to
`gemma`.

## Docker Model Downloads Repeatedly

Use a named `/data` volume:

```bash
docker run --rm -i \
  -v mcp-data:/data \
  ghcr.io/steinx/memory-mcp-1file:latest \
  memory-mcp --data-dir /data --stdio
```

The release image stores model files under `/data/models`, so the named volume
preserves both the database and model cache.
