# Memory Retrieval Baseline

## Run context

- Command used: `/Users/xiayiming1/Documents/Workspace/memory-mcp-1file/target/fast/memory-mcp --stdio`
- Embedding model: `e5_small`
- Data dir: `/var/folders/mx/tffz_j1d5lg_twyfntz97yr00000gp/T/task-8-memory-retrieval-data-kvmyz06d`
- Started at: `2026-04-28T10:32:15.884090+00:00`
- Duration (s): `57.01`

### stderr tail
- [memory-mcp] Auto-configured block cache: 12 MB (available RAM: 61 MB)
- [2m2026-04-28T10:32:15.925756Z[0m [32m INFO[0m [2mmemory_mcp[0m[2m:[0m memory-mcp starting [3mversion[0m[2m=[0m"0.8.2" [3mpid[0m[2m=[0m84176 [3mppid[0m[2m=[0m84175 [3mmode[0m[2m=[0m"stdio" [3mmodel[0m[2m=[0me5_small [3mdata_dir[0m[2m=[0m/var/folders/mx/tffz_j1d5lg_twyfntz97yr00000gp/T/task-8-memory-retrieval-data-kvmyz06d
- [2m2026-04-28T10:32:15.931620Z[0m [32m INFO[0m [2msurrealdb::core::kvs::ds[0m[2m:[0m Starting kvs store at absolute path surrealkv:/var/folders/mx/tffz_j1d5lg_twyfntz97yr00000gp/T/task-8-memory-retrieval-data-kvmyz06d/db
- [2m2026-04-28T10:32:15.934449Z[0m [32m INFO[0m [2msurrealdb::core::kvs::surrealkv[0m[2m:[0m Enabling value log separation: true
- [2m2026-04-28T10:32:15.946727Z[0m [32m INFO[0m [2msurrealdb::core::kvs::surrealkv[0m[2m:[0m Setting value log max file size: 268435456
- [2m2026-04-28T10:32:15.946739Z[0m [32m INFO[0m [2msurrealdb::core::kvs::surrealkv[0m[2m:[0m Setting value log threshold: 4096
- [2m2026-04-28T10:32:15.946741Z[0m [32m INFO[0m [2msurrealdb::core::kvs::surrealkv[0m[2m:[0m Versioning enabled: false with retention period: 0ns
- [2m2026-04-28T10:32:15.946742Z[0m [32m INFO[0m [2msurrealdb::core::kvs::surrealkv[0m[2m:[0m Versioning with versioned_index: false
- [2m2026-04-28T10:32:15.946891Z[0m [32m INFO[0m [2msurrealdb::core::kvs::surrealkv[0m[2m:[0m Setting block cache capacity: 12582912
- [2m2026-04-28T10:32:15.946974Z[0m [32m INFO[0m [2msurrealdb::core::kvs::surrealkv[0m[2m:[0m Setting block size: 65536
- [2m2026-04-28T10:32:15.947767Z[0m [32m INFO[0m [2msurrealkv::lsm[0m[2m:[0m === Starting LSM tree initialization ===
- [2m2026-04-28T10:32:15.947940Z[0m [32m INFO[0m [2msurrealkv::lsm[0m[2m:[0m Database path: "/var/folders/mx/tffz_j1d5lg_twyfntz97yr00000gp/T/task-8-memory-retrieval-data-kvmyz06d/db"
- [2m2026-04-28T10:32:15.972812Z[0m [32m INFO[0m [2msurrealkv::lsm[0m[2m:[0m Manifest state: log_number=0, last_sequence=0
- [2m2026-04-28T10:32:15.972823Z[0m [32m INFO[0m [2msurrealkv::wal::recovery[0m[2m:[0m Starting WAL recovery from directory: "/var/folders/mx/tffz_j1d5lg_twyfntz97yr00000gp/T/task-8-memory-retrieval-data-kvmyz06d/db/wal"
- [2m2026-04-28T10:32:15.972910Z[0m [32m INFO[0m [2msurrealkv::wal::recovery[0m[2m:[0m Replaying WAL segments #00000000000000000000 to #00000000000000000000
- [2m2026-04-28T10:32:15.993409Z[0m [32m INFO[0m [2msurrealkv::wal::recovery[0m[2m:[0m WAL recovery complete: 0 batches across 0 segments, 0 memtables created, max_seq_num=None
- [2m2026-04-28T10:32:15.993718Z[0m [32m INFO[0m [2msurrealkv::lsm[0m[2m:[0m === LSM tree initialization complete ===
- [2m2026-04-28T10:32:16.011089Z[0m [32m INFO[0m [2msurrealdb::core::kvs::surrealkv[0m[2m:[0m Sync mode: every transaction commit
- [2m2026-04-28T10:32:16.011343Z[0m [32m INFO[0m [2msurrealdb::core::kvs::surrealkv[0m[2m:[0m Grouped commit: enabled (timeout=5000000ns, wait_threshold=12, max_batch_size=4096)
- [2m2026-04-28T10:32:16.011532Z[0m [32m INFO[0m [2msurrealdb::core::kvs::ds[0m[2m:[0m Started surrealkv kvs store
- [2m2026-04-28T10:32:16.744677Z[0m [32m INFO[0m [2mmemory_mcp::storage::surrealdb[0m[2m:[0m Dimension check passed [3mmodel[0m[2m=[0m384 [3mdb[0m[2m=[0m384
- [2m2026-04-28T10:32:16.744701Z[0m [32m INFO[0m [2mmemory_mcp[0m[2m:[0m Embedding engine configured [3moutput_dim[0m[2m=[0m384 [3mmodel[0m[2m=[0me5_small
- [2m2026-04-28T10:32:16.780647Z[0m [32m INFO[0m [2mmemory_mcp::embedding::service[0m[2m:[0m Loading embedding model: E5Small
- [2m2026-04-28T10:32:16.781196Z[0m [32m INFO[0m [2mmemory_mcp::embedding::worker[0m[2m:[0m Embedding worker started, waiting for requests
- [2m2026-04-28T10:32:16.781214Z[0m [32m INFO[0m [2mmemory_mcp::forgetting::capacity[0m[2m:[0m Capacity controller started [3mcheck_interval_secs[0m[2m=[0m3600 [3msoft_limit[0m[2m=[0m10000 [3mcleanup_target_ratio[0m[2m=[0m0.800000011920929
- [2m2026-04-28T10:32:16.787795Z[0m [32m INFO[0m [2mmemory_mcp::forgetting::capacity[0m[2m:[0m Capacity OK: 0 memories [3mcount[0m[2m=[0m0
- [2m2026-04-28T10:32:16.788043Z[0m [32m INFO[0m [2mrmcp::handler::server[0m[2m:[0m client initialized
- [2m2026-04-28T10:32:16.788062Z[0m [32m INFO[0m [1mserve_inner[0m[2m:[0m [2mrmcp::service[0m[2m:[0m Service initialized as server [3mpeer_info[0m[2m=[0mSome(InitializeRequestParams { meta: None, protocol_version: ProtocolVersion("2024-11-05"), capabilities: ClientCapabilities { experimental: None, extensions: None, roots: None, sampling: None, elicitation: None, tasks: None }, client_info: Implementation { name: "memory-retrieval-benchmark", title: None, version: "0.1.0", description: None, icons: None, website_url: None } })
- [2m2026-04-28T10:32:16.788101Z[0m [32m INFO[0m [2mmemory_mcp[0m[2m:[0m Server started in stdio mode, waiting for client disconnect or signals...
- [2m2026-04-28T10:32:23.609920Z[0m [32m INFO[0m [2mmemory_mcp::embedding::service[0m[2m:[0m Downloading model weights from HuggingFace Hub...
- [2m2026-04-28T10:33:01.917999Z[0m [32m INFO[0m [2mmemory_mcp::embedding::service[0m[2m:[0m Embedding model ready [3melapsed_sec[0m[2m=[0m"45.1"

## Aggregate metrics

| Metric | Value |
|---|---:|
| baseline_query_count | 10 |
| blocker_count | 0 |
| hit_rate | 0.8 |
| latency_summary | {"count": 10, "max_latency_ms": 26.9792080000002, "mean_latency_ms": 11.00468759999913, "p95_latency_ms": 26.9792080000002} |
| max_latency_ms | 26.9792 |
| mean_expected_rank | 1.125 |
| mean_latency_ms | 11.0047 |
| mrr | 0.75 |
| ndcg_at_10 | 0.6996 |
| ndcg_at_5 | 0.6963 |
| observed_summary_partial_reason_codes | [] |
| p95_latency_ms | 26.9792 |
| positive_mean_mrr | 0.9375 |
| positive_mean_precision_at_10 | 0.225 |
| positive_mean_precision_at_5 | 0.35 |
| positive_query_count | 8 |
| precision_at_10 | 0.18 |
| precision_at_5 | 0.28 |
| query_count | 10 |
| readiness_fallback | {"classification": "degraded", "elapsed_s": 7.013, "explanation": "Readiness used fallback settling because no direct ready signal was available; results can still be valid but should be reviewed with elapsed time.", "fallback_sleep_s": 3.0, "fallback_used": true, "impact": "degraded", "poll_attempts": 3, "readiness_signal": "none", "reason": "no_direct_readiness_signal", "status": "fallback_after_no_signal"} |
| reason_code_classification | {} |
| recall_at_10 | 0.7083 |
| recall_at_5 | 0.675 |
| seed_completed | True |

## Warnings

- baseline refresh not requested; canonical baseline targets are protected and output was redirected

## Readiness fallback

- Status: `fallback_after_no_signal`
- Impact: `degraded`
- Elapsed (s): `7.013`
- Fallback used: `True`
- Explanation: Readiness used fallback settling because no direct ready signal was available; results can still be valid but should be reviewed with elapsed time.

## Per-query metrics

| Query | Rank | MRR | R@5 | R@10 | NDCG@5 | NDCG@10 | P@5 | P@10 | Latency ms | Failure | Top-1 |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|---|
| q_recall_fusion_auth_timeout | 1 | 1 | 0.5 | 0.5 | 0.6131 | 0.6131 | 0.2 | 0.1 | 26.9792 | none | 8f02b4a070d5423a2a6b (mem_decision_cache_ttl) |
| q_recall_fusion_benchmark_scope | 2 | 0.5 | 1 | 1 | 0.6509 | 0.6509 | 0.4 | 0.2 | 24.2385 | none | 5e1ac120586ff8eab83a (mem_context_namespace_evals) |
| q_recall_fusion_terse_notes | 1 | 1 | 1 | 1 | 1 | 1 | 0.4 | 0.2 | 18.8805 | none | 91b83331eb3cc08ced98 (mem_user_pref_concise_answers) |
| q_search_bm25_cache_prefix | 1 | 1 | 1 | 1 | 1 | 1 | 0.2 | 0.1 | 1.7762 | none | 8f02b4a070d5423a2a6b (mem_decision_cache_ttl) |
| q_search_bm25_temporal_windows | 1 | 1 | 1 | 1 | 1 | 1 | 0.4 | 0.2 | 1.344 | none | 9dc8851caee30ff2f512 (mem_decision_temporal_utc) |
| q_search_vector_billing_retry | 1 | 1 | 1 | 1 | 1 | 1 | 0.2 | 0.1 | 17.8812 | none | ae41d2bab228307626ac (mem_task_billing_retry_window) |
| q_get_valid_temporal_checkpoint | 1 | 1 | 0.25 | 0.5833 | 0.6992 | 0.7314 | 0.6 | 0.7 | 1.8905 | none | 5e1ac120586ff8eab83a (mem_context_namespace_evals) |
| q_get_valid_filtered_auth_namespace | 1 | 1 | 1 | 1 | 1 | 1 | 0.4 | 0.2 | 1.1465 | none | 8f02b4a070d5423a2a6b (mem_decision_cache_ttl) |
| q_negative_no_match_nonsense | — | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 14.5346 | wrong_rank | b3ef26046902891ed9b1 (mem_task_temporal_cutoff) |
| q_negative_no_match_missing_prefix | — | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 1.3756 | wrong_rank | 8f02b4a070d5423a2a6b (mem_decision_cache_ttl) |
