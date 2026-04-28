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
- readiness_fallback: `{"classification": "degraded", "elapsed_s": 7.01, "explanation": "Readiness used fallback settling because no direct ready signal was available; results can still be valid but should be reviewed with elapsed time.", "fallback_sleep_s": 3.0, "fallback_used": true, "impact": "degraded", "poll_attempts": 3, "readiness_signal": "none", "reason": "no_direct_readiness_signal", "status": "fallback_after_no_signal"}`

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
| precision_at_5 | 0.3429 |
| precision_at_10 | 0.1714 |
| recall_at_5 | 0.8571 |
| recall_at_10 | 0.8571 |
| ndcg_at_5 | 0.8571 |
| ndcg_at_10 | 0.8571 |
| mean_latency_ms | 14.8045 |
| max_latency_ms | 27.6828 |
| p95_latency_ms | 27.6828 |
| blocker_count | 0 |
| positive_query_count | 6 |
| positive_hit_rate | 1 |
| positive_mean_mrr | 1 |
| positive_mean_recall_at_5 | 1 |
| positive_mean_ndcg_at_5 | 1 |
| positive_mean_precision_at_5 | 0.4 |
| runtime_minutes | 1.049 |

### Deterministic / local-only metadata

- threshold_policy_enforcement: `local-only`
- determinism_policy: `{"name": "stable_fixture_order+stable_tie_break+stable_report_order+tolerance_1e-9_1e-6"}`
- runtime_target: `{"optional_policy": "explicit medium-tier run", "required_by_default": false, "target_minutes": "15-30"}`

## Run context

- Command used: `/Users/xiayiming1/Documents/Workspace/memory-mcp-1file/target/fast/memory-mcp --stdio`
- Embedding model: `e5_small`
- Data dir: `/var/folders/mx/tffz_j1d5lg_twyfntz97yr00000gp/T/task-8-memory-retrieval-data-0r4rbvzy`
- Started at: `2026-04-28T12:58:21.738151+00:00`
- Duration (s): `62.96`

