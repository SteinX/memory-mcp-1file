# Task 2 recall_code baseline

Baseline-only benchmark for existing `recall_code`; retrieval code, RRF/PPR weights, indexing logic, and response shapes were not changed.

## Environment

- Root: `/Users/xiayiming1/Documents/Workspace/memory-mcp-1file`
- Benchmark scope: `fixture_baseline_from_required_real_repo_files`
- Project ID: `task-2-recall-code-fixture`
- Model: `e5_small`
- Server command: `['/Users/xiayiming1/Documents/Workspace/memory-mcp-1file/target/debug/memory-mcp', '--stdio']`
- Data dir strategy: temporary isolated DATA_DIR (/var/folders/mx/tffz_j1d5lg_twyfntz97yr00000gp/T/task-2-recall-code-data-a0nsyk2c) for baseline reproducibility
- Duration: 243.9 seconds
- Fixture source paths: `src/graph/rrf.rs`, `src/graph/ppr.rs`, `src/server/logic/code/search.rs`, `src/codebase/indexer.rs`, `src/server/handler.rs`

## Scope and blockers

- Executable metrics were collected against a bounded fixture project copied from the five required real repository files, not by indexing the full repository. This avoids repeating the prior full-repo hang while preserving the curated target files/functions.
- Blocker count: 1
- Blockers: index_readiness: index readiness timeout
- Readiness note: queries were still executed when structural indexing was usable but semantic lifecycle remained pending; degraded vector availability is recorded in raw responses.

## Aggregate metrics

- Queries: 15 total (8 natural-language, 5 code-ish/symbol, 2 negative)
- Positive mean precision@5: 0.4154
- Positive mean precision@10: 0.2846
- Positive MRR: 0.6393
- Negative empty-result rate: 0.0000
- Mean latency: 1866.95 ms
- Max latency: 3961.10 ms
- Mean result count: 10.00
- Diagnostic contract present for all queries: True
- Diagnostic summary present for all queries: True
- Score breakdown present whenever results exist: True

## Per-query results

