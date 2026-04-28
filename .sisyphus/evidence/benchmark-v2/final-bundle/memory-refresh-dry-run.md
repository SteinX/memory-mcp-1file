# Memory Retrieval Baseline

## Benchmark V2 summary

| Field | Value |
|---|---|
| schema_version | `2.0` |
| fixture_tier | `small` |
| baseline_version | `v2-initial` |
| threshold_policy | `local-v2-threshold-policy` |
| threshold_status | `pass` |
| threshold_status_reason | `all required threshold checks passed` |

### Readiness summary

- reason_codes: `[]`
- reason_code_classification: `{}`
- readiness_fallback: `{"classification": "degraded", "elapsed_s": 7.021, "explanation": "Readiness used fallback settling because no direct ready signal was available; results can still be valid but should be reviewed with elapsed time.", "fallback_sleep_s": 3.0, "fallback_used": true, "impact": "degraded", "poll_attempts": 3, "readiness_signal": "none", "reason": "no_direct_readiness_signal", "status": "fallback_after_no_signal"}`

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
| precision_at_5 | 0.2857 |
| precision_at_10 | 0.1429 |
| recall_at_5 | 0.7857 |
| recall_at_10 | 0.7857 |
| ndcg_at_5 | 0.8019 |
| ndcg_at_10 | 0.8019 |
| mean_latency_ms | 11.013 |
| max_latency_ms | 29.6946 |
| p95_latency_ms | 29.6946 |
| blocker_count | 0 |
| positive_query_count | 6 |
| positive_hit_rate | 1 |
| positive_mean_mrr | 1 |
| positive_mean_recall_at_5 | 0.9167 |
| positive_mean_ndcg_at_5 | 0.9355 |
| positive_mean_precision_at_5 | 0.3333 |
| runtime_minutes | 1.0486 |

### Deterministic / local-only metadata

- threshold_policy_enforcement: `local-only`
- determinism_policy: `{"name": "stable_fixture_order+stable_tie_break+stable_report_order+tolerance_1e-9_1e-6"}`
- runtime_target: `{"optional_policy": "small tier default", "required_by_default": true, "target_minutes": "5-10"}`

## Run context

- Command used: `/Users/xiayiming1/Documents/Workspace/memory-mcp-1file/target/fast/memory-mcp --stdio`
- Embedding model: `e5_small`
- Data dir: `/var/folders/mx/tffz_j1d5lg_twyfntz97yr00000gp/T/task-8-memory-retrieval-data-9s7ftrat`
- Started at: `2026-04-28T12:34:34.118573+00:00`
- Duration (s): `62.94`

