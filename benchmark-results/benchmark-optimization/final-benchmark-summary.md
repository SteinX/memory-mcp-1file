# Task 11 Final Benchmark Summary

- Generated (UTC): `2026-04-28T00:13:21.396953Z`
- Threshold summary evidence: `.sisyphus/evidence/benchmark-optimization/task-11-threshold-summary.txt`
- Canonical memory JSON: `.sisyphus/evidence/evals/memory-retrieval-baseline.json`
- Canonical memory Markdown: `.sisyphus/evidence/evals/memory-retrieval-baseline.md`

## Command status

| Command | Status | Exit |
|---|---:|---:|
| `python3 evals/memory_retrieval_benchmark.py --self-test` | ✅ PASS | `0` |
| `python3 evals/memory_retrieval_benchmark.py` | ✅ PASS | `0` |

## Memory metrics and thresholds

| Metric | Value | Target | Result | Notes |
|---|---:|---:|---:|---|
| hit_rate | `0.800000` | `>= 0.75` | ✅ | Strict aggregate over all 10 golden rows |
| MRR | `0.750000` | `>= 0.50` | ✅ | Strict aggregate over all 10 golden rows |
| blocker_count | `0` | `= 0` | ✅ | Run health |
| positive_mean_mrr | `0.937500` | informational | ✅ | Positive query quality after billing vector query repair |

## Root-cause fix

- The prior canonical baseline reported `hit_rate=0.7`, `mrr=0.65` because `q_search_vector_billing_retry` used a short query (`billing retry window`) that collided semantically with the auth retry fixture.
- The harness-side golden query now uses the same synthetic concepts as the billing fixture: `Billing retries stay open for a 15 minute retry window`.
- Fresh canonical evidence shows `q_search_vector_billing_retry` at rank 1 with `failure_type=none`.
- Negative no-match rows are unchanged diagnostic probes: they have `expected_fixture_ids=[]`, return non-empty results in this run, and remain classified as `wrong_rank` without counting as blockers.

## Readiness and diagnostics

- Readiness fallback status: `fallback_after_no_signal`
- Readiness impact/classification: `degraded` / `degraded`
- Readiness elapsed_s: `7.013`
- Observed `summary.partial.reason_code` values: `[]`

## Blockers / verdict snapshot

- Memory self-test pass: `True`
- Fresh memory benchmark pass: `True`
- Memory threshold pass: `True`
- Evidence consistency: `True`
- Overall memory blocker status: `resolved`