| Query ID | Type | P@5 | P@10 | MRR | Latency ms | Count | Top path | Notes |
|---|---:|---:|---:|---:|---:|---:|---|---|
| nl_rrf_merge | natural_language | 1.0000 | 0.6000 | 1.0000 | 3055.01 | 10 | `/var/folders/mx/tffz_j1d5lg_twyfntz97yr00000gp/T/task-2-recall-code-project-czjqr9m4/task-2-recall-code-fixture/src/graph/rrf.rs` | rrf.rs top5=True |
| nl_ppr_kernel | natural_language | 0.8000 | 0.5000 | 1.0000 | 638.50 | 10 | `/var/folders/mx/tffz_j1d5lg_twyfntz97yr00000gp/T/task-2-recall-code-project-czjqr9m4/task-2-recall-code-fixture/src/graph/ppr.rs` |  |
| nl_code_search_contract | natural_language | 0.2000 | 0.1000 | 1.0000 | 560.51 | 10 | `/var/folders/mx/tffz_j1d5lg_twyfntz97yr00000gp/T/task-2-recall-code-project-czjqr9m4/task-2-recall-code-fixture/src/server/logic/code/search.rs` |  |
| nl_recall_code_hybrid | natural_language | 0.0000 | 0.1000 | 0.1111 | 3714.35 | 10 | `/var/folders/mx/tffz_j1d5lg_twyfntz97yr00000gp/T/task-2-recall-code-project-czjqr9m4/task-2-recall-code-fixture/src/server/handler.rs` |  |
| nl_index_spawn_blocking | natural_language | 0.2000 | 0.2000 | 1.0000 | 789.74 | 10 | `/var/folders/mx/tffz_j1d5lg_twyfntz97yr00000gp/T/task-2-recall-code-project-czjqr9m4/task-2-recall-code-fixture/src/codebase/indexer.rs` |  |
| nl_handler_mode_dispatch | natural_language | 0.4000 | 0.2000 | 0.5000 | 1236.00 | 10 | `/var/folders/mx/tffz_j1d5lg_twyfntz97yr00000gp/T/task-2-recall-code-project-czjqr9m4/task-2-recall-code-fixture/src/server/handler.rs` |  |
| nl_symbol_exact_channel | natural_language | 0.0000 | 0.0000 | 0.0000 | 3048.15 | 10 | `/var/folders/mx/tffz_j1d5lg_twyfntz97yr00000gp/T/task-2-recall-code-project-czjqr9m4/task-2-recall-code-fixture/src/server/handler.rs` |  |
| nl_hub_dampening | natural_language | 0.6000 | 0.4000 | 1.0000 | 657.10 | 10 | `/var/folders/mx/tffz_j1d5lg_twyfntz97yr00000gp/T/task-2-recall-code-project-czjqr9m4/task-2-recall-code-fixture/src/graph/ppr.rs` |  |
| code_rrf_merge_symbol | codeish_symbol | 0.8000 | 0.6000 | 1.0000 | 3961.10 | 10 | `/var/folders/mx/tffz_j1d5lg_twyfntz97yr00000gp/T/task-2-recall-code-project-czjqr9m4/task-2-recall-code-fixture/src/graph/rrf.rs` |  |
| code_run_ppr | codeish_symbol | 0.2000 | 0.1000 | 0.5000 | 916.06 | 10 | `/var/folders/mx/tffz_j1d5lg_twyfntz97yr00000gp/T/task-2-recall-code-project-czjqr9m4/task-2-recall-code-fixture/src/graph/ppr.rs` |  |
| code_personalized_page_rank | codeish_symbol | 1.0000 | 0.6000 | 1.0000 | 781.32 | 10 | `/var/folders/mx/tffz_j1d5lg_twyfntz97yr00000gp/T/task-2-recall-code-project-czjqr9m4/task-2-recall-code-fixture/src/graph/ppr.rs` |  |
| code_recall_code_fn | codeish_symbol | 0.0000 | 0.0000 | 0.0000 | 3423.71 | 10 | `/var/folders/mx/tffz_j1d5lg_twyfntz97yr00000gp/T/task-2-recall-code-project-czjqr9m4/task-2-recall-code-fixture/src/graph/ppr.rs` |  |
| code_index_project_fn | codeish_symbol | 0.2000 | 0.3000 | 0.2000 | 945.06 | 10 | `/var/folders/mx/tffz_j1d5lg_twyfntz97yr00000gp/T/task-2-recall-code-project-czjqr9m4/task-2-recall-code-fixture/src/graph/ppr.rs` |  |
| negative_kubernetes_billing | negative_no_match | 0.0000 | 0.0000 | 0.0000 | 1010.32 | 10 | `/var/folders/mx/tffz_j1d5lg_twyfntz97yr00000gp/T/task-2-recall-code-project-czjqr9m4/task-2-recall-code-fixture/src/graph/ppr.rs` | returned_potentially_misleading_results |
| negative_react_checkout | negative_no_match | 0.0000 | 0.0000 | 0.0000 | 3267.27 | 10 | `/var/folders/mx/tffz_j1d5lg_twyfntz97yr00000gp/T/task-2-recall-code-project-czjqr9m4/task-2-recall-code-fixture/src/graph/ppr.rs` |  |

## Representative raw responses

- RRF query: `.sisyphus/evidence/task-2-rrf-query.json`
- Negative query: `.sisyphus/evidence/task-2-negative-query.json`

## Interpretation notes

- Precision and MRR are computed against known expected file/function targets for this repo.
- Negative queries are intentionally no-match; non-empty results are recorded as misleading baseline behavior, not hidden.
- `mode=hybrid` was used for all benchmark queries to exercise vector + BM25 + symbol/PPR fusion where available.
