2026-04-23
- Extended `src/storage/surrealdb/memory_ops.rs` search projections for BM25 and vector retrieval to include `event_time`, `ingestion_time`, `access_count`, `last_accessed_at`, and `memory_type`.
- `response.take()` for custom SurrealDB query rows requires an intermediate struct deriving `SurrealValue`; using a local `SearchRow` keeps DB-specific defaults and chrono conversion logic scoped to the storage layer.
- The projected temporal fields can deserialize directly into `Option<chrono::DateTime<chrono::Utc>>`, while `valid_until` continues to use the repo's existing `crate::types::Datetime` type in `SearchResult`.

- Added `src/forgetting/access.rs` with a bounded `tokio::sync::mpsc` access channel, cloneable `AccessTracker`, and shutdown-aware `AccessWriter` following the repo's background worker pattern.
- Access tracking needed a dedicated object-safe `MemoryStorage` adapter in `src/storage/traits.rs` because the existing `StorageBackend` async trait is not dyn-compatible; the writer only depends on `record_memory_access`.
- `record_memory_access` now updates `memories` atomically in SurrealDB by incrementing `access_count` and setting `last_accessed_at` without rewriting the full memory record.

- Recall decay scoring belongs in `src/server/logic/search.rs` immediately after the RRF `combined_score` is multiplied by `importance_multiplier`; that preserves existing importance semantics and applies forgetting strictly post-fusion.
- For retrieval-time decay, use `event_time` first, fall back to `ingestion_time`, clamp future ages to zero, and only re-rank fused recall results when `MEMORY_DECAY_ENABLED` is on; disabled mode should leave `score` ordering unchanged while emitting `decay_factor = 1.0`.

2026-04-24
- `search_memory` single-mode handlers (`search` vector mode and `search_text` BM25 mode) should share one scoring helper that maps `SearchResult -> ScoredMemory` to keep decay wiring identical across both paths and avoid copy-pasted math.
- Single-mode decay should reuse the same `crate::forgetting::decay` functions as hybrid recall (`effective_age_days`, `decay_factor`, `reinforcement_bonus`, `apply_decay_scoring`), including the same event/ingestion fallback anchor logic.
- Preserve legacy ordering when `MEMORY_DECAY_ENABLED=false`; only re-sort by final score when decay is enabled, while still populating `decay_factor` as `1.0` in disabled mode for contract consistency.

- Added post-selection access tracking in `src/server/logic/search.rs` for `search`, `search_text`, and `recall` via a shared optional `AccessTracker` helper so only final returned results emit events after truncation and min-score filtering.
- The access emitter stays fire-and-forget by calling `AccessTracker::track()` per returned memory; this keeps search latency unchanged and matches the existing bounded-channel drop behavior.
- A regression test can validate the exact returned IDs by feeding a local `mpsc` tracker into the private search implementation and asserting the channel only receives the truncated result set.
- Server init now follows the same background-task pattern as the embedding worker: build shared resources in `src/main.rs`, store them on `AppState`, and spawn long-lived tasks with `state.shutdown_rx()` so stdio/http shutdown triggers clean exit for forgetting loops too.
- The object-safe `MemoryStorage` adapter used by forgetting workers must return `Send` futures, and the corresponding `StorageBackend` methods need matching `-> impl Future + Send` guarantees; otherwise `tokio::spawn` rejects the background tasks even though the runtime logic is correct.
- `ForgettingConfig::from_env()` is the internal env-only startup boundary for forgetting tuning; wiring startup through that constructor keeps the feature internal and avoids any MCP or CLI surface changes.
