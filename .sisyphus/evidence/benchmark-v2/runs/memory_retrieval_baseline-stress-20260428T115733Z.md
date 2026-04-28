# Memory Retrieval Baseline

## Benchmark V2 summary

| Field | Value |
|---|---|
| schema_version | `2.0` |
| fixture_tier | `stress` |
| baseline_version | `v2-initial` |
| threshold_policy | `local-v2-threshold-policy` |
| threshold_status | `deferred` |
| threshold_status_reason | `threshold evaluation deferred because no queries were executed` |

### Readiness summary

- reason_codes: `[]`
- reason_code_classification: `{}`
- readiness_fallback: `{"elapsed_s": null, "explanation": "No settle_readiness result was recorded for this run.", "impact": "informational", "status": "unavailable"}`

### Failure buckets

| Failure type | Count |
|---|---:|
| none | 0 |

### Baseline diff summary

- status: `deferred`
- reason: `baseline diff summary is produced by explicit baseline-diff workflow`

### Metric summary

| Metric | Value |
|---|---:|
| query_count | 0 |
| hit_rate | 0 |
| mrr | 0 |
| precision_at_5 | 0 |
| precision_at_10 | 0 |
| recall_at_5 | 0 |
| recall_at_10 | 0 |
| ndcg_at_5 | 0 |
| ndcg_at_10 | 0 |
| mean_latency_ms | 0 |
| max_latency_ms | 0 |
| p95_latency_ms | 0 |
| blocker_count | 0 |
| positive_query_count | 0 |
| positive_hit_rate | — |
| positive_mean_mrr | — |
| positive_mean_recall_at_5 | — |
| positive_mean_ndcg_at_5 | — |
| positive_mean_precision_at_5 | — |
| runtime_minutes | 0.2529 |

### Deterministic / local-only metadata

- threshold_policy_enforcement: `local-only`
- determinism_policy: `{"name": "stable_fixture_order+stable_tie_break+stable_report_order+tolerance_1e-9_1e-6"}`
- runtime_target: `{"optional_policy": "manual-only stress tier", "required_by_default": false, "target_minutes": "45-90+"}`

## Run context

- Command used: `/Users/xiayiming1/Documents/Workspace/memory-mcp-1file/target/fast/memory-mcp --stdio`
- Embedding model: `e5_small`
- Data dir: `/var/folders/mx/tffz_j1d5lg_twyfntz97yr00000gp/T/task-8-memory-retrieval-data-ld59y6ff`
- Started at: `2026-04-28T11:57:17.960319+00:00`
- Duration (s): `15.2`