### stderr tail
- [memory-mcp] Auto-configured block cache: 16 MB (available RAM: 81 MB)
- [2m2026-04-28T12:58:21.750978Z[0m [32m INFO[0m [2mmemory_mcp[0m[2m:[0m memory-mcp starting [3mversion[0m[2m=[0m"0.8.2" [3mpid[0m[2m=[0m61690 [3mppid[0m[2m=[0m61688 [3mmode[0m[2m=[0m"stdio" [3mmodel[0m[2m=[0me5_small [3mdata_dir[0m[2m=[0m/var/folders/mx/tffz_j1d5lg_twyfntz97yr00000gp/T/task-8-memory-retrieval-data-0r4rbvzy
- [2m2026-04-28T12:58:21.751315Z[0m [32m INFO[0m [2msurrealdb::core::kvs::ds[0m[2m:[0m Starting kvs store at absolute path surrealkv:/var/folders/mx/tffz_j1d5lg_twyfntz97yr00000gp/T/task-8-memory-retrieval-data-0r4rbvzy/db
- [2m2026-04-28T12:58:21.751480Z[0m [32m INFO[0m [2msurrealdb::core::kvs::surrealkv[0m[2m:[0m Enabling value log separation: true
- [2m2026-04-28T12:58:21.761051Z[0m [32m INFO[0m [2msurrealdb::core::kvs::surrealkv[0m[2m:[0m Setting value log max file size: 268435456
- [2m2026-04-28T12:58:21.761064Z[0m [32m INFO[0m [2msurrealdb::core::kvs::surrealkv[0m[2m:[0m Setting value log threshold: 4096
- [2m2026-04-28T12:58:21.761071Z[0m [32m INFO[0m [2msurrealdb::core::kvs::surrealkv[0m[2m:[0m Versioning enabled: false with retention period: 0ns
- [2m2026-04-28T12:58:21.761072Z[0m [32m INFO[0m [2msurrealdb::core::kvs::surrealkv[0m[2m:[0m Versioning with versioned_index: false
- [2m2026-04-28T12:58:21.761075Z[0m [32m INFO[0m [2msurrealdb::core::kvs::surrealkv[0m[2m:[0m Setting block cache capacity: 16777216
- [2m2026-04-28T12:58:21.761083Z[0m [32m INFO[0m [2msurrealdb::core::kvs::surrealkv[0m[2m:[0m Setting block size: 65536
- [2m2026-04-28T12:58:21.762534Z[0m [32m INFO[0m [2msurrealkv::lsm[0m[2m:[0m === Starting LSM tree initialization ===
- [2m2026-04-28T12:58:21.762541Z[0m [32m INFO[0m [2msurrealkv::lsm[0m[2m:[0m Database path: "/var/folders/mx/tffz_j1d5lg_twyfntz97yr00000gp/T/task-8-memory-retrieval-data-0r4rbvzy/db"
- [2m2026-04-28T12:58:21.795183Z[0m [32m INFO[0m [2msurrealkv::lsm[0m[2m:[0m Manifest state: log_number=0, last_sequence=0
- [2m2026-04-28T12:58:21.795194Z[0m [32m INFO[0m [2msurrealkv::wal::recovery[0m[2m:[0m Starting WAL recovery from directory: "/var/folders/mx/tffz_j1d5lg_twyfntz97yr00000gp/T/task-8-memory-retrieval-data-0r4rbvzy/db/wal"
- [2m2026-04-28T12:58:21.795341Z[0m [32m INFO[0m [2msurrealkv::wal::recovery[0m[2m:[0m Replaying WAL segments #00000000000000000000 to #00000000000000000000
- [2m2026-04-28T12:58:21.817509Z[0m [32m INFO[0m [2msurrealkv::wal::recovery[0m[2m:[0m WAL recovery complete: 0 batches across 0 segments, 0 memtables created, max_seq_num=None
- [2m2026-04-28T12:58:21.818089Z[0m [32m INFO[0m [2msurrealkv::lsm[0m[2m:[0m === LSM tree initialization complete ===
- [2m2026-04-28T12:58:21.825994Z[0m [32m INFO[0m [2msurrealdb::core::kvs::surrealkv[0m[2m:[0m Sync mode: every transaction commit
- [2m2026-04-28T12:58:21.826013Z[0m [32m INFO[0m [2msurrealdb::core::kvs::surrealkv[0m[2m:[0m Grouped commit: enabled (timeout=5000000ns, wait_threshold=12, max_batch_size=4096)
- [2m2026-04-28T12:58:21.826037Z[0m [32m INFO[0m [2msurrealdb::core::kvs::ds[0m[2m:[0m Started surrealkv kvs store
- [2m2026-04-28T12:58:22.460294Z[0m [32m INFO[0m [2mmemory_mcp::storage::surrealdb[0m[2m:[0m Dimension check passed [3mmodel[0m[2m=[0m384 [3mdb[0m[2m=[0m384
- [2m2026-04-28T12:58:22.460320Z[0m [32m INFO[0m [2mmemory_mcp[0m[2m:[0m Embedding engine configured [3moutput_dim[0m[2m=[0m384 [3mmodel[0m[2m=[0me5_small
- [2m2026-04-28T12:58:22.489705Z[0m [32m INFO[0m [2mmemory_mcp::embedding::service[0m[2m:[0m Loading embedding model: E5Small
- [2m2026-04-28T12:58:22.489703Z[0m [32m INFO[0m [2mmemory_mcp::embedding::worker[0m[2m:[0m Embedding worker started, waiting for requests
- [2m2026-04-28T12:58:22.489703Z[0m [32m INFO[0m [2mmemory_mcp::forgetting::capacity[0m[2m:[0m Capacity controller started [3mcheck_interval_secs[0m[2m=[0m3600 [3msoft_limit[0m[2m=[0m10000 [3mcleanup_target_ratio[0m[2m=[0m0.800000011920929
- [2m2026-04-28T12:58:22.490384Z[0m [32m INFO[0m [2mrmcp::handler::server[0m[2m:[0m client initialized
- [2m2026-04-28T12:58:22.490400Z[0m [32m INFO[0m [1mserve_inner[0m[2m:[0m [2mrmcp::service[0m[2m:[0m Service initialized as server [3mpeer_info[0m[2m=[0mSome(InitializeRequestParams { meta: None, protocol_version: ProtocolVersion("2024-11-05"), capabilities: ClientCapabilities { experimental: None, extensions: None, roots: None, sampling: None, elicitation: None, tasks: None }, client_info: Implementation { name: "memory-retrieval-benchmark", title: None, version: "0.1.0", description: None, icons: None, website_url: None } })
- [2m2026-04-28T12:58:22.490413Z[0m [32m INFO[0m [2mmemory_mcp[0m[2m:[0m Server started in stdio mode, waiting for client disconnect or signals...
- [2m2026-04-28T12:58:22.491154Z[0m [32m INFO[0m [2mmemory_mcp::forgetting::capacity[0m[2m:[0m Capacity OK: 0 memories [3mcount[0m[2m=[0m0
- [2m2026-04-28T12:58:29.010747Z[0m [32m INFO[0m [2mmemory_mcp::embedding::service[0m[2m:[0m Downloading model weights from HuggingFace Hub...
- [2m2026-04-28T12:59:15.447805Z[0m [32m INFO[0m [2mmemory_mcp::embedding::service[0m[2m:[0m Embedding model ready [3melapsed_sec[0m[2m=[0m"53.0"

## Aggregate metrics

| Metric | Value |
|---|---:|
| baseline_query_count | 7 |
| blocker_count | 0 |
| hit_rate | 0.8571 |
| latency_summary | {"count": 7, "max_latency_ms": 27.682750000003864, "mean_latency_ms": 14.804547428571116, "p95_latency_ms": 27.682750000003864} |
| max_latency_ms | 27.6828 |
| mean_expected_rank | 1 |
| mean_latency_ms | 14.8045 |
| mrr | 0.8571 |
| ndcg_at_10 | 0.8571 |
| ndcg_at_5 | 0.8571 |
| observed_summary_partial_reason_codes | [] |
| p95_latency_ms | 27.6828 |
| positive_hit_rate | 1 |
| positive_mean_mrr | 1 |
| positive_mean_ndcg_at_5 | 1 |
| positive_mean_precision_at_10 | 0.2 |
| positive_mean_precision_at_5 | 0.4 |
| positive_mean_recall_at_5 | 1 |
| positive_query_count | 6 |
| precision_at_10 | 0.1714 |
| precision_at_5 | 0.3429 |
| query_count | 7 |
| readiness_fallback | {"classification": "degraded", "elapsed_s": 7.01, "explanation": "Readiness used fallback settling because no direct ready signal was available; results can still be valid but should be reviewed with elapsed time.", "fallback_sleep_s": 3.0, "fallback_used": true, "impact": "degraded", "poll_attempts": 3, "readiness_signal": "none", "reason": "no_direct_readiness_signal", "status": "fallback_after_no_signal"} |
| reason_code_classification | {} |
| recall_at_10 | 0.8571 |
| recall_at_5 | 0.8571 |
| runtime_minutes | 1.049 |
| seed_completed | True |
| threshold_evaluation | {"enforcement": "local-only", "evaluated_metrics": 8, "failure_counts": {"blocker": 0, "warn": 0}, "failures": [], "fixture_tier": "medium", "policy_name": "local-v2-threshold-policy", "reason": "all required threshold checks passed", "status": "pass"} |

## Readiness fallback

- Status: `fallback_after_no_signal`
- Impact: `degraded`
- Elapsed (s): `7.01`
- Fallback used: `True`
- Explanation: Readiness used fallback settling because no direct ready signal was available; results can still be valid but should be reviewed with elapsed time.

## Per-query metrics

| Query | Rank | MRR | R@5 | R@10 | NDCG@5 | NDCG@10 | P@5 | P@10 | Latency ms | Failure | Top-1 |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|---|
| medium_q_long_memory_aurora_retention_context | 1 | 1 | 1 | 1 | 1 | 1 | 0.6 | 0.3 | 27.6828 | none | bd35a5c5496cf1427575 (medium_mem_task_aurora_retention_snapshots) |
| medium_q_namespace_boundary_vega_only | 1 | 1 | 1 | 1 | 1 | 1 | 0.4 | 0.2 | 15.7812 | none | 64a7b4f4b9db6290e68c (medium_mem_task_vega_timeout_regressions) |
| medium_q_temporal_boundary_post_cutover | 1 | 1 | 1 | 1 | 1 | 1 | 0.2 | 0.1 | 0.755 | none | f2a8939317396ec099a7 (medium_mem_decision_lumen_parser_fallback_removed) |
| medium_q_negative_no_match_synthetic_nonsense | — | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 17.8073 | wrong_rank | f2a8939317396ec099a7 (medium_mem_decision_lumen_parser_fallback_removed) |
| medium_q_partial_readiness_contract_reason_codes | 1 | 1 | 1 | 1 | 1 | 1 | 0.4 | 0.2 | 1.025 | none | 6d5508448e943cd1435b (medium_mem_research_hermes_reason_codes) |
| medium_q_id_alias_mapping_fixture_vs_server | 1 | 1 | 1 | 1 | 1 | 1 | 0.4 | 0.2 | 19.5626 | none | 4a72bab1826163ffa42a (medium_mem_task_id_alias_mapping) |
| medium_q_record_shaped_id_normalization | 1 | 1 | 1 | 1 | 1 | 1 | 0.4 | 0.2 | 21.018 | none | f98bc5a535a1187360b9 (medium_mem_research_record_id_normalization) |
