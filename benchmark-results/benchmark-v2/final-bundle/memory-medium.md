# Memory Retrieval Baseline

## Benchmark V2 summary

| Field | Value |
|---|---|
| schema_version | `2.0` |
| fixture_tier | `medium` |
| baseline_version | `v2-initial` |
| threshold_policy | `local-v2-threshold-policy` |
| threshold_status | `pass` |
| threshold_status_reason | `all required threshold checks passed` |

### Readiness summary

- reason_codes: `[]`
- reason_code_classification: `{}`
- readiness_fallback: `{"classification": "degraded", "elapsed_s": 7.019, "explanation": "Readiness used fallback settling because no direct ready signal was available; results can still be valid but should be reviewed with elapsed time.", "fallback_sleep_s": 3.0, "fallback_used": true, "impact": "degraded", "poll_attempts": 3, "readiness_signal": "none", "reason": "no_direct_readiness_signal", "status": "fallback_after_no_signal"}`

### Failure buckets

| Failure type | Count |
|---|---:|
| none | 6 |
| wrong_rank | 1 |

### Baseline diff summary

- status: `deferred`
- reason: `baseline diff summary is produced by explicit baseline-diff workflow`

### Metric summary

| Metric | Value |
|---|---:|
| query_count | 7 |
| hit_rate | 0.8571 |
| mrr | 0.8571 |
| precision_at_5 | 0.3143 |
| precision_at_10 | 0.1571 |
| recall_at_5 | 0.8095 |
| recall_at_10 | 0.8095 |
| ndcg_at_5 | 0.8236 |
| ndcg_at_10 | 0.8236 |
| mean_latency_ms | 17.1442 |
| max_latency_ms | 49.3113 |
| p95_latency_ms | 49.3113 |
| blocker_count | 0 |
| positive_query_count | 6 |
| positive_hit_rate | 1 |
| positive_mean_mrr | 1 |
| positive_mean_recall_at_5 | 0.9444 |
| positive_mean_ndcg_at_5 | 0.9609 |
| positive_mean_precision_at_5 | 0.3667 |
| runtime_minutes | 1.1861 |

### Deterministic / local-only metadata

- threshold_policy_enforcement: `local-only`
- determinism_policy: `{"name": "stable_fixture_order+stable_tie_break+stable_report_order+tolerance_1e-9_1e-6"}`
- runtime_target: `{"optional_policy": "explicit medium-tier run", "required_by_default": false, "target_minutes": "15-30"}`

## Run context

- Command used: `/Users/xiayiming1/Documents/Workspace/memory-mcp-1file/target/fast/memory-mcp --stdio`
- Embedding model: `e5_small`
- Data dir: `/var/folders/mx/tffz_j1d5lg_twyfntz97yr00000gp/T/task-8-memory-retrieval-data-7vh9fk3f`
- Started at: `2026-04-28T12:31:21.354991+00:00`
- Duration (s): `71.18`

