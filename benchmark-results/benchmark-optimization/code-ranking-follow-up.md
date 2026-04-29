# Code Ranking Follow-up Diagnosis — Task 6

## Summary

Task 6 clarified the two reported weak positive queries without changing ranking code:

- `nl_recall_code_hybrid` now treats the hybrid search orchestration file and graph fusion files as valid targets because the query asks for vector/BM25/symbol graph/PageRank fusion, not only the `recall_code` wrapper.
- `code_recall_code_fn` now treats both `src/server/logic/code/search.rs` and `src/server/handler.rs` as valid targets because both fixture files define an async `recall_code` function matching the exact code-ish query.

After rerunning `python3 evals/code_retrieval_benchmark.py`, both targeted queries rank at 1 and MRR is `0.8667`, satisfying the MRR target. The aggregate hit rate remains `0.8667`, below `0.90`, because the benchmark includes two `negative_no_match` rows with empty `expected_paths`; the shared metrics helper counts those rows as non-hits.

## Why this is not a ranking-algorithm change

The remaining aggregate miss is not caused by the two weak positive queries. It is caused by benchmark semantics for negative no-match rows:

- `negative_kubernetes_billing` has no expected path and still returns top-k project results.
- `negative_react_checkout` has no expected path and still returns top-k project results.
- With empty expected paths, `compute_query_metrics` emits `expected_rank=None` and `hit_count=0`; aggregate `hit_rate` therefore tops out at `13/15 = 0.8667` even when every positive query is found.

Changing RRF weights, PPR, BM25, vector search, or server search code would be the wrong fix for Task 6 because the task explicitly prohibits ranking/source changes and the positive weak-query issue is resolved by benchmark/golden clarification.

## Actionable follow-up options

1. Decide benchmark semantics for `negative_no_match` rows:
   - If negative rows are retrieval-quality probes, keep them out of positive `hit_rate` and report a separate `negative_empty_rate` / `negative_suppression_rate`.
   - If negative rows must return zero results, create a separate server/product task to design no-match thresholding or abstention behavior.
2. If no-match suppression is desired, implement it as a future ranking/retrieval design task with explicit acceptance criteria; do not hide it inside benchmark optimization.
3. Preserve the hard negative queries. They are useful diagnostics, but they should not make the positive-query hit-rate gate mathematically impossible.

## Evidence

- Weak-query evidence: `.sisyphus/evidence/benchmark-optimization/task-6-code-weak-query-results.txt`
- No ranking source diff: `.sisyphus/evidence/benchmark-optimization/task-6-no-ranking-src-diff.txt`
- Fresh benchmark report: `.sisyphus/evidence/evals/code-retrieval-baseline.json`