### stderr tail
- [memory-mcp] Auto-configured block cache: 281 MB (available RAM: 1406 MB)
- [2m2026-04-28T11:57:17.986552Z[0m [32m INFO[0m [2mmemory_mcp[0m[2m:[0m memory-mcp starting [3mversion[0m[2m=[0m"0.8.2" [3mpid[0m[2m=[0m42239 [3mppid[0m[2m=[0m42236 [3mmode[0m[2m=[0m"stdio" [3mmodel[0m[2m=[0me5_small [3mdata_dir[0m[2m=[0m/var/folders/mx/tffz_j1d5lg_twyfntz97yr00000gp/T/task-8-memory-retrieval-data-ld59y6ff
- [2m2026-04-28T11:57:17.993913Z[0m [32m INFO[0m [2msurrealdb::core::kvs::ds[0m[2m:[0m Starting kvs store at absolute path surrealkv:/var/folders/mx/tffz_j1d5lg_twyfntz97yr00000gp/T/task-8-memory-retrieval-data-ld59y6ff/db
- [2m2026-04-28T11:57:17.997400Z[0m [32m INFO[0m [2msurrealdb::core::kvs::surrealkv[0m[2m:[0m Enabling value log separation: true
- [2m2026-04-28T11:57:18.008341Z[0m [32m INFO[0m [2msurrealdb::core::kvs::surrealkv[0m[2m:[0m Setting value log max file size: 268435456
- [2m2026-04-28T11:57:18.008352Z[0m [32m INFO[0m [2msurrealdb::core::kvs::surrealkv[0m[2m:[0m Setting value log threshold: 4096
- [2m2026-04-28T11:57:18.008354Z[0m [32m INFO[0m [2msurrealdb::core::kvs::surrealkv[0m[2m:[0m Versioning enabled: false with retention period: 0ns
- [2m2026-04-28T11:57:18.008354Z[0m [32m INFO[0m [2msurrealdb::core::kvs::surrealkv[0m[2m:[0m Versioning with versioned_index: false
- [2m2026-04-28T11:57:18.008517Z[0m [32m INFO[0m [2msurrealdb::core::kvs::surrealkv[0m[2m:[0m Setting block cache capacity: 294649856
- [2m2026-04-28T11:57:18.008744Z[0m [32m INFO[0m [2msurrealdb::core::kvs::surrealkv[0m[2m:[0m Setting block size: 65536
- [2m2026-04-28T11:57:18.009271Z[0m [32m INFO[0m [2msurrealkv::lsm[0m[2m:[0m === Starting LSM tree initialization ===
- [2m2026-04-28T11:57:18.009497Z[0m [32m INFO[0m [2msurrealkv::lsm[0m[2m:[0m Database path: "/var/folders/mx/tffz_j1d5lg_twyfntz97yr00000gp/T/task-8-memory-retrieval-data-ld59y6ff/db"
- [2m2026-04-28T11:57:18.027533Z[0m [32m INFO[0m [2msurrealkv::lsm[0m[2m:[0m Manifest state: log_number=0, last_sequence=0
- [2m2026-04-28T11:57:18.027543Z[0m [32m INFO[0m [2msurrealkv::wal::recovery[0m[2m:[0m Starting WAL recovery from directory: "/var/folders/mx/tffz_j1d5lg_twyfntz97yr00000gp/T/task-8-memory-retrieval-data-ld59y6ff/db/wal"
- [2m2026-04-28T11:57:18.027637Z[0m [32m INFO[0m [2msurrealkv::wal::recovery[0m[2m:[0m Replaying WAL segments #00000000000000000000 to #00000000000000000000
- [2m2026-04-28T11:57:18.034254Z[0m [32m INFO[0m [2msurrealkv::wal::recovery[0m[2m:[0m WAL recovery complete: 0 batches across 0 segments, 0 memtables created, max_seq_num=None
- [2m2026-04-28T11:57:18.034549Z[0m [32m INFO[0m [2msurrealkv::lsm[0m[2m:[0m === LSM tree initialization complete ===
- [2m2026-04-28T11:57:18.052055Z[0m [32m INFO[0m [2msurrealdb::core::kvs::surrealkv[0m[2m:[0m Sync mode: every transaction commit
- [2m2026-04-28T11:57:18.052285Z[0m [32m INFO[0m [2msurrealdb::core::kvs::surrealkv[0m[2m:[0m Grouped commit: enabled (timeout=5000000ns, wait_threshold=12, max_batch_size=4096)
- [2m2026-04-28T11:57:18.052526Z[0m [32m INFO[0m [2msurrealdb::core::kvs::ds[0m[2m:[0m Started surrealkv kvs store
- [2m2026-04-28T11:57:18.840786Z[0m [32m INFO[0m [2mmemory_mcp::storage::surrealdb[0m[2m:[0m Dimension check passed [3mmodel[0m[2m=[0m384 [3mdb[0m[2m=[0m384
- [2m2026-04-28T11:57:18.840798Z[0m [32m INFO[0m [2mmemory_mcp[0m[2m:[0m Embedding engine configured [3moutput_dim[0m[2m=[0m384 [3mmodel[0m[2m=[0me5_small
- [2m2026-04-28T11:57:18.876599Z[0m [32m INFO[0m [2mmemory_mcp::embedding::service[0m[2m:[0m Loading embedding model: E5Small
- [2m2026-04-28T11:57:18.877532Z[0m [32m INFO[0m [2mmemory_mcp::forgetting::capacity[0m[2m:[0m Capacity controller started [3mcheck_interval_secs[0m[2m=[0m3600 [3msoft_limit[0m[2m=[0m10000 [3mcleanup_target_ratio[0m[2m=[0m0.800000011920929
- [2m2026-04-28T11:57:18.877871Z[0m [32m INFO[0m [2mmemory_mcp::embedding::worker[0m[2m:[0m Embedding worker started, waiting for requests
- [2m2026-04-28T11:57:18.881753Z[0m [32m INFO[0m [2mmemory_mcp::forgetting::capacity[0m[2m:[0m Capacity OK: 0 memories [3mcount[0m[2m=[0m0
- [2m2026-04-28T11:57:18.882376Z[0m [32m INFO[0m [2mrmcp::handler::server[0m[2m:[0m client initialized
- [2m2026-04-28T11:57:18.882388Z[0m [32m INFO[0m [1mserve_inner[0m[2m:[0m [2mrmcp::service[0m[2m:[0m Service initialized as server [3mpeer_info[0m[2m=[0mSome(InitializeRequestParams { meta: None, protocol_version: ProtocolVersion("2024-11-05"), capabilities: ClientCapabilities { experimental: None, extensions: None, roots: None, sampling: None, elicitation: None, tasks: None }, client_info: Implementation { name: "memory-retrieval-benchmark", title: None, version: "0.1.0", description: None, icons: None, website_url: None } })
- [2m2026-04-28T11:57:18.882409Z[0m [32m INFO[0m [2mmemory_mcp[0m[2m:[0m Server started in stdio mode, waiting for client disconnect or signals...
- [2m2026-04-28T11:57:22.546307Z[0m [32m INFO[0m [2mmemory_mcp::embedding::service[0m[2m:[0m Downloading model weights from HuggingFace Hub...
- [2m2026-04-28T11:57:29.764286Z[0m [32m INFO[0m [2mmemory_mcp::embedding::service[0m[2m:[0m Embedding model ready [3melapsed_sec[0m[2m=[0m"10.9"

## Aggregate metrics

| Metric | Value |
|---|---:|
| baseline_query_count | 0 |
| blocker_count | 0 |
| hit_rate | 0 |
| latency_summary | {"count": 0, "max_latency_ms": 0.0, "mean_latency_ms": 0.0, "p95_latency_ms": 0.0} |
| max_latency_ms | 0 |
| mean_expected_rank | — |
| mean_latency_ms | 0 |
| mrr | 0 |
| ndcg_at_10 | 0 |
| ndcg_at_5 | 0 |
| observed_summary_partial_reason_codes | [] |
| p95_latency_ms | 0 |
| positive_hit_rate | — |
| positive_mean_mrr | — |
| positive_mean_ndcg_at_5 | — |
| positive_mean_precision_at_10 | — |
| positive_mean_precision_at_5 | — |
| positive_mean_recall_at_5 | — |
| positive_query_count | 0 |
| precision_at_10 | 0 |
| precision_at_5 | 0 |
| query_count | 0 |
| readiness_fallback | {"elapsed_s": null, "explanation": "No settle_readiness result was recorded for this run.", "impact": "informational", "status": "unavailable"} |
| reason_code_classification | {} |
| recall_at_10 | 0 |
| recall_at_5 | 0 |
| runtime_minutes | 0.2529 |
| seed_completed | False |
| threshold_evaluation | {"enforcement": "local-only", "evaluated_metrics": 0, "failures": [], "fixture_tier": "stress", "policy_name": "local-v2-threshold-policy", "reason": "threshold evaluation deferred because no queries were executed", "status": "deferred"} |

## Warnings

- stress tier is manifest-only; runtime seeding/query execution deferred
- baseline refresh not requested; canonical baseline targets are protected and output was redirected

## Readiness fallback

- Status: `unavailable`
- Impact: `informational`
- Elapsed (s): `—`
- Fallback used: `—`
- Explanation: No settle_readiness result was recorded for this run.

## Per-query metrics

| Query | Rank | MRR | R@5 | R@10 | NDCG@5 | NDCG@10 | P@5 | P@10 | Latency ms | Failure | Top-1 |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|---|