### stderr tail
- [memory-mcp] Auto-configured block cache: 13 MB (available RAM: 65 MB)
- [2m2026-04-28T12:31:21.383691Z[0m [32m INFO[0m [2mmemory_mcp[0m[2m:[0m memory-mcp starting [3mversion[0m[2m=[0m"0.8.2" [3mpid[0m[2m=[0m13039 [3mppid[0m[2m=[0m13036 [3mmode[0m[2m=[0m"stdio" [3mmodel[0m[2m=[0me5_small [3mdata_dir[0m[2m=[0m/var/folders/mx/tffz_j1d5lg_twyfntz97yr00000gp/T/task-8-memory-retrieval-data-7vh9fk3f
- [2m2026-04-28T12:31:21.389095Z[0m [32m INFO[0m [2msurrealdb::core::kvs::ds[0m[2m:[0m Starting kvs store at absolute path surrealkv:/var/folders/mx/tffz_j1d5lg_twyfntz97yr00000gp/T/task-8-memory-retrieval-data-7vh9fk3f/db
- [2m2026-04-28T12:31:21.392082Z[0m [32m INFO[0m [2msurrealdb::core::kvs::surrealkv[0m[2m:[0m Enabling value log separation: true
- [2m2026-04-28T12:31:21.403280Z[0m [32m INFO[0m [2msurrealdb::core::kvs::surrealkv[0m[2m:[0m Setting value log max file size: 268435456
- [2m2026-04-28T12:31:21.403296Z[0m [32m INFO[0m [2msurrealdb::core::kvs::surrealkv[0m[2m:[0m Setting value log threshold: 4096
- [2m2026-04-28T12:31:21.403297Z[0m [32m INFO[0m [2msurrealdb::core::kvs::surrealkv[0m[2m:[0m Versioning enabled: false with retention period: 0ns
- [2m2026-04-28T12:31:21.403298Z[0m [32m INFO[0m [2msurrealdb::core::kvs::surrealkv[0m[2m:[0m Versioning with versioned_index: false
- [2m2026-04-28T12:31:21.403470Z[0m [32m INFO[0m [2msurrealdb::core::kvs::surrealkv[0m[2m:[0m Setting block cache capacity: 13631488
- [2m2026-04-28T12:31:21.403546Z[0m [32m INFO[0m [2msurrealdb::core::kvs::surrealkv[0m[2m:[0m Setting block size: 65536
- [2m2026-04-28T12:31:21.404243Z[0m [32m INFO[0m [2msurrealkv::lsm[0m[2m:[0m === Starting LSM tree initialization ===
- [2m2026-04-28T12:31:21.404546Z[0m [32m INFO[0m [2msurrealkv::lsm[0m[2m:[0m Database path: "/var/folders/mx/tffz_j1d5lg_twyfntz97yr00000gp/T/task-8-memory-retrieval-data-7vh9fk3f/db"
- [2m2026-04-28T12:31:21.425735Z[0m [32m INFO[0m [2msurrealkv::lsm[0m[2m:[0m Manifest state: log_number=0, last_sequence=0
- [2m2026-04-28T12:31:21.425743Z[0m [32m INFO[0m [2msurrealkv::wal::recovery[0m[2m:[0m Starting WAL recovery from directory: "/var/folders/mx/tffz_j1d5lg_twyfntz97yr00000gp/T/task-8-memory-retrieval-data-7vh9fk3f/db/wal"
- [2m2026-04-28T12:31:21.425814Z[0m [32m INFO[0m [2msurrealkv::wal::recovery[0m[2m:[0m Replaying WAL segments #00000000000000000000 to #00000000000000000000
- [2m2026-04-28T12:31:21.446175Z[0m [32m INFO[0m [2msurrealkv::wal::recovery[0m[2m:[0m WAL recovery complete: 0 batches across 0 segments, 0 memtables created, max_seq_num=None
- [2m2026-04-28T12:31:21.446425Z[0m [32m INFO[0m [2msurrealkv::lsm[0m[2m:[0m === LSM tree initialization complete ===
- [2m2026-04-28T12:31:21.457985Z[0m [32m INFO[0m [2msurrealdb::core::kvs::surrealkv[0m[2m:[0m Sync mode: every transaction commit
- [2m2026-04-28T12:31:21.458176Z[0m [32m INFO[0m [2msurrealdb::core::kvs::surrealkv[0m[2m:[0m Grouped commit: enabled (timeout=5000000ns, wait_threshold=12, max_batch_size=4096)
- [2m2026-04-28T12:31:21.458336Z[0m [32m INFO[0m [2msurrealdb::core::kvs::ds[0m[2m:[0m Started surrealkv kvs store
- [2m2026-04-28T12:31:22.152537Z[0m [32m INFO[0m [2mmemory_mcp::storage::surrealdb[0m[2m:[0m Dimension check passed [3mmodel[0m[2m=[0m384 [3mdb[0m[2m=[0m384
- [2m2026-04-28T12:31:22.152556Z[0m [32m INFO[0m [2mmemory_mcp[0m[2m:[0m Embedding engine configured [3moutput_dim[0m[2m=[0m384 [3mmodel[0m[2m=[0me5_small
- [2m2026-04-28T12:31:22.186071Z[0m [32m INFO[0m [2mmemory_mcp::embedding::service[0m[2m:[0m Loading embedding model: E5Small
- [2m2026-04-28T12:31:22.187208Z[0m [32m INFO[0m [2mmemory_mcp::forgetting::capacity[0m[2m:[0m Capacity controller started [3mcheck_interval_secs[0m[2m=[0m3600 [3msoft_limit[0m[2m=[0m10000 [3mcleanup_target_ratio[0m[2m=[0m0.800000011920929
- [2m2026-04-28T12:31:22.187604Z[0m [32m INFO[0m [2mmemory_mcp::embedding::worker[0m[2m:[0m Embedding worker started, waiting for requests
- [2m2026-04-28T12:31:22.194259Z[0m [32m INFO[0m [2mrmcp::handler::server[0m[2m:[0m client initialized
- [2m2026-04-28T12:31:22.194277Z[0m [32m INFO[0m [1mserve_inner[0m[2m:[0m [2mrmcp::service[0m[2m:[0m Service initialized as server [3mpeer_info[0m[2m=[0mSome(InitializeRequestParams { meta: None, protocol_version: ProtocolVersion("2024-11-05"), capabilities: ClientCapabilities { experimental: None, extensions: None, roots: None, sampling: None, elicitation: None, tasks: None }, client_info: Implementation { name: "memory-retrieval-benchmark", title: None, version: "0.1.0", description: None, icons: None, website_url: None } })
- [2m2026-04-28T12:31:22.194345Z[0m [32m INFO[0m [2mmemory_mcp[0m[2m:[0m Server started in stdio mode, waiting for client disconnect or signals...
- [2m2026-04-28T12:31:22.195164Z[0m [32m INFO[0m [2mmemory_mcp::forgetting::capacity[0m[2m:[0m Capacity OK: 0 memories [3mcount[0m[2m=[0m0
- [2m2026-04-28T12:31:29.373709Z[0m [32m INFO[0m [2mmemory_mcp::embedding::service[0m[2m:[0m Downloading model weights from HuggingFace Hub...
- [2m2026-04-28T12:32:20.543473Z[0m [32m INFO[0m [2mmemory_mcp::embedding::service[0m[2m:[0m Embedding model ready [3melapsed_sec[0m[2m=[0m"58.4"

## Aggregate metrics

| Metric | Value |
|---|---:|
| baseline_query_count | 7 |
| blocker_count | 0 |
| hit_rate | 0.8571 |
| latency_summary | {"count": 7, "max_latency_ms": 49.31125000000236, "mean_latency_ms": 17.14423214285838, "p95_latency_ms": 49.31125000000236} |
| max_latency_ms | 49.3113 |
| mean_expected_rank | 1 |
| mean_latency_ms | 17.1442 |
| mrr | 0.8571 |
| ndcg_at_10 | 0.8236 |
| ndcg_at_5 | 0.8236 |
| observed_summary_partial_reason_codes | [] |
| p95_latency_ms | 49.3113 |
| positive_hit_rate | 1 |
| positive_mean_mrr | 1 |
| positive_mean_ndcg_at_5 | 0.9609 |
| positive_mean_precision_at_10 | 0.1833 |
| positive_mean_precision_at_5 | 0.3667 |
| positive_mean_recall_at_5 | 0.9444 |
| positive_query_count | 6 |
| precision_at_10 | 0.1571 |
| precision_at_5 | 0.3143 |
| query_count | 7 |
| readiness_fallback | {"classification": "degraded", "elapsed_s": 7.019, "explanation": "Readiness used fallback settling because no direct ready signal was available; results can still be valid but should be reviewed with elapsed time.", "fallback_sleep_s": 3.0, "fallback_used": true, "impact": "degraded", "poll_attempts": 3, "readiness_signal": "none", "reason": "no_direct_readiness_signal", "status": "fallback_after_no_signal"} |
| reason_code_classification | {} |
| recall_at_10 | 0.8095 |
| recall_at_5 | 0.8095 |
| runtime_minutes | 1.1861 |
| seed_completed | True |
| threshold_evaluation | {"enforcement": "local-only", "evaluated_metrics": 8, "failure_counts": {"blocker": 0, "warn": 0}, "failures": [], "fixture_tier": "medium", "policy_name": "local-v2-threshold-policy", "reason": "all required threshold checks passed", "status": "pass"} |

## Readiness fallback

- Status: `fallback_after_no_signal`
- Impact: `degraded`
- Elapsed (s): `7.019`
- Fallback used: `True`
- Explanation: Readiness used fallback settling because no direct ready signal was available; results can still be valid but should be reviewed with elapsed time.

## Per-query metrics

| Query | Rank | MRR | R@5 | R@10 | NDCG@5 | NDCG@10 | P@5 | P@10 | Latency ms | Failure | Top-1 |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|---|
| medium_q_long_memory_aurora_retention_context | 1 | 1 | 0.6667 | 0.6667 | 0.7654 | 0.7654 | 0.4 | 0.2 | 49.3113 | none | bdc1ffa846bab4f8853f (medium_mem_decision_aurora_archive_partitions) |
| medium_q_namespace_boundary_vega_only | 1 | 1 | 1 | 1 | 1 | 1 | 0.4 | 0.2 | 15.1273 | none | 4eb02a3241be05db03da (medium_mem_task_vega_timeout_regressions) |
| medium_q_temporal_boundary_post_cutover | 1 | 1 | 1 | 1 | 1 | 1 | 0.2 | 0.1 | 0.8923 | none | a3e44c80a98fd612d178 (medium_mem_decision_lumen_parser_fallback_removed) |
| medium_q_negative_no_match_synthetic_nonsense | — | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 16.6752 | wrong_rank | a3e44c80a98fd612d178 (medium_mem_decision_lumen_parser_fallback_removed) |
| medium_q_partial_readiness_contract_reason_codes | 1 | 1 | 1 | 1 | 1 | 1 | 0.4 | 0.2 | 1.2079 | none | e0f5eb386bf088fcc532 (medium_mem_research_hermes_reason_codes) |
| medium_q_id_alias_mapping_fixture_vs_server | 1 | 1 | 1 | 1 | 1 | 1 | 0.4 | 0.2 | 17.6433 | none | 4dae26e937679905b7dd (medium_mem_task_id_alias_mapping) |
| medium_q_record_shaped_id_normalization | 1 | 1 | 1 | 1 | 1 | 1 | 0.4 | 0.2 | 19.1524 | none | 95a2494df7ba7382cf3f (medium_mem_research_record_id_normalization) |
