# Code Retrieval Baseline Diff

## Baseline pair

- Before: `/Users/xiayiming1/Documents/Workspace/memory-mcp-1file/.sisyphus/evidence/task-2-recall-code-baseline.json`
- After: `/Users/xiayiming1/Documents/Workspace/memory-mcp-1file/.sisyphus/evidence/evals/code-retrieval-baseline.json`
- Before available: `True`
- After available: `True`

## Metric deltas

| Metric | Before | After | Delta |
|---|---:|---:|---:|
| hit_rate | — | 0.8667 | — |
| mrr | — | 0.8333 | — |
| precision_at_5 | — | 0.7733 | — |
| precision_at_10 | — | 0.64 | — |
| recall_at_5 | — | — | — |
| recall_at_10 | — | — | — |
| ndcg_at_5 | — | — | — |
| ndcg_at_10 | — | — | — |
| mean_latency_ms | 1866.9467 | 31.868 | -1835.0787 |
| max_latency_ms | 3961.1 | 39.88 | -3921.22 |
| p95_latency_ms | — | 39.88 | — |
| blocker_count | 1 | 0 | -1 |

## Reason codes

- before: `[]`
- after: `["partial"]`
- delta: `{"added": ["partial"], "count_delta": 1, "removed": []}`

## Blockers

- before: `["\u2014: index readiness timeout"]`
- after: `[]`
- delta: `{"added": [], "count_delta": -1, "removed": ["\u2014: index readiness timeout"]}`

## Baseline change summary

- changed: `["blocker_count", "max_latency_ms", "mean_latency_ms"]`
- missing_before: `["hit_rate", "mrr", "p95_latency_ms", "precision_at_10", "precision_at_5"]`
- missing_after: `[]`
- improved: `["blocker_count", "max_latency_ms", "mean_latency_ms"]`
- regressed: `[]`
- unchanged: `[]`

## Policy

- Baseline diff is reporting/evidence only.
- No CI gate or automated regression blocker is enforced by this diff output.
