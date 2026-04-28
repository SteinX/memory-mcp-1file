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
- readiness_fallback: `{"classification": "degraded", "elapsed_s": 7.013, "explanation": "Readiness used fallback settling because no direct ready signal was available; results can still be valid but should be reviewed with elapsed time.", "fallback_sleep_s": 3.0, "fallback_used": true, "impact": "degraded", "poll_attempts": 3, "readiness_signal": "none", "reason": "no_direct_readiness_signal", "status": "fallback_after_no_signal"}`

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
| precision_at_10 | 0.1571 |
| recall_at_5 | 0.7857 |
| recall_at_10 | 0.8571 |
| ndcg_at_5 | 0.8019 |
| ndcg_at_10 | 0.8272 |
| mean_latency_ms | 11.4403 |
| max_latency_ms | 30.894 |
| p95_latency_ms | 30.894 |
| blocker_count | 0 |
| positive_query_count | 6 |
| positive_hit_rate | 1 |
| positive_mean_mrr | 1 |
| positive_mean_recall_at_5 | 0.9167 |
| positive_mean_ndcg_at_5 | 0.9355 |
| positive_mean_precision_at_5 | 0.3333 |
| runtime_minutes | 1.0487 |

### Deterministic / local-only metadata

- threshold_policy_enforcement: `local-only`
- determinism_policy: `{"name": "stable_fixture_order+stable_tie_break+stable_report_order+tolerance_1e-9_1e-6"}`
- runtime_target: `{"optional_policy": "small tier default", "required_by_default": true, "target_minutes": "5-10"}`

## Run context

- Command used: `/Users/xiayiming1/Documents/Workspace/memory-mcp-1file/target/fast/memory-mcp --stdio`
- Embedding model: `e5_small`
- Data dir: `/var/folders/mx/tffz_j1d5lg_twyfntz97yr00000gp/T/task-8-memory-retrieval-data-zz1nlj3u`
- Started at: `2026-04-28T11:54:16.926324+00:00`
- Duration (s): `62.94`

