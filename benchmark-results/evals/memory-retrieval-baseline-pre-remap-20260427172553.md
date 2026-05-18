# Memory Retrieval Baseline

## Run context

- Command used: `/Users/xiayiming1/Documents/Workspace/memory-mcp-1file/target/fast/memory-mcp --stdio`
- Embedding model: `e5_small`
- Data dir: `/var/folders/mx/tffz_j1d5lg_twyfntz97yr00000gp/T/task-8-memory-retrieval-data-4q3aahta`
- Started at: `2026-04-27T04:46:53.778672+00:00`
- Duration (s): `119.08`

### stderr tail
- [memory-mcp] Auto-configured block cache: 94 MB (available RAM: 474 MB)
- [2m2026-04-27T04:46:53.787603Z[0m [32m INFO[0m [2mmemory_mcp[0m[2m:[0m memory-mcp starting [3mversion[0m[2m=[0m"0.8.2" [3mpid[0m[2m=[0m26136 [3mppid[0m[2m=[0m26135 [3mmode[0m[2m=[0m"stdio" [3mmodel[0m[2m=[0me5_small [3mdata_dir[0m[2m=[0m/var/folders/mx/tffz_j1d5lg_twyfntz97yr00000gp/T/task-8-memory-retrieval-data-4q3aahta
- [2m2026-04-27T04:46:53.787863Z[0m [32m INFO[0m [2msurrealdb::core::kvs::ds[0m[2m:[0m Starting kvs store at absolute path surrealkv:/var/folders/mx/tffz_j1d5lg_twyfntz97yr00000gp/T/task-8-memory-retrieval-data-4q3aahta/db
- [2m2026-04-27T04:46:53.788020Z[0m [32m INFO[0m [2msurrealdb::core::kvs::surrealkv[0m[2m:[0m Enabling value log separation: true
- [2m2026-04-27T04:46:53.799630Z[0m [32m INFO[0m [2msurrealdb::core::kvs::surrealkv[0m[2m:[0m Setting value log max file size: 268435456
- [2m2026-04-27T04:46:53.799643Z[0m [32m INFO[0m [2msurrealdb::core::kvs::surrealkv[0m[2m:[0m Setting value log threshold: 4096
- [2m2026-04-27T04:46:53.799644Z[0m [32m INFO[0m [2msurrealdb::core::kvs::surrealkv[0m[2m:[0m Versioning enabled: false with retention period: 0ns
- [2m2026-04-27T04:46:53.799645Z[0m [32m INFO[0m [2msurrealdb::core::kvs::surrealkv[0m[2m:[0m Versioning with versioned_index: false
- [2m2026-04-27T04:46:53.799649Z[0m [32m INFO[0m [2msurrealdb::core::kvs::surrealkv[0m[2m:[0m Setting block cache capacity: 98566144
- [2m2026-04-27T04:46:53.799657Z[0m [32m INFO[0m [2msurrealdb::core::kvs::surrealkv[0m[2m:[0m Setting block size: 65536
- [2m2026-04-27T04:46:53.800262Z[0m [32m INFO[0m [2msurrealkv::lsm[0m[2m:[0m === Starting LSM tree initialization ===
- [2m2026-04-27T04:46:53.800270Z[0m [32m INFO[0m [2msurrealkv::lsm[0m[2m:[0m Database path: "/var/folders/mx/tffz_j1d5lg_twyfntz97yr00000gp/T/task-8-memory-retrieval-data-4q3aahta/db"
- [2m2026-04-27T04:46:53.817436Z[0m [32m INFO[0m [2msurrealkv::lsm[0m[2m:[0m Manifest state: log_number=0, last_sequence=0
- [2m2026-04-27T04:46:53.817446Z[0m [32m INFO[0m [2msurrealkv::wal::recovery[0m[2m:[0m Starting WAL recovery from directory: "/var/folders/mx/tffz_j1d5lg_twyfntz97yr00000gp/T/task-8-memory-retrieval-data-4q3aahta/db/wal"
- [2m2026-04-27T04:46:53.817512Z[0m [32m INFO[0m [2msurrealkv::wal::recovery[0m[2m:[0m Replaying WAL segments #00000000000000000000 to #00000000000000000000
- [2m2026-04-27T04:46:53.824117Z[0m [32m INFO[0m [2msurrealkv::wal::recovery[0m[2m:[0m WAL recovery complete: 0 batches across 0 segments, 0 memtables created, max_seq_num=None
- [2m2026-04-27T04:46:53.824295Z[0m [32m INFO[0m [2msurrealkv::lsm[0m[2m:[0m === LSM tree initialization complete ===
- [2m2026-04-27T04:46:53.832301Z[0m [32m INFO[0m [2msurrealdb::core::kvs::surrealkv[0m[2m:[0m Sync mode: every transaction commit
- [2m2026-04-27T04:46:53.832316Z[0m [32m INFO[0m [2msurrealdb::core::kvs::surrealkv[0m[2m:[0m Grouped commit: enabled (timeout=5000000ns, wait_threshold=12, max_batch_size=4096)
- [2m2026-04-27T04:46:53.832336Z[0m [32m INFO[0m [2msurrealdb::core::kvs::ds[0m[2m:[0m Started surrealkv kvs store
- [2m2026-04-27T04:46:54.478012Z[0m [32m INFO[0m [2mmemory_mcp::storage::surrealdb[0m[2m:[0m Dimension check passed [3mmodel[0m[2m=[0m384 [3mdb[0m[2m=[0m384
- [2m2026-04-27T04:46:54.478029Z[0m [32m INFO[0m [2mmemory_mcp[0m[2m:[0m Embedding engine configured [3moutput_dim[0m[2m=[0m384 [3mmodel[0m[2m=[0me5_small
- [2m2026-04-27T04:46:54.506414Z[0m [32m INFO[0m [2mmemory_mcp::embedding::service[0m[2m:[0m Loading embedding model: E5Small
- [2m2026-04-27T04:46:54.506420Z[0m [32m INFO[0m [2mmemory_mcp::embedding::worker[0m[2m:[0m Embedding worker started, waiting for requests
- [2m2026-04-27T04:46:54.506448Z[0m [32m INFO[0m [2mmemory_mcp::forgetting::capacity[0m[2m:[0m Capacity controller started [3mcheck_interval_secs[0m[2m=[0m3600 [3msoft_limit[0m[2m=[0m10000 [3mcleanup_target_ratio[0m[2m=[0m0.800000011920929
- [2m2026-04-27T04:46:54.507442Z[0m [32m INFO[0m [2mrmcp::handler::server[0m[2m:[0m client initialized
- [2m2026-04-27T04:46:54.507460Z[0m [32m INFO[0m [1mserve_inner[0m[2m:[0m [2mrmcp::service[0m[2m:[0m Service initialized as server [3mpeer_info[0m[2m=[0mSome(InitializeRequestParams { meta: None, protocol_version: ProtocolVersion("2024-11-05"), capabilities: ClientCapabilities { experimental: None, extensions: None, roots: None, sampling: None, elicitation: None, tasks: None }, client_info: Implementation { name: "memory-retrieval-benchmark", title: None, version: "0.1.0", description: None, icons: None, website_url: None } })
- [2m2026-04-27T04:46:54.507482Z[0m [32m INFO[0m [2mmemory_mcp[0m[2m:[0m Server started in stdio mode, waiting for client disconnect or signals...
- [2m2026-04-27T04:46:54.508441Z[0m [32m INFO[0m [2mmemory_mcp::forgetting::capacity[0m[2m:[0m Capacity OK: 0 memories [3mcount[0m[2m=[0m0
- [2m2026-04-27T04:47:01.263446Z[0m [32m INFO[0m [2mmemory_mcp::embedding::service[0m[2m:[0m Downloading model weights from HuggingFace Hub...
- [2m2026-04-27T04:48:44.437746Z[0m [32m INFO[0m [2mmemory_mcp::embedding::service[0m[2m:[0m Embedding model ready [3melapsed_sec[0m[2m=[0m"109.9"

## Aggregate metrics

| Metric | Value |
|---|---:|
| baseline_query_count | 10 |
| blocker_count | 0 |
| hit_rate | 0 |
| latency_summary | {"count": 10, "max_latency_ms": 42.5356249999993, "mean_latency_ms": 11.051641600002426, "p95_latency_ms": 42.5356249999993} |
| max_latency_ms | 42.5356 |
| mean_expected_rank | — |
| mean_latency_ms | 11.0516 |
| mrr | 0 |
| observed_summary_partial_reason_codes | [] |
| p95_latency_ms | 42.5356 |
| positive_mean_mrr | 0 |
| positive_mean_precision_at_10 | 0 |
| positive_mean_precision_at_5 | 0 |
| positive_query_count | 8 |
| precision_at_10 | 0 |
| precision_at_5 | 0 |
| query_count | 10 |
| seed_completed | True |

## Per-query metrics

| Query | Rank | MRR | P@5 | P@10 | Latency ms |
|---|---:|---:|---:|---:|---:|
| q_recall_fusion_auth_timeout | — | 0 | 0 | 0 | 42.5356 |
| q_recall_fusion_benchmark_scope | — | 0 | 0 | 0 | 17.9104 |
| q_recall_fusion_terse_notes | — | 0 | 0 | 0 | 16.3918 |
| q_search_bm25_cache_prefix | — | 0 | 0 | 0 | 1.2748 |
| q_search_bm25_temporal_windows | — | 0 | 0 | 0 | 1.3769 |
| q_search_vector_billing_retry | — | 0 | 0 | 0 | 13.557 |
| q_get_valid_temporal_checkpoint | — | 0 | 0 | 0 | 1.5944 |
| q_get_valid_filtered_auth_namespace | — | 0 | 0 | 0 | 1.2352 |
| q_negative_no_match_nonsense | — | 0 | 0 | 0 | 13.5892 |
| q_negative_no_match_missing_prefix | — | 0 | 0 | 0 | 1.051 |
