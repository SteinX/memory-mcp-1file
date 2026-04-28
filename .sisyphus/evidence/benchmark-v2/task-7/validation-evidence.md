# T7 label QA and fixture-drift checks evidence

Date: 2026-04-28

## Scope

- Modified benchmark validation only:
  - `evals/memory_retrieval_benchmark.py`
  - `evals/code_retrieval_benchmark.py`
- No fixture/golden files were rewritten by validation.

## Validation added

- Memory validation rejects duplicate fixture IDs with duplicate value/count diagnostics.
- Memory validation detects explicit ID-like alias collisions in fixture metadata alias fields.
- Memory validation rejects missing or drifted `label_rationale` fields for medium benchmark queries, including expected/no-match consistency.
- Code validation rejects duplicate query IDs, missing rationale, missing expected paths, symbols absent from expected files, and non-empty expected labels on negative/deleted/renamed sentinel queries.

## Self-test evidence

```text
$ python3 evals/memory_retrieval_benchmark.py --self-test
self-test passed (fixtures=15 memories, queries=10, negative_queries=2, mini_fixtures=10 memories, mini_queries=7, medium_fixtures=13 memories, medium_queries=7, stress_manifests=validated)
label_qa_rejections included:
- duplicate_memory_ids: Medium long-memory fixture IDs must be unique; duplicate values: medium_mem_task_aurora_retention_snapshots (2x)
- alias_collision: Medium long-memory fixture alias collisions detected
- missing_label_rationale: Memory query missing label_rationale object: medium_q_long_memory_aurora_retention_context
- expected_id_drift: Memory query medium_q_long_memory_aurora_retention_context label_rationale.expected_ids drift
- negative_consistency: Memory query medium_q_negative_no_match_synthetic_nonsense negative_no_match must have empty expected_ids and no_match_expected=true
```

```text
$ python3 evals/code_retrieval_benchmark.py --self-test
self-test: loaded baseline query count=15 from evals/golden/code_retrieval_queries_v2.json (canonical=evals/golden/code_retrieval_queries_v2.json)
self-test: validated medium-tier query count=14 from evals/golden/code_retrieval_queries_v2.json
self-test passed
```

## Diagnostics

```text
lsp_diagnostics evals/memory_retrieval_benchmark.py: No diagnostics found
lsp_diagnostics evals/code_retrieval_benchmark.py: No diagnostics found
```
