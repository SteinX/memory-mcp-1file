# Memory Retrieval Baseline

## Run context

- Command used: `/Users/xiayiming1/Documents/Workspace/memory-mcp-1file/target/fast/memory-mcp --stdio`
- Embedding model: `e5_small`
- Data dir: `/var/folders/mx/tffz_j1d5lg_twyfntz97yr00000gp/T/task-8-memory-retrieval-data-1fov0eqr`
- Started at: `2026-04-28T05:00:38.604260+00:00`
- Duration (s): `125.47`

### stderr tail
- [memory-mcp] Auto-configured block cache: 11 MB (available RAM: 56 MB)
- [2m2026-04-28T05:00:39.422066Z[0m [32m INFO[0m [2mmemory_mcp[0m[2m:[0m memory-mcp starting [3mversion[0m[2m=[0m"0.8.2" [3mpid[0m[2m=[0m38452 [3mppid[0m[2m=[0m38420 [3mmode[0m[2m=[0m"stdio" [3mmodel[0m[2m=[0me5_small [3mdata_dir[0m[2m=[0m/var/folders/mx/tffz_j1d5lg_twyfntz97yr00000gp/T/task-8-memory-retrieval-data-1fov0eqr
- [2m2026-04-28T05:00:39.451620Z[0m [32m INFO[0m [2msurrealdb::core::kvs::ds[0m[2m:[0m Starting kvs store at absolute path surrealkv:/var/folders/mx/tffz_j1d5lg_twyfntz97yr00000gp/T/task-8-memory-retrieval-data-1fov0eqr/db
- [2m2026-04-28T05:00:39.454586Z[0m [32m INFO[0m [2msurrealdb::core::kvs::surrealkv[0m[2m:[0m Enabling value log separation: true
- [2m2026-04-28T05:00:39.464997Z[0m [32m INFO[0m [2msurrealdb::core::kvs::surrealkv[0m[2m:[0m Setting value log max file size: 268435456
- [2m2026-04-28T05:00:39.465011Z[0m [32m INFO[0m [2msurrealdb::core::kvs::surrealkv[0m[2m:[0m Setting value log threshold: 4096
- [2m2026-04-28T05:00:39.465012Z[0m [32m INFO[0m [2msurrealdb::core::kvs::surrealkv[0m[2m:[0m Versioning enabled: false with retention period: 0ns
- [2m2026-04-28T05:00:39.465013Z[0m [32m INFO[0m [2msurrealdb::core::kvs::surrealkv[0m[2m:[0m Versioning with versioned_index: false
- [2m2026-04-28T05:00:39.465173Z[0m [32m INFO[0m [2msurrealdb::core::kvs::surrealkv[0m[2m:[0m Setting block cache capacity: 11534336
- [2m2026-04-28T05:00:39.465499Z[0m [32m INFO[0m [2msurrealdb::core::kvs::surrealkv[0m[2m:[0m Setting block size: 65536
- [2m2026-04-28T05:00:39.734903Z[0m [32m INFO[0m [2msurrealkv::lsm[0m[2m:[0m === Starting LSM tree initialization ===
- [2m2026-04-28T05:00:39.735167Z[0m [32m INFO[0m [2msurrealkv::lsm[0m[2m:[0m Database path: "/var/folders/mx/tffz_j1d5lg_twyfntz97yr00000gp/T/task-8-memory-retrieval-data-1fov0eqr/db"
- [2m2026-04-28T05:00:39.866690Z[0m [32m INFO[0m [2msurrealkv::lsm[0m[2m:[0m Manifest state: log_number=0, last_sequence=0
- [2m2026-04-28T05:00:39.866702Z[0m [32m INFO[0m [2msurrealkv::wal::recovery[0m[2m:[0m Starting WAL recovery from directory: "/var/folders/mx/tffz_j1d5lg_twyfntz97yr00000gp/T/task-8-memory-retrieval-data-1fov0eqr/db/wal"
- [2m2026-04-28T05:00:39.866808Z[0m [32m INFO[0m [2msurrealkv::wal::recovery[0m[2m:[0m Replaying WAL segments #00000000000000000000 to #00000000000000000000
- [2m2026-04-28T05:00:39.887561Z[0m [32m INFO[0m [2msurrealkv::wal::recovery[0m[2m:[0m WAL recovery complete: 0 batches across 0 segments, 0 memtables created, max_seq_num=None
- [2m2026-04-28T05:00:39.888719Z[0m [32m INFO[0m [2msurrealkv::lsm[0m[2m:[0m === LSM tree initialization complete ===
- [2m2026-04-28T05:00:39.903115Z[0m [32m INFO[0m [2msurrealdb::core::kvs::surrealkv[0m[2m:[0m Sync mode: every transaction commit
- [2m2026-04-28T05:00:39.903363Z[0m [32m INFO[0m [2msurrealdb::core::kvs::surrealkv[0m[2m:[0m Grouped commit: enabled (timeout=5000000ns, wait_threshold=12, max_batch_size=4096)
- [2m2026-04-28T05:00:39.903616Z[0m [32m INFO[0m [2msurrealdb::core::kvs::ds[0m[2m:[0m Started surrealkv kvs store
- [2m2026-04-28T05:00:40.641093Z[0m [32m INFO[0m [2mmemory_mcp::storage::surrealdb[0m[2m:[0m Dimension check passed [3mmodel[0m[2m=[0m384 [3mdb[0m[2m=[0m384
- [2m2026-04-28T05:00:40.641110Z[0m [32m INFO[0m [2mmemory_mcp[0m[2m:[0m Embedding engine configured [3moutput_dim[0m[2m=[0m384 [3mmodel[0m[2m=[0me5_small
- [2m2026-04-28T05:00:40.717788Z[0m [32m INFO[0m [2mmemory_mcp::embedding::service[0m[2m:[0m Loading embedding model: E5Small
- [2m2026-04-28T05:00:40.718595Z[0m [32m INFO[0m [2mmemory_mcp::forgetting::capacity[0m[2m:[0m Capacity controller started [3mcheck_interval_secs[0m[2m=[0m3600 [3msoft_limit[0m[2m=[0m10000 [3mcleanup_target_ratio[0m[2m=[0m0.800000011920929
- [2m2026-04-28T05:00:40.718763Z[0m [32m INFO[0m [2mmemory_mcp::embedding::worker[0m[2m:[0m Embedding worker started, waiting for requests
- [2m2026-04-28T05:00:40.721797Z[0m [32m INFO[0m [2mmemory_mcp::forgetting::capacity[0m[2m:[0m Capacity OK: 0 memories [3mcount[0m[2m=[0m0
- [2m2026-04-28T05:00:40.723505Z[0m [32m INFO[0m [2mrmcp::handler::server[0m[2m:[0m client initialized
- [2m2026-04-28T05:00:40.723520Z[0m [32m INFO[0m [1mserve_inner[0m[2m:[0m [2mrmcp::service[0m[2m:[0m Service initialized as server [3mpeer_info[0m[2m=[0mSome(InitializeRequestParams { meta: None, protocol_version: ProtocolVersion("2024-11-05"), capabilities: ClientCapabilities { experimental: None, extensions: None, roots: None, sampling: None, elicitation: None, tasks: None }, client_info: Implementation { name: "memory-retrieval-benchmark", title: None, version: "0.1.0", description: None, icons: None, website_url: None } })
- [2m2026-04-28T05:00:40.723543Z[0m [32m INFO[0m [2mmemory_mcp[0m[2m:[0m Server started in stdio mode, waiting for client disconnect or signals...
- [2m2026-04-28T05:00:49.217579Z[0m [32m INFO[0m [2mmemory_mcp::embedding::service[0m[2m:[0m Downloading model weights from HuggingFace Hub...
- [2m2026-04-28T05:02:34.600879Z[0m [32m INFO[0m [2mmemory_mcp::embedding::service[0m[2m:[0m Embedding model ready [3melapsed_sec[0m[2m=[0m"113.9"

## Aggregate metrics

| Metric | Value |
|---|---:|
| baseline_query_count | 10 |
| blocker_count | 0 |
| hit_rate | 0.8 |
| latency_summary | {"count": 10, "max_latency_ms": 130.45512500001166, "mean_latency_ms": 22.23098340000007, "p95_latency_ms": 130.45512500001166} |
| max_latency_ms | 130.4551 |
| mean_expected_rank | 1.125 |
| mean_latency_ms | 22.231 |
| mrr | 0.75 |
| ndcg_at_10 | 0.6996 |
| ndcg_at_5 | 0.6963 |
| observed_summary_partial_reason_codes | [] |
| p95_latency_ms | 130.4551 |
| positive_mean_mrr | 0.9375 |
| positive_mean_precision_at_10 | 0.225 |
| positive_mean_precision_at_5 | 0.35 |
| positive_query_count | 8 |
| precision_at_10 | 0.18 |
| precision_at_5 | 0.28 |
| query_count | 10 |
| readiness_fallback | {"classification": "degraded", "elapsed_s": 7.015, "explanation": "Readiness used fallback settling because no direct ready signal was available; results can still be valid but should be reviewed with elapsed time.", "fallback_sleep_s": 3.0, "fallback_used": true, "impact": "degraded", "poll_attempts": 3, "readiness_signal": "none", "reason": "no_direct_readiness_signal", "status": "fallback_after_no_signal"} |
| reason_code_classification | {} |
| recall_at_10 | 0.7083 |
| recall_at_5 | 0.675 |
| seed_completed | True |

## Readiness fallback

- Status: `fallback_after_no_signal`
- Impact: `degraded`
- Elapsed (s): `7.015`
- Fallback used: `True`
- Explanation: Readiness used fallback settling because no direct ready signal was available; results can still be valid but should be reviewed with elapsed time.

## Per-query metrics

| Query | Rank | MRR | R@5 | R@10 | NDCG@5 | NDCG@10 | P@5 | P@10 | Latency ms | Failure | Top-1 |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|---|
| q_recall_fusion_auth_timeout | 1 | 1 | 1 | 1 | 1 | 1 | 0.4 | 0.2 | 130.4551 | none | abf112a06bf7f41af803 (mem_task_auth_timeout) |
| q_recall_fusion_benchmark_scope | 2 | 0.5 | 1 | 1 | 0.6509 | 0.6509 | 0.4 | 0.2 | 19.8895 | none | 823ab9c8566d1c817aa2 (mem_context_namespace_evals) |
| q_recall_fusion_terse_notes | 1 | 1 | 0.5 | 0.5 | 0.6131 | 0.6131 | 0.2 | 0.1 | 15.5176 | none | e4940467ee23246677fc (mem_user_pref_concise_answers) |
| q_search_bm25_cache_prefix | 1 | 1 | 1 | 1 | 1 | 1 | 0.2 | 0.1 | 1.4215 | none | 6a1dee768aae44230215 (mem_decision_cache_ttl) |
| q_search_bm25_temporal_windows | 1 | 1 | 1 | 1 | 1 | 1 | 0.4 | 0.2 | 1.2749 | none | 345a8538acd5f53305ba (mem_decision_temporal_utc) |
| q_search_vector_billing_retry | 1 | 1 | 1 | 1 | 1 | 1 | 0.2 | 0.1 | 16.7663 | none | 2684aec6fa58b09a1d83 (mem_task_billing_retry_window) |
| q_get_valid_temporal_checkpoint | 1 | 1 | 0.25 | 0.5833 | 0.6992 | 0.7314 | 0.6 | 0.7 | 1.676 | none | 823ab9c8566d1c817aa2 (mem_context_namespace_evals) |
| q_get_valid_filtered_auth_namespace | 1 | 1 | 1 | 1 | 1 | 1 | 0.4 | 0.2 | 1.0337 | none | 6a1dee768aae44230215 (mem_decision_cache_ttl) |
| q_negative_no_match_nonsense | — | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 32.9648 | wrong_rank | dd8fa760daee0b2fe437 (mem_task_temporal_cutoff) |
| q_negative_no_match_missing_prefix | — | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 1.3106 | wrong_rank | 6a1dee768aae44230215 (mem_decision_cache_ttl) |
