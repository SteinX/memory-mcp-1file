# Memory Retrieval Baseline

## Run context

- Command used: `/Users/xiayiming1/Documents/Workspace/memory-mcp-1file/target/fast/memory-mcp --stdio`
- Embedding model: `e5_small`
- Data dir: `/var/folders/mx/tffz_j1d5lg_twyfntz97yr00000gp/T/task-8-memory-retrieval-data-yrjwia7y`
- Started at: `2026-04-28T05:08:06.387063+00:00`
- Duration (s): `124.06`

### stderr tail
- [memory-mcp] Auto-configured block cache: 11 MB (available RAM: 58 MB)
- [2m2026-04-28T05:08:06.439444Z[0m [32m INFO[0m [2mmemory_mcp[0m[2m:[0m memory-mcp starting [3mversion[0m[2m=[0m"0.8.2" [3mpid[0m[2m=[0m54092 [3mppid[0m[2m=[0m54089 [3mmode[0m[2m=[0m"stdio" [3mmodel[0m[2m=[0me5_small [3mdata_dir[0m[2m=[0m/var/folders/mx/tffz_j1d5lg_twyfntz97yr00000gp/T/task-8-memory-retrieval-data-yrjwia7y
- [2m2026-04-28T05:08:06.447585Z[0m [32m INFO[0m [2msurrealdb::core::kvs::ds[0m[2m:[0m Starting kvs store at absolute path surrealkv:/var/folders/mx/tffz_j1d5lg_twyfntz97yr00000gp/T/task-8-memory-retrieval-data-yrjwia7y/db
- [2m2026-04-28T05:08:06.451552Z[0m [32m INFO[0m [2msurrealdb::core::kvs::surrealkv[0m[2m:[0m Enabling value log separation: true
- [2m2026-04-28T05:08:06.464832Z[0m [32m INFO[0m [2msurrealdb::core::kvs::surrealkv[0m[2m:[0m Setting value log max file size: 268435456
- [2m2026-04-28T05:08:06.464847Z[0m [32m INFO[0m [2msurrealdb::core::kvs::surrealkv[0m[2m:[0m Setting value log threshold: 4096
- [2m2026-04-28T05:08:06.464849Z[0m [32m INFO[0m [2msurrealdb::core::kvs::surrealkv[0m[2m:[0m Versioning enabled: false with retention period: 0ns
- [2m2026-04-28T05:08:06.464850Z[0m [32m INFO[0m [2msurrealdb::core::kvs::surrealkv[0m[2m:[0m Versioning with versioned_index: false
- [2m2026-04-28T05:08:06.464998Z[0m [32m INFO[0m [2msurrealdb::core::kvs::surrealkv[0m[2m:[0m Setting block cache capacity: 11534336
- [2m2026-04-28T05:08:06.465088Z[0m [32m INFO[0m [2msurrealdb::core::kvs::surrealkv[0m[2m:[0m Setting block size: 65536
- [2m2026-04-28T05:08:06.465838Z[0m [32m INFO[0m [2msurrealkv::lsm[0m[2m:[0m === Starting LSM tree initialization ===
- [2m2026-04-28T05:08:06.466026Z[0m [32m INFO[0m [2msurrealkv::lsm[0m[2m:[0m Database path: "/var/folders/mx/tffz_j1d5lg_twyfntz97yr00000gp/T/task-8-memory-retrieval-data-yrjwia7y/db"
- [2m2026-04-28T05:08:06.487846Z[0m [32m INFO[0m [2msurrealkv::lsm[0m[2m:[0m Manifest state: log_number=0, last_sequence=0
- [2m2026-04-28T05:08:06.487856Z[0m [32m INFO[0m [2msurrealkv::wal::recovery[0m[2m:[0m Starting WAL recovery from directory: "/var/folders/mx/tffz_j1d5lg_twyfntz97yr00000gp/T/task-8-memory-retrieval-data-yrjwia7y/db/wal"
- [2m2026-04-28T05:08:06.487941Z[0m [32m INFO[0m [2msurrealkv::wal::recovery[0m[2m:[0m Replaying WAL segments #00000000000000000000 to #00000000000000000000
- [2m2026-04-28T05:08:06.505313Z[0m [32m INFO[0m [2msurrealkv::wal::recovery[0m[2m:[0m WAL recovery complete: 0 batches across 0 segments, 0 memtables created, max_seq_num=None
- [2m2026-04-28T05:08:06.505681Z[0m [32m INFO[0m [2msurrealkv::lsm[0m[2m:[0m === LSM tree initialization complete ===
- [2m2026-04-28T05:08:06.523164Z[0m [32m INFO[0m [2msurrealdb::core::kvs::surrealkv[0m[2m:[0m Sync mode: every transaction commit
- [2m2026-04-28T05:08:06.523365Z[0m [32m INFO[0m [2msurrealdb::core::kvs::surrealkv[0m[2m:[0m Grouped commit: enabled (timeout=5000000ns, wait_threshold=12, max_batch_size=4096)
- [2m2026-04-28T05:08:06.523521Z[0m [32m INFO[0m [2msurrealdb::core::kvs::ds[0m[2m:[0m Started surrealkv kvs store
- [2m2026-04-28T05:08:07.273497Z[0m [32m INFO[0m [2mmemory_mcp::storage::surrealdb[0m[2m:[0m Dimension check passed [3mmodel[0m[2m=[0m384 [3mdb[0m[2m=[0m384
- [2m2026-04-28T05:08:07.273511Z[0m [32m INFO[0m [2mmemory_mcp[0m[2m:[0m Embedding engine configured [3moutput_dim[0m[2m=[0m384 [3mmodel[0m[2m=[0me5_small
- [2m2026-04-28T05:08:07.306418Z[0m [32m INFO[0m [2mmemory_mcp::embedding::service[0m[2m:[0m Loading embedding model: E5Small
- [2m2026-04-28T05:08:07.307055Z[0m [32m INFO[0m [2mmemory_mcp::forgetting::capacity[0m[2m:[0m Capacity controller started [3mcheck_interval_secs[0m[2m=[0m3600 [3msoft_limit[0m[2m=[0m10000 [3mcleanup_target_ratio[0m[2m=[0m0.800000011920929
- [2m2026-04-28T05:08:07.307197Z[0m [32m INFO[0m [2mmemory_mcp::embedding::worker[0m[2m:[0m Embedding worker started, waiting for requests
- [2m2026-04-28T05:08:07.309423Z[0m [32m INFO[0m [2mrmcp::handler::server[0m[2m:[0m client initialized
- [2m2026-04-28T05:08:07.309437Z[0m [32m INFO[0m [1mserve_inner[0m[2m:[0m [2mrmcp::service[0m[2m:[0m Service initialized as server [3mpeer_info[0m[2m=[0mSome(InitializeRequestParams { meta: None, protocol_version: ProtocolVersion("2024-11-05"), capabilities: ClientCapabilities { experimental: None, extensions: None, roots: None, sampling: None, elicitation: None, tasks: None }, client_info: Implementation { name: "memory-retrieval-benchmark", title: None, version: "0.1.0", description: None, icons: None, website_url: None } })
- [2m2026-04-28T05:08:07.309459Z[0m [32m INFO[0m [2mmemory_mcp[0m[2m:[0m Server started in stdio mode, waiting for client disconnect or signals...
- [2m2026-04-28T05:08:07.310564Z[0m [32m INFO[0m [2mmemory_mcp::forgetting::capacity[0m[2m:[0m Capacity OK: 0 memories [3mcount[0m[2m=[0m0
- [2m2026-04-28T05:08:14.268975Z[0m [32m INFO[0m [2mmemory_mcp::embedding::service[0m[2m:[0m Downloading model weights from HuggingFace Hub...
- [2m2026-04-28T05:09:58.068626Z[0m [32m INFO[0m [2mmemory_mcp::embedding::service[0m[2m:[0m Embedding model ready [3melapsed_sec[0m[2m=[0m"110.8"

## Aggregate metrics

| Metric | Value |
|---|---:|
| baseline_query_count | 10 |
| blocker_count | 0 |
| hit_rate | 0.8 |
| latency_summary | {"count": 10, "max_latency_ms": 41.100333999992245, "mean_latency_ms": 13.169399999998177, "p95_latency_ms": 41.100333999992245} |
| max_latency_ms | 41.1003 |
| mean_expected_rank | 1.125 |
| mean_latency_ms | 13.1694 |
| mrr | 0.75 |
| ndcg_at_10 | 0.7382 |
| ndcg_at_5 | 0.735 |
| observed_summary_partial_reason_codes | [] |
| p95_latency_ms | 41.1003 |
| positive_mean_mrr | 0.9375 |
| positive_mean_precision_at_10 | 0.2375 |
| positive_mean_precision_at_5 | 0.375 |
| positive_query_count | 8 |
| precision_at_10 | 0.19 |
| precision_at_5 | 0.3 |
| query_count | 10 |
| readiness_fallback | {"classification": "degraded", "elapsed_s": 7.013, "explanation": "Readiness used fallback settling because no direct ready signal was available; results can still be valid but should be reviewed with elapsed time.", "fallback_sleep_s": 3.0, "fallback_used": true, "impact": "degraded", "poll_attempts": 3, "readiness_signal": "none", "reason": "no_direct_readiness_signal", "status": "fallback_after_no_signal"} |
| reason_code_classification | {} |
| recall_at_10 | 0.7583 |
| recall_at_5 | 0.725 |
| seed_completed | True |

## Readiness fallback

- Status: `fallback_after_no_signal`
- Impact: `degraded`
- Elapsed (s): `7.013`
- Fallback used: `True`
- Explanation: Readiness used fallback settling because no direct ready signal was available; results can still be valid but should be reviewed with elapsed time.

## Blockers

- {"command_or_tool": "--refresh-baseline", "diagnostics": {"required_field": "refresh_reason"}, "message": "explicit baseline refresh requires --refresh-reason", "phase": "baseline_refresh_policy", "stderr_tail": ["[memory-mcp] Auto-configured block cache: 11 MB (available RAM: 58 MB)", "\u001b[2m2026-04-28T05:08:06.439444Z\u001b[0m \u001b[32m INFO\u001b[0m \u001b[2mmemory_mcp\u001b[0m\u001b[2m:\u001b[0m memory-mcp starting \u001b[3mversion\u001b[0m\u001b[2m=\u001b[0m\"0.8.2\" \u001b[3mpid\u001b[0m\u001b[2m=\u001b[0m54092 \u001b[3mppid\u001b[0m\u001b[2m=\u001b[0m54089 \u001b[3mmode\u001b[0m\u001b[2m=\u001b[0m\"stdio\" \u001b[3mmodel\u001b[0m\u001b[2m=\u001b[0me5_small \u001b[3mdata_dir\u001b[0m\u001b[2m=\u001b[0m/var/folders/mx/tffz_j1d5lg_twyfntz97yr00000gp/T/task-8-memory-retrieval-data-yrjwia7y", "\u001b[2m2026-04-28T05:08:06.447585Z\u001b[0m \u001b[32m INFO\u001b[0m \u001b[2msurrealdb::core::kvs::ds\u001b[0m\u001b[2m:\u001b[0m Starting kvs store at absolute path surrealkv:/var/folders/mx/tffz_j1d5lg_twyfntz97yr00000gp/T/task-8-memory-retrieval-data-yrjwia7y/db", "\u001b[2m2026-04-28T05:08:06.451552Z\u001b[0m \u001b[32m INFO\u001b[0m \u001b[2msurrealdb::core::kvs::surrealkv\u001b[0m\u001b[2m:\u001b[0m Enabling value log separation: true", "\u001b[2m2026-04-28T05:08:06.464832Z\u001b[0m \u001b[32m INFO\u001b[0m \u001b[2msurrealdb::core::kvs::surrealkv\u001b[0m\u001b[2m:\u001b[0m Setting value log max file size: 268435456", "\u001b[2m2026-04-28T05:08:06.464847Z\u001b[0m \u001b[32m INFO\u001b[0m \u001b[2msurrealdb::core::kvs::surrealkv\u001b[0m\u001b[2m:\u001b[0m Setting value log threshold: 4096", "\u001b[2m2026-04-28T05:08:06.464849Z\u001b[0m \u001b[32m INFO\u001b[0m \u001b[2msurrealdb::core::kvs::surrealkv\u001b[0m\u001b[2m:\u001b[0m Versioning enabled: false with retention period: 0ns", "\u001b[2m2026-04-28T05:08:06.464850Z\u001b[0m \u001b[32m INFO\u001b[0m \u001b[2msurrealdb::core::kvs::surrealkv\u001b[0m\u001b[2m:\u001b[0m Versioning with versioned_index: false", "\u001b[2m2026-04-28T05:08:06.464998Z\u001b[0m \u001b[32m INFO\u001b[0m \u001b[2msurrealdb::core::kvs::surrealkv\u001b[0m\u001b[2m:\u001b[0m Setting block cache capacity: 11534336", "\u001b[2m2026-04-28T05:08:06.465088Z\u001b[0m \u001b[32m INFO\u001b[0m \u001b[2msurrealdb::core::kvs::surrealkv\u001b[0m\u001b[2m:\u001b[0m Setting block size: 65536", "\u001b[2m2026-04-28T05:08:06.465838Z\u001b[0m \u001b[32m INFO\u001b[0m \u001b[2msurrealkv::lsm\u001b[0m\u001b[2m:\u001b[0m === Starting LSM tree initialization ===", "\u001b[2m2026-04-28T05:08:06.466026Z\u001b[0m \u001b[32m INFO\u001b[0m \u001b[2msurrealkv::lsm\u001b[0m\u001b[2m:\u001b[0m Database path: \"/var/folders/mx/tffz_j1d5lg_twyfntz97yr00000gp/T/task-8-memory-retrieval-data-yrjwia7y/db\"", "\u001b[2m2026-04-28T05:08:06.487846Z\u001b[0m \u001b[32m INFO\u001b[0m \u001b[2msurrealkv::lsm\u001b[0m\u001b[2m:\u001b[0m Manifest state: log_number=0, last_sequence=0", "\u001b[2m2026-04-28T05:08:06.487856Z\u001b[0m \u001b[32m INFO\u001b[0m \u001b[2msurrealkv::wal::recovery\u001b[0m\u001b[2m:\u001b[0m Starting WAL recovery from directory: \"/var/folders/mx/tffz_j1d5lg_twyfntz97yr00000gp/T/task-8-memory-retrieval-data-yrjwia7y/db/wal\"", "\u001b[2m2026-04-28T05:08:06.487941Z\u001b[0m \u001b[32m INFO\u001b[0m \u001b[2msurrealkv::wal::recovery\u001b[0m\u001b[2m:\u001b[0m Replaying WAL segments #00000000000000000000 to #00000000000000000000", "\u001b[2m2026-04-28T05:08:06.505313Z\u001b[0m \u001b[32m INFO\u001b[0m \u001b[2msurrealkv::wal::recovery\u001b[0m\u001b[2m:\u001b[0m WAL recovery complete: 0 batches across 0 segments, 0 memtables created, max_seq_num=None", "\u001b[2m2026-04-28T05:08:06.505681Z\u001b[0m \u001b[32m INFO\u001b[0m \u001b[2msurrealkv::lsm\u001b[0m\u001b[2m:\u001b[0m === LSM tree initialization complete ===", "\u001b[2m2026-04-28T05:08:06.523164Z\u001b[0m \u001b[32m INFO\u001b[0m \u001b[2msurrealdb::core::kvs::surrealkv\u001b[0m\u001b[2m:\u001b[0m Sync mode: every transaction commit", "\u001b[2m2026-04-28T05:08:06.523365Z\u001b[0m \u001b[32m INFO\u001b[0m \u001b[2msurrealdb::core::kvs::surrealkv\u001b[0m\u001b[2m:\u001b[0m Grouped commit: enabled (timeout=5000000ns, wait_threshold=12, max_batch_size=4096)", "\u001b[2m2026-04-28T05:08:06.523521Z\u001b[0m \u001b[32m INFO\u001b[0m \u001b[2msurrealdb::core::kvs::ds\u001b[0m\u001b[2m:\u001b[0m Started surrealkv kvs store", "\u001b[2m2026-04-28T05:08:07.273497Z\u001b[0m \u001b[32m INFO\u001b[0m \u001b[2mmemory_mcp::storage::surrealdb\u001b[0m\u001b[2m:\u001b[0m Dimension check passed \u001b[3mmodel\u001b[0m\u001b[2m=\u001b[0m384 \u001b[3mdb\u001b[0m\u001b[2m=\u001b[0m384", "\u001b[2m2026-04-28T05:08:07.273511Z\u001b[0m \u001b[32m INFO\u001b[0m \u001b[2mmemory_mcp\u001b[0m\u001b[2m:\u001b[0m Embedding engine configured \u001b[3moutput_dim\u001b[0m\u001b[2m=\u001b[0m384 \u001b[3mmodel\u001b[0m\u001b[2m=\u001b[0me5_small", "\u001b[2m2026-04-28T05:08:07.306418Z\u001b[0m \u001b[32m INFO\u001b[0m \u001b[2mmemory_mcp::embedding::service\u001b[0m\u001b[2m:\u001b[0m Loading embedding model: E5Small", "\u001b[2m2026-04-28T05:08:07.307055Z\u001b[0m \u001b[32m INFO\u001b[0m \u001b[2mmemory_mcp::forgetting::capacity\u001b[0m\u001b[2m:\u001b[0m Capacity controller started \u001b[3mcheck_interval_secs\u001b[0m\u001b[2m=\u001b[0m3600 \u001b[3msoft_limit\u001b[0m\u001b[2m=\u001b[0m10000 \u001b[3mcleanup_target_ratio\u001b[0m\u001b[2m=\u001b[0m0.800000011920929", "\u001b[2m2026-04-28T05:08:07.307197Z\u001b[0m \u001b[32m INFO\u001b[0m \u001b[2mmemory_mcp::embedding::worker\u001b[0m\u001b[2m:\u001b[0m Embedding worker started, waiting for requests", "\u001b[2m2026-04-28T05:08:07.309423Z\u001b[0m \u001b[32m INFO\u001b[0m \u001b[2mrmcp::handler::server\u001b[0m\u001b[2m:\u001b[0m client initialized", "\u001b[2m2026-04-28T05:08:07.309437Z\u001b[0m \u001b[32m INFO\u001b[0m \u001b[1mserve_inner\u001b[0m\u001b[2m:\u001b[0m \u001b[2mrmcp::service\u001b[0m\u001b[2m:\u001b[0m Service initialized as server \u001b[3mpeer_info\u001b[0m\u001b[2m=\u001b[0mSome(InitializeRequestParams { meta: None, protocol_version: ProtocolVersion(\"2024-11-05\"), capabilities: ClientCapabilities { experimental: None, extensions: None, roots: None, sampling: None, elicitation: None, tasks: None }, client_info: Implementation { name: \"memory-retrieval-benchmark\", title: None, version: \"0.1.0\", description: None, icons: None, website_url: None } })", "\u001b[2m2026-04-28T05:08:07.309459Z\u001b[0m \u001b[32m INFO\u001b[0m \u001b[2mmemory_mcp\u001b[0m\u001b[2m:\u001b[0m Server started in stdio mode, waiting for client disconnect or signals...", "\u001b[2m2026-04-28T05:08:07.310564Z\u001b[0m \u001b[32m INFO\u001b[0m \u001b[2mmemory_mcp::forgetting::capacity\u001b[0m\u001b[2m:\u001b[0m Capacity OK: 0 memories \u001b[3mcount\u001b[0m\u001b[2m=\u001b[0m0", "\u001b[2m2026-04-28T05:08:14.268975Z\u001b[0m \u001b[32m INFO\u001b[0m \u001b[2mmemory_mcp::embedding::service\u001b[0m\u001b[2m:\u001b[0m Downloading model weights from HuggingFace Hub...", "\u001b[2m2026-04-28T05:09:58.068626Z\u001b[0m \u001b[32m INFO\u001b[0m \u001b[2mmemory_mcp::embedding::service\u001b[0m\u001b[2m:\u001b[0m Embedding model ready \u001b[3melapsed_sec\u001b[0m\u001b[2m=\u001b[0m\"110.8\""], "summary_partial_reason_code": null}

## Per-query metrics

| Query | Rank | MRR | R@5 | R@10 | NDCG@5 | NDCG@10 | P@5 | P@10 | Latency ms | Failure | Top-1 |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|---|
| q_recall_fusion_auth_timeout | 1 | 1 | 1 | 1 | 1 | 1 | 0.4 | 0.2 | 41.1003 | none | b810fcc664cc1aa91b00 (mem_task_auth_timeout) |
| q_recall_fusion_benchmark_scope | 2 | 0.5 | 1 | 1 | 0.6509 | 0.6509 | 0.4 | 0.2 | 39.8126 | none | 2bab98910d88200a0164 (mem_context_namespace_evals) |
| q_recall_fusion_terse_notes | 1 | 1 | 1 | 1 | 1 | 1 | 0.4 | 0.2 | 14.8533 | none | dbce7ba47cf83099e06f (mem_user_pref_concise_answers) |
| q_search_bm25_cache_prefix | 1 | 1 | 1 | 1 | 1 | 1 | 0.2 | 0.1 | 1.2899 | none | a2582f76183c57ad5cd7 (mem_decision_cache_ttl) |
| q_search_bm25_temporal_windows | 1 | 1 | 1 | 1 | 1 | 1 | 0.4 | 0.2 | 1.0665 | none | 43716377611ba7acde93 (mem_decision_temporal_utc) |
| q_search_vector_billing_retry | 1 | 1 | 1 | 1 | 1 | 1 | 0.2 | 0.1 | 16.324 | none | 85c07f27ae2014854a5c (mem_task_billing_retry_window) |
| q_get_valid_temporal_checkpoint | 1 | 1 | 0.25 | 0.5833 | 0.6992 | 0.7314 | 0.6 | 0.7 | 1.6931 | none | 2bab98910d88200a0164 (mem_context_namespace_evals) |
| q_get_valid_filtered_auth_namespace | 1 | 1 | 1 | 1 | 1 | 1 | 0.4 | 0.2 | 0.996 | none | a2582f76183c57ad5cd7 (mem_decision_cache_ttl) |
| q_negative_no_match_nonsense | — | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 13.5393 | wrong_rank | a2582f76183c57ad5cd7 (mem_decision_cache_ttl) |
| q_negative_no_match_missing_prefix | — | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 1.0191 | wrong_rank | a2582f76183c57ad5cd7 (mem_decision_cache_ttl) |
