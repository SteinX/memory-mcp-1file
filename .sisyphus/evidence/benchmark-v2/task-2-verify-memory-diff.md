# Memory Retrieval Baseline Diff

## Baseline pair

- Before: `/Users/xiayiming1/Documents/Workspace/memory-mcp-1file/.sisyphus/evidence/evals/memory-retrieval-baseline-pre-remap-20260427172553.json`
- After: `/Users/xiayiming1/Documents/Workspace/memory-mcp-1file/.sisyphus/evidence/evals/memory-retrieval-baseline.json`
- Before available: `True`
- After available: `True`

## Metric deltas

| Metric | Before | After | Delta |
|---|---:|---:|---:|
| hit_rate | 0 | 0.8 | 0.8 |
| mrr | 0 | 0.75 | 0.75 |
| precision_at_5 | 0 | 0.3 | 0.3 |
| precision_at_10 | 0 | 0.19 | 0.19 |
| recall_at_5 | — | — | — |
| recall_at_10 | — | — | — |
| ndcg_at_5 | — | — | — |
| ndcg_at_10 | — | — | — |
| mean_latency_ms | 11.0516 | 9.4511 | -1.6006 |
| max_latency_ms | 42.5356 | 27.2623 | -15.2734 |
| p95_latency_ms | 42.5356 | 27.2623 | -15.2734 |
| blocker_count | 0 | 0 | 0 |

## Reason codes

- before: `[]`
- after: `[]`
- delta: `{"added": [], "count_delta": 0, "removed": []}`

## Blockers

- before: `[]`
- after: `[]`
- delta: `{"added": [], "count_delta": 0, "removed": []}`

## Baseline change summary

- changed: `["hit_rate", "max_latency_ms", "mean_latency_ms", "mrr", "p95_latency_ms", "precision_at_10", "precision_at_5"]`
- missing_before: `[]`
- missing_after: `[]`
- improved: `["hit_rate", "max_latency_ms", "mean_latency_ms", "mrr", "p95_latency_ms", "precision_at_10", "precision_at_5"]`
- regressed: `[]`
- unchanged: `["blocker_count"]`

## Policy

- Baseline diff is reporting/evidence only.
- No CI gate or automated regression blocker is enforced by this diff output.