### stderr tail
- [memory-mcp] Auto-configured block cache: 12 MB (available RAM: 60 MB)
- [2m2026-04-28T12:34:34.147854Z[0m [32m INFO[0m [2mmemory_mcp[0m[2m:[0m memory-mcp starting [3mversion[0m[2m=[0m"0.8.2" [3mpid[0m[2m=[0m17965 [3mppid[0m[2m=[0m17964 [3mmode[0m[2m=[0m"stdio" [3mmodel[0m[2m=[0me5_small [3mdata_dir[0m[2m=[0m/var/folders/mx/tffz_j1d5lg_twyfntz97yr00000gp/T/task-8-memory-retrieval-data-9s7ftrat
- [2m2026-04-28T12:34:34.153922Z[0m [32m INFO[0m [2msurrealdb::core::kvs::ds[0m[2m:[0m Starting kvs store at absolute path surrealkv:/var/folders/mx/tffz_j1d5lg_twyfntz97yr00000gp/T/task-8-memory-retrieval-data-9s7ftrat/db
- [2m2026-04-28T12:34:34.157280Z[0m [32m INFO[0m [2msurrealdb::core::kvs::surrealkv[0m[2m:[0m Enabling value log separation: true
- [2m2026-04-28T12:34:34.170572Z[0m [32m INFO[0m [2msurrealdb::core::kvs::surrealkv[0m[2m:[0m Setting value log max file size: 268435456
- [2m2026-04-28T12:34:34.170585Z[0m [32m INFO[0m [2msurrealdb::core::kvs::surrealkv[0m[2m:[0m Setting value log threshold: 4096
- [2m2026-04-28T12:34:34.170586Z[0m [32m INFO[0m [2msurrealdb::core::kvs::surrealkv[0m[2m:[0m Versioning enabled: false with retention period: 0ns
- [2m2026-04-28T12:34:34.170587Z[0m [32m INFO[0m [2msurrealdb::core::kvs::surrealkv[0m[2m:[0m Versioning with versioned_index: false
- [2m2026-04-28T12:34:34.170756Z[0m [32m INFO[0m [2msurrealdb::core::kvs::surrealkv[0m[2m:[0m Setting block cache capacity: 12582912
- [2m2026-04-28T12:34:34.170816Z[0m [32m INFO[0m [2msurrealdb::core::kvs::surrealkv[0m[2m:[0m Setting block size: 65536
- [2m2026-04-28T12:34:34.171541Z[0m [32m INFO[0m [2msurrealkv::lsm[0m[2m:[0m === Starting LSM tree initialization ===
- [2m2026-04-28T12:34:34.171964Z[0m [32m INFO[0m [2msurrealkv::lsm[0m[2m:[0m Database path: "/var/folders/mx/tffz_j1d5lg_twyfntz97yr00000gp/T/task-8-memory-retrieval-data-9s7ftrat/db"
- [2m2026-04-28T12:34:34.198124Z[0m [32m INFO[0m [2msurrealkv::lsm[0m[2m:[0m Manifest state: log_number=0, last_sequence=0
- [2m2026-04-28T12:34:34.198133Z[0m [32m INFO[0m [2msurrealkv::wal::recovery[0m[2m:[0m Starting WAL recovery from directory: "/var/folders/mx/tffz_j1d5lg_twyfntz97yr00000gp/T/task-8-memory-retrieval-data-9s7ftrat/db/wal"
- [2m2026-04-28T12:34:34.198203Z[0m [32m INFO[0m [2msurrealkv::wal::recovery[0m[2m:[0m Replaying WAL segments #00000000000000000000 to #00000000000000000000
- [2m2026-04-28T12:34:34.224635Z[0m [32m INFO[0m [2msurrealkv::wal::recovery[0m[2m:[0m WAL recovery complete: 0 batches across 0 segments, 0 memtables created, max_seq_num=None
- [2m2026-04-28T12:34:34.224939Z[0m [32m INFO[0m [2msurrealkv::lsm[0m[2m:[0m === LSM tree initialization complete ===
- [2m2026-04-28T12:34:34.243270Z[0m [32m INFO[0m [2msurrealdb::core::kvs::surrealkv[0m[2m:[0m Sync mode: every transaction commit
- [2m2026-04-28T12:34:34.243474Z[0m [32m INFO[0m [2msurrealdb::core::kvs::surrealkv[0m[2m:[0m Grouped commit: enabled (timeout=5000000ns, wait_threshold=12, max_batch_size=4096)
- [2m2026-04-28T12:34:34.243638Z[0m [32m INFO[0m [2msurrealdb::core::kvs::ds[0m[2m:[0m Started surrealkv kvs store
- [2m2026-04-28T12:34:34.955357Z[0m [32m INFO[0m [2mmemory_mcp::storage::surrealdb[0m[2m:[0m Dimension check passed [3mmodel[0m[2m=[0m384 [3mdb[0m[2m=[0m384
- [2m2026-04-28T12:34:34.955375Z[0m [32m INFO[0m [2mmemory_mcp[0m[2m:[0m Embedding engine configured [3moutput_dim[0m[2m=[0m384 [3mmodel[0m[2m=[0me5_small
- [2m2026-04-28T12:34:34.988981Z[0m [32m INFO[0m [2mmemory_mcp::embedding::service[0m[2m:[0m Loading embedding model: E5Small
- [2m2026-04-28T12:34:34.990854Z[0m [32m INFO[0m [2mmemory_mcp::forgetting::capacity[0m[2m:[0m Capacity controller started [3mcheck_interval_secs[0m[2m=[0m3600 [3msoft_limit[0m[2m=[0m10000 [3mcleanup_target_ratio[0m[2m=[0m0.800000011920929
- [2m2026-04-28T12:34:34.991086Z[0m [32m INFO[0m [2mmemory_mcp::embedding::worker[0m[2m:[0m Embedding worker started, waiting for requests
- [2m2026-04-28T12:34:34.997390Z[0m [32m INFO[0m [2mmemory_mcp::forgetting::capacity[0m[2m:[0m Capacity OK: 0 memories [3mcount[0m[2m=[0m0
- [2m2026-04-28T12:34:34.998526Z[0m [32m INFO[0m [2mrmcp::handler::server[0m[2m:[0m client initialized
- [2m2026-04-28T12:34:34.998543Z[0m [32m INFO[0m [1mserve_inner[0m[2m:[0m [2mrmcp::service[0m[2m:[0m Service initialized as server [3mpeer_info[0m[2m=[0mSome(InitializeRequestParams { meta: None, protocol_version: ProtocolVersion("2024-11-05"), capabilities: ClientCapabilities { experimental: None, extensions: None, roots: None, sampling: None, elicitation: None, tasks: None }, client_info: Implementation { name: "memory-retrieval-benchmark", title: None, version: "0.1.0", description: None, icons: None, website_url: None } })
- [2m2026-04-28T12:34:34.998579Z[0m [32m INFO[0m [2mmemory_mcp[0m[2m:[0m Server started in stdio mode, waiting for client disconnect or signals...
- [2m2026-04-28T12:34:41.253266Z[0m [32m INFO[0m [2mmemory_mcp::embedding::service[0m[2m:[0m Downloading model weights from HuggingFace Hub...
- [2m2026-04-28T12:35:28.942383Z[0m [32m INFO[0m [2mmemory_mcp::embedding::service[0m[2m:[0m Embedding model ready [3melapsed_sec[0m[2m=[0m"54.0"

## Aggregate metrics

| Metric | Value |
|---|---:|
| baseline_query_count | 7 |
| blocker_count | 0 |
| hit_rate | 0.8571 |
| latency_summary | {"count": 7, "max_latency_ms": 29.694624999997643, "mean_latency_ms": 11.01297600000046, "p95_latency_ms": 29.694624999997643} |
| max_latency_ms | 29.6946 |
| mean_expected_rank | 1 |
| mean_latency_ms | 11.013 |
| mrr | 0.8571 |
| ndcg_at_10 | 0.8019 |
| ndcg_at_5 | 0.8019 |
| observed_summary_partial_reason_codes | [] |
| p95_latency_ms | 29.6946 |
| positive_hit_rate | 1 |
| positive_mean_mrr | 1 |
| positive_mean_ndcg_at_5 | 0.9355 |
| positive_mean_precision_at_10 | 0.1667 |
| positive_mean_precision_at_5 | 0.3333 |
| positive_mean_recall_at_5 | 0.9167 |
| positive_query_count | 6 |
| precision_at_10 | 0.1429 |
| precision_at_5 | 0.2857 |
| query_count | 7 |
| readiness_fallback | {"classification": "degraded", "elapsed_s": 7.021, "explanation": "Readiness used fallback settling because no direct ready signal was available; results can still be valid but should be reviewed with elapsed time.", "fallback_sleep_s": 3.0, "fallback_used": true, "impact": "degraded", "poll_attempts": 3, "readiness_signal": "none", "reason": "no_direct_readiness_signal", "status": "fallback_after_no_signal"} |
| reason_code_classification | {} |
| recall_at_10 | 0.7857 |
| recall_at_5 | 0.7857 |
| runtime_minutes | 1.0486 |
| seed_completed | True |
| threshold_evaluation | {"enforcement": "local-only", "evaluated_metrics": 8, "failure_counts": {"blocker": 0, "warn": 0}, "failures": [], "fixture_tier": "small", "policy_name": "local-v2-threshold-policy", "reason": "all required threshold checks passed", "status": "pass"} |

## Readiness fallback

- Status: `fallback_after_no_signal`
- Impact: `degraded`
- Elapsed (s): `7.021`
- Fallback used: `True`
- Explanation: Readiness used fallback settling because no direct ready signal was available; results can still be valid but should be reviewed with elapsed time.

## Per-query metrics

| Query | Rank | MRR | R@5 | R@10 | NDCG@5 | NDCG@10 | P@5 | P@10 | Latency ms | Failure | Top-1 |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|---|
| mini_q_recall_checkout_retry_backoff | 1 | 1 | 1 | 1 | 1 | 1 | 0.4 | 0.2 | 29.6946 | none | 354486fb548006c3f375 (mini_mem_task_checkout_retry) |
| mini_q_search_bm25_refund_ops | 1 | 1 | 1 | 1 | 1 | 1 | 0.4 | 0.2 | 1.0218 | none | 0e8578862f0a65a48c28 (mini_mem_research_refund_notifications) |
| mini_q_search_vector_helios_stability | 1 | 1 | 0.5 | 0.5 | 0.6131 | 0.6131 | 0.2 | 0.1 | 14.2229 | none | cbffba65c544c0658c76 (mini_mem_task_helios_crash_review) |
| mini_q_get_valid_checkout_namespace | 1 | 1 | 1 | 1 | 1 | 1 | 0.4 | 0.2 | 0.7226 | none | 09f966bf9f5f8503acfb (mini_mem_decision_checkout_backoff) |
| mini_q_get_valid_temporal_post_migration | 1 | 1 | 1 | 1 | 1 | 1 | 0.2 | 0.1 | 0.5004 | none | 0041689079f812d8cada (mini_mem_decision_parser_fallback_removed) |
| mini_q_recall_person_project_topic | 1 | 1 | 1 | 1 | 1 | 1 | 0.4 | 0.2 | 14.2973 | none | a7f3d086286702eb5dea (mini_mem_user_pref_release_notes) |
| mini_q_negative_no_match_nonsense | — | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 16.6312 | wrong_rank | 53ad68b569bdd637d712 (mini_mem_context_process_namespace) |
