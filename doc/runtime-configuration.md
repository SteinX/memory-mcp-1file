# Runtime Configuration

This page covers process-level configuration: data paths, model selection,
metrics, Docker behavior, and release artifacts.

## CLI And Environment Variables

| Arg | Env | Default | Description |
|---|---|---|---|
| `--data-dir` | `DATA_DIR` | platform-local app data dir (`memory-mcp`) | SurrealDB database location. |
| `--model-cache-dir` | `EMBEDDING_MODEL_CACHE_DIR` | native: platform app-data dir (`memory-mcp/models`); container: `--data-dir/models` | Downloaded HuggingFace model files. |
| `--model` | `EMBEDDING_MODEL` | `gemma` | Embedding model: `qwen3`, `gemma`, `bge_m3`, `nomic`, `e5_multi`, `e5_small`. |
| `--mrl-dim` | `MRL_DIM` | model native dimension | Output dimension for MRL-capable models such as Qwen3 and Gemma. |
| `--batch-size` | `BATCH_SIZE` | `8` | Maximum embedding inference batch size. |
| `--cache-size` | `CACHE_SIZE` | `1000` | LRU cache capacity for embeddings. |
| `--timeout` | `TIMEOUT_MS` | `30000` | Tool timeout in milliseconds. |
| `--model-load-timeout-secs` | `MODEL_LOAD_TIMEOUT_SECS` | `600` | Maximum wait for first model load or download on a fresh machine. |
| `--idle-timeout` | `IDLE_TIMEOUT` | `0` | Idle timeout in minutes. `0` is recommended for MCP stdio. |
| `--stdio` | none | disabled | Use stdio transport. Without this flag, the server starts HTTP/SSE. |
| `--port` | `PORT` | `8080` | HTTP server port. |
| `--bind` | `BIND` | `127.0.0.1` | HTTP bind address. The release Docker image overrides this to `0.0.0.0`. |
| `--log-level` | `LOG_LEVEL` | `info` | Log verbosity. |
| `--log-file` | `LOG_FILE` | none | Optional log file path. |
| `--log-file-max-size-mb` | `LOG_FILE_MAX_SIZE_MB` | `10` | Log rotation size when `--log-file` is set. |
| `--project-path` | `PROJECT_PATH` | none | Primary project root for code intelligence. Fallback is `/project` when present. |
| `--allowed-project-roots` | `ALLOWED_PROJECT_ROOTS` | none | Comma-delimited allowlist for project roots visible to the server. |
| `--max-managed-projects` | `MAX_MANAGED_PROJECTS` | `5` | Maximum managed lifecycle projects in the registry. |
| `--block-cache-mb` | `SURREAL_SURREALKV_BLOCK_CACHE_CAPACITY_MB` | auto | SurrealKV block cache size in MB. |
| none | `SURREAL_SURREALKV_BLOCK_CACHE_CAPACITY` | auto | SurrealKV block cache size in bytes. |
| none | `HF_TOKEN` | none | Optional HuggingFace token for private or rate-limited model downloads. |
| none | `MEMORY_MCP_METRICS_DIR` | none | Enables JSONL performance metrics in the given directory. |
| none | `MEMORY_MCP_METRICS` | `true` when metrics dir is set | Set to `0`, `false`, `off`, or `no` to disable metrics. |

## Model Cache

Native runs store downloaded HuggingFace model files in a durable platform
app-data directory by default. This is intentionally outside `--data-dir` and is
not the OS cache directory, so multiple local server instances can reuse one
model download.

Backward compatibility rule: if the selected model already exists under
`${data_dir}/models`, the server keeps using that legacy cache.

Docker images set `EMBEDDING_MODEL_CACHE_DIR=/data/models`, so the model cache
stays inside the named `/data` volume.

## Model Selection

| Argument | HuggingFace Repo | Dimensions | Size | Use case |
|---|---|---:|---:|---|
| `gemma` | `unsloth/embeddinggemma-300m-qat-q4_0-unquantized` | 768 (MRL) | ~195 MB | Default. Small enough for Docker-friendly local use. |
| `qwen3` | `Qwen/Qwen3-Embedding-0.6B` | 1024 (MRL) | ~1.2 GB | Highest-quality bundled option, with larger RAM and disk cost. |
| `bge_m3` | `BAAI/bge-m3` | 1024 | ~420 MB | Multilingual retrieval option. |
| `nomic` | `nomic-ai/nomic-embed-text-v1.5` | 768 | ~270 MB | Long-context BERT-compatible option. |
| `e5_multi` | `intfloat/multilingual-e5-base` | 768 | ~180 MB | Legacy compatibility. |
| `e5_small` | `intfloat/multilingual-e5-small` | 384 | ~85 MB | Fastest dev/test option. |

Default:

```bash
memory-mcp --model gemma
```

Higher-quality bundled model:

```bash
memory-mcp --model qwen3
```

## MRL Dimensions

Qwen3 and Gemma support Matryoshka Representation Learning. You can truncate
embeddings with `--mrl-dim` to reduce storage and speed vector search.

```bash
memory-mcp --model qwen3 --mrl-dim 512
```

Changing embedding dimensions makes existing vector data incompatible. Reuse a
data directory only when the stored data was created with the same model and
dimension settings.

## Metrics

Metrics are opt-in and do not change startup, indexing, or query behavior.

```bash
MEMORY_MCP_METRICS_DIR=/tmp/memory-mcp-metrics memory-mcp --data-dir /data --project-path /project
```

Each line contains `timestamp`, `event`, `pid`, and `fields`.

| Event | Purpose |
|---|---|
| `startup.storage_open` | SurrealDB/SurrealKV open and schema initialization time. |
| `startup.dimension_check` | Embedding dimension compatibility check time. |
| `startup.state_ready` | Time until in-process server state is constructed. |
| `startup.code_job_recovery` | Startup recovery pass for interrupted index jobs. |
| `startup.code_lifecycle` | Code-intelligence lifecycle startup result and duration. |
| `warmup.code_bm25` | In-memory code BM25 warm-up duration and chunk count. |
| `warmup.memory_bm25` | In-memory memory lexical warm-up duration and memory count. |
| `query.memory_vector` | Memory vector search total and sub-stage timings. |
| `query.memory_bm25` | Memory BM25 total and lexical timings. |
| `query.memory_recall` | Hybrid memory recall total and vector/BM25/PPR timings. |
| `query.code_search` | Internal code vector search timing, used by `recall_code mode="vector"`. |
| `query.code_recall` | Hybrid code recall total and vector/BM25/PPR timings. |

## Docker Notes

- The release image defaults to HTTP/SSE on port `8080`.
- Desktop and CLI MCP integrations should override the container command with
  `memory-mcp --data-dir /data --stdio`.
- The release workflow publishes linux/amd64 and linux/arm64 artifacts.
- The container image resolves the correct binary per target architecture.
- If a tag push is not picked up by GitHub Actions, run the Release workflow
  manually from `master` and provide the existing tag in `release_tag`.

## Data Compatibility

Changing models can make existing vector rows incompatible:

- Different dimensions require a new data directory or reindex.
- Same dimensions across different model families are still not recommended
  because semantic spaces differ.
- Docker users should reuse one `/data` volume only with the same model and
  dimension settings.