### stderr tail
- [memory-mcp] Auto-configured block cache: 18 MB (available RAM: 90 MB)
- [2m2026-04-28T11:54:16.962912Z[0m [32m INFO[0m [2mmemory_mcp[0m[2m:[0m memory-mcp starting [3mversion[0m[2m=[0m"0.8.2" [3mpid[0m[2m=[0m36632 [3mppid[0m[2m=[0m36628 [3mmode[0m[2m=[0m"stdio" [3mmodel[0m[2m=[0me5_small [3mdata_dir[0m[2m=[0m/var/folders/mx/tffz_j1d5lg_twyfntz97yr00000gp/T/task-8-memory-retrieval-data-zz1nlj3u
- [2m2026-04-28T11:54:16.968853Z[0m [32m INFO[0m [2msurrealdb::core::kvs::ds[0m[2m:[0m Starting kvs store at absolute path surrealkv:/var/folders/mx/tffz_j1d5lg_twyfntz97yr00000gp/T/task-8-memory-retrieval-data-zz1nlj3u/db
- [2m2026-04-28T11:54:16.971921Z[0m [32m INFO[0m [2msurrealdb::core::kvs::surrealkv[0m[2m:[0m Enabling value log separation: true
- [2m2026-04-28T11:54:16.981216Z[0m [32m INFO[0m [2msurrealdb::core::kvs::surrealkv[0m[2m:[0m Setting value log max file size: 268435456
- [2m2026-04-28T11:54:16.981224Z[0m [32m INFO[0m [2msurrealdb::core::kvs::surrealkv[0m[2m:[0m Setting value log threshold: 4096
- [2m2026-04-28T11:54:16.981225Z[0m [32m INFO[0m [2msurrealdb::core::kvs::surrealkv[0m[2m:[0m Versioning enabled: false with retention period: 0ns
- [2m2026-04-28T11:54:16.981226Z[0m [32m INFO[0m [2msurrealdb::core::kvs::surrealkv[0m[2m:[0m Versioning with versioned_index: false
- [2m2026-04-28T11:54:16.981383Z[0m [32m INFO[0m [2msurrealdb::core::kvs::surrealkv[0m[2m:[0m Setting block cache capacity: 18874368
- [2m2026-04-28T11:54:16.981600Z[0m [32m INFO[0m [2msurrealdb::core::kvs::surrealkv[0m[2m:[0m Setting block size: 65536
- [2m2026-04-28T11:54:16.982329Z[0m [32m INFO[0m [2msurrealkv::lsm[0m[2m:[0m === Starting LSM tree initialization ===
- [2m2026-04-28T11:54:16.982540Z[0m [32m INFO[0m [2msurrealkv::lsm[0m[2m:[0m Database path: "/var/folders/mx/tffz_j1d5lg_twyfntz97yr00000gp/T/task-8-memory-retrieval-data-zz1nlj3u/db"
- [2m2026-04-28T11:54:17.005908Z[0m [32m INFO[0m [2msurrealkv::lsm[0m[2m:[0m Manifest state: log_number=0, last_sequence=0
- [2m2026-04-28T11:54:17.005917Z[0m [32m INFO[0m [2msurrealkv::wal::recovery[0m[2m:[0m Starting WAL recovery from directory: "/var/folders/mx/tffz_j1d5lg_twyfntz97yr00000gp/T/task-8-memory-retrieval-data-zz1nlj3u/db/wal"
- [2m2026-04-28T11:54:17.006000Z[0m [32m INFO[0m [2msurrealkv::wal::recovery[0m[2m:[0m Replaying WAL segments #00000000000000000000 to #00000000000000000000
- [2m2026-04-28T11:54:17.026271Z[0m [32m INFO[0m [2msurrealkv::wal::recovery[0m[2m:[0m WAL recovery complete: 0 batches across 0 segments, 0 memtables created, max_seq_num=None
- [2m2026-04-28T11:54:17.026721Z[0m [32m INFO[0m [2msurrealkv::lsm[0m[2m:[0m === LSM tree initialization complete ===
- [2m2026-04-28T11:54:17.038902Z[0m [32m INFO[0m [2msurrealdb::core::kvs::surrealkv[0m[2m:[0m Sync mode: every transaction commit
- [2m2026-04-28T11:54:17.039207Z[0m [32m INFO[0m [2msurrealdb::core::kvs::surrealkv[0m[2m:[0m Grouped commit: enabled (timeout=5000000ns, wait_threshold=12, max_batch_size=4096)
- [2m2026-04-28T11:54:17.039460Z[0m [32m INFO[0m [2msurrealdb::core::kvs::ds[0m[2m:[0m Started surrealkv kvs store
- [2m2026-04-28T11:54:17.746253Z[0m [32m INFO[0m [2mmemory_mcp::storage::surrealdb[0m[2m:[0m Dimension check passed [3mmodel[0m[2m=[0m384 [3mdb[0m[2m=[0m384
- [2m2026-04-28T11:54:17.746265Z[0m [32m INFO[0m [2mmemory_mcp[0m[2m:[0m Embedding engine configured [3moutput_dim[0m[2m=[0m384 [3mmodel[0m[2m=[0me5_small
- [2m2026-04-28T11:54:17.775406Z[0m [32m INFO[0m [2mmemory_mcp::forgetting::capacity[0m[2m:[0m Capacity controller started [3mcheck_interval_secs[0m[2m=[0m3600 [3msoft_limit[0m[2m=[0m10000 [3mcleanup_target_ratio[0m[2m=[0m0.800000011920929
- [2m2026-04-28T11:54:17.775525Z[0m [32m INFO[0m [2mmemory_mcp::embedding::worker[0m[2m:[0m Embedding worker started, waiting for requests
- [2m2026-04-28T11:54:17.775646Z[0m [32m INFO[0m [2mmemory_mcp::embedding::service[0m[2m:[0m Loading embedding model: E5Small
- [2m2026-04-28T11:54:17.777284Z[0m [32m INFO[0m [2mrmcp::handler::server[0m[2m:[0m client initialized
- [2m2026-04-28T11:54:17.777296Z[0m [32m INFO[0m [1mserve_inner[0m[2m:[0m [2mrmcp::service[0m[2m:[0m Service initialized as server [3mpeer_info[0m[2m=[0mSome(InitializeRequestParams { meta: None, protocol_version: ProtocolVersion("2024-11-05"), capabilities: ClientCapabilities { experimental: None, extensions: None, roots: None, sampling: None, elicitation: None, tasks: None }, client_info: Implementation { name: "memory-retrieval-benchmark", title: None, version: "0.1.0", description: None, icons: None, website_url: None } })
- [2m2026-04-28T11:54:17.777317Z[0m [32m INFO[0m [2mmemory_mcp[0m[2m:[0m Server started in stdio mode, waiting for client disconnect or signals...
- [2m2026-04-28T11:54:17.779780Z[0m [32m INFO[0m [2mmemory_mcp::forgetting::capacity[0m[2m:[0m Capacity OK: 0 memories [3mcount[0m[2m=[0m0
- [2m2026-04-28T11:54:24.650206Z[0m [32m INFO[0m [2mmemory_mcp::embedding::service[0m[2m:[0m Downloading model weights from HuggingFace Hub...
- [2m2026-04-28T11:55:10.473969Z[0m [32m INFO[0m [2mmemory_mcp::embedding::service[0m[2m:[0m Embedding model ready [3melapsed_sec[0m[2m=[0m"52.7"

## Aggregate metrics

| Metric | Value |
|---|---:|
| baseline_query_count | 7 |
| blocker_count | 0 |
| hit_rate | 0.8571 |
| latency_summary | {"count": 7, "max_latency_ms": 30.894040999996264, "mean_latency_ms": 11.44029771428531, "p95_latency_ms": 30.894040999996264} |
| max_latency_ms | 30.894 |
| mean_expected_rank | 1 |
| mean_latency_ms | 11.4403 |
| mrr | 0.8571 |
| ndcg_at_10 | 0.8272 |
| ndcg_at_5 | 0.8019 |
| observed_summary_partial_reason_codes | [] |
| p95_latency_ms | 30.894 |
| positive_hit_rate | 1 |
| positive_mean_mrr | 1 |
| positive_mean_ndcg_at_5 | 0.9355 |
| positive_mean_precision_at_10 | 0.1833 |
| positive_mean_precision_at_5 | 0.3333 |
| positive_mean_recall_at_5 | 0.9167 |
| positive_query_count | 6 |
| precision_at_10 | 0.1571 |
| precision_at_5 | 0.2857 |
| query_count | 7 |
| readiness_fallback | {"classification": "degraded", "elapsed_s": 7.013, "explanation": "Readiness used fallback settling because no direct ready signal was available; results can still be valid but should be reviewed with elapsed time.", "fallback_sleep_s": 3.0, "fallback_used": true, "impact": "degraded", "poll_attempts": 3, "readiness_signal": "none", "reason": "no_direct_readiness_signal", "status": "fallback_after_no_signal"} |
| reason_code_classification | {} |
| recall_at_10 | 0.8571 |
| recall_at_5 | 0.7857 |
| runtime_minutes | 1.0487 |
| seed_completed | True |
| threshold_evaluation | {"enforcement": "local-only", "evaluated_metrics": 8, "failure_counts": {"blocker": 0, "warn": 0}, "failures": [], "fixture_tier": "small", "policy_name": "local-v2-threshold-policy", "reason": "all required threshold checks passed", "status": "pass"} |

## Readiness fallback

- Status: `fallback_after_no_signal`
- Impact: `degraded`
- Elapsed (s): `7.013`
- Fallback used: `True`
- Explanation: Readiness used fallback settling because no direct ready signal was available; results can still be valid but should be reviewed with elapsed time.

## Per-query metrics

| Query | Rank | MRR | R@5 | R@10 | NDCG@5 | NDCG@10 | P@5 | P@10 | Latency ms | Failure | Top-1 |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|---|
| mini_q_recall_checkout_retry_backoff | 1 | 1 | 0.5 | 1 | 0.6131 | 0.7904 | 0.2 | 0.2 | 30.894 | none | e6a75891e45d42453c3d (mini_mem_decision_checkout_backoff) |
| mini_q_search_bm25_refund_ops | 1 | 1 | 1 | 1 | 1 | 1 | 0.4 | 0.2 | 1.2955 | none | 73c85e08368b1e84f4f5 (mini_mem_research_refund_notifications) |
| mini_q_search_vector_helios_stability | 1 | 1 | 1 | 1 | 1 | 1 | 0.4 | 0.2 | 15.4734 | none | 9dc56b979149c7ce989d (mini_mem_task_helios_crash_review) |
| mini_q_get_valid_checkout_namespace | 1 | 1 | 1 | 1 | 1 | 1 | 0.4 | 0.2 | 0.914 | none | e6a75891e45d42453c3d (mini_mem_decision_checkout_backoff) |
| mini_q_get_valid_temporal_post_migration | 1 | 1 | 1 | 1 | 1 | 1 | 0.2 | 0.1 | 0.558 | none | 2a99c011e7899db89319 (mini_mem_decision_parser_fallback_removed) |
| mini_q_recall_person_project_topic | 1 | 1 | 1 | 1 | 1 | 1 | 0.4 | 0.2 | 14.3902 | none | 369bf56ae5cba0baabe1 (mini_mem_user_pref_release_notes) |
| mini_q_negative_no_match_nonsense | — | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 16.557 | wrong_rank | 9cb37dea32d27bd1a22a (mini_mem_context_process_namespace) |
