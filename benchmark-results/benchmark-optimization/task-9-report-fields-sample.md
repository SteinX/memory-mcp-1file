# Task 9 Sample

## Run context

## Aggregate metrics

| Metric | Value |
|---|---:|
| hit_rate | 0.5 |
| latency_summary | {"count": 2, "max_latency_ms": 2.0, "mean_latency_ms": 1.5, "p95_latency_ms": 2.0} |
| max_latency_ms | 2 |
| mean_expected_rank | 2 |
| mean_latency_ms | 1.5 |
| mrr | 0.25 |
| ndcg_at_10 | 0.3155 |
| ndcg_at_5 | 0.3155 |
| p95_latency_ms | 2 |
| precision_at_10 | 0.05 |
| precision_at_5 | 0.1 |
| query_count | 2 |
| recall_at_10 | 0.5 |
| recall_at_5 | 0.5 |

## Per-query metrics

| Query | Rank | MRR | R@5 | R@10 | NDCG@5 | NDCG@10 | P@5 | P@10 | Latency ms | Failure | Top-1 |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|---|
| q1 | 2 | 0.5 | 1 | 1 | 0.6309 | 0.6309 | 0.2 | 0.1 | 1 | — | — |
| q2 | — | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 2 | — | — |
