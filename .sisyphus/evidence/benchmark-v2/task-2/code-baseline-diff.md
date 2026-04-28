# Code Retrieval Baseline Diff

## Baseline pair

- Before: `.sisyphus/evidence/task-2-recall-code-baseline.json`
- After: `.sisyphus/evidence/task-2-recall-code-baseline.json`
- Before available: `True`
- After available: `True`

## Metric deltas

| Metric | Before | After | Delta |
|---|---:|---:|---:|
| hit_rate | — | — | — |
| mrr | — | — | — |
| precision_at_5 | — | — | — |
| precision_at_10 | — | — | — |
| recall_at_5 | — | — | — |
| recall_at_10 | — | — | — |
| ndcg_at_5 | — | — | — |
| ndcg_at_10 | — | — | — |
| mean_latency_ms | 1866.9467 | 1866.9467 | 0 |
| max_latency_ms | 3961.1 | 3961.1 | 0 |
| p95_latency_ms | — | — | — |
| blocker_count | 1 | 1 | 0 |

## Reason codes

- before: `[]`
- after: `[]`
- delta: `{"added": [], "count_delta": 0, "removed": []}`

## Blockers

- before: `["\u2014: index readiness timeout"]`
- after: `["\u2014: index readiness timeout"]`
- delta: `{"added": [], "count_delta": 0, "removed": []}`

## Baseline change summary

- changed: `[]`
- missing_before: `[]`
- missing_after: `[]`
- improved: `[]`
- regressed: `[]`
- unchanged: `["blocker_count", "max_latency_ms", "mean_latency_ms"]`

## Policy

- Baseline diff is reporting/evidence only.
- No CI gate or automated regression blocker is enforced by this diff output.
