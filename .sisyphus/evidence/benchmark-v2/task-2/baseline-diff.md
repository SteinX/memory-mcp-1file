# Memory Retrieval Baseline Diff

## Baseline pair

- Before: `.sisyphus/evidence/evals/memory-retrieval-baseline.json`
- After: `.sisyphus/evidence/benchmark-v2/task-2/refresh.json`
- Before available: `True`
- After available: `True`

## Metric deltas

| Metric | Before | After | Delta |
|---|---:|---:|---:|
| hit_rate | 0.8 | 0.8 | 0 |
| mrr | 0.75 | 0.7333 | -0.0167 |
| precision_at_5 | 0.3 | 0.28 | -0.02 |
| precision_at_10 | 0.19 | 0.18 | -0.01 |
| recall_at_5 | — | — | — |
| recall_at_10 | — | — | — |
| ndcg_at_5 | — | — | — |
| ndcg_at_10 | — | — | — |
| mean_latency_ms | 9.4511 | 10.3328 | 0.8817 |
| max_latency_ms | 27.2623 | 32.0618 | 4.7995 |
| p95_latency_ms | 27.2623 | 32.0618 | 4.7995 |
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

- changed: `["max_latency_ms", "mean_latency_ms", "mrr", "p95_latency_ms", "precision_at_10", "precision_at_5"]`
- missing_before: `[]`
- missing_after: `[]`
- improved: `[]`
- regressed: `["max_latency_ms", "mean_latency_ms", "mrr", "p95_latency_ms", "precision_at_10", "precision_at_5"]`
- unchanged: `["blocker_count", "hit_rate"]`

## Policy

- Baseline diff is reporting/evidence only.
- No CI gate or automated regression blocker is enforced by this diff output.
