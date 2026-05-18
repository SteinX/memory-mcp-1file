# Task 4 Memory Baseline Root-Cause Report

Generated: 2026-04-27T11:20:05.075215+00:00

## Threshold outcome

| Metric | Observed | Threshold | Result |
|---|---:|---:|---|
| blocker_count | 0 | 0 | PASS |
| hit_rate | 0.6 | >= 0.75 | FAIL |
| mrr | 0.6 | >= 0.5 | PASS |
| positive_query_count | 8 | n/a | info |
| positive_mean_mrr | 0.75 | n/a | info |

## Classification

- **BM25/vector readiness:** not the remaining blocker. `blocker_count=0`, `observed_summary_partial_reason_codes=[]`, and six retrieval/search positive rows hit expected memories.
- **get_valid semantics / result-shape mismatch:** primary positive-query failure. `get_valid_temporal` and `get_valid_filtered` return records whose `result_id` is a Surreal record-shaped string such as `{'key': {'String': ...}, 'table': 'memories'}`. The T2 remap expects raw server IDs, so `fixture_id` remains null and scoring records `wrong_rank` even when expected server IDs appear in the returned rows.
- **negative-query behavior:** both negative queries return non-zero results and are classified as `wrong_rank`. This explains aggregate hit-rate drag but is separate from the positive-query quality threshold.
- **query wording:** not enough evidence to blame wording for positive misses; the `get_valid_filtered` row returns exactly two expected previews, so the wording/findability is adequate while scoring/remap semantics are not.
- **real retrieval quality:** not indicated for the positive misses in this run; remaining positive failures are `get_valid` read-path/scoring semantics rather than vector/BM25 retrieval quality.

## Remaining miss evidence

### q_get_valid_temporal_checkpoint

- classification: get_valid semantics
- query_type: get_valid_temporal
- negative: False
- result_count: 15
- expected_rank: None
- failure_type: wrong_rank
- expected_fixture_ids: `["mem_context_namespace_evals", "mem_context_no_private_data", "mem_context_stdio_harness", "mem_decision_cache_ttl", "mem_decision_query_ids", "mem_decision_temporal_utc", "mem_project_benchmark_scope", "mem_research_rrf_fusion", "mem_task_auth_timeout", "mem_task_billing_retry_window", "mem_user_pref_concise_answers", "mem_user_pref_terse_notes"]`
- expected_server_ids: `["3df07c756a6bbf9cfdc4", "6fb4e4a7fa60c0e5d9ab", "3862bc3de76e0a14cb9b", "aaf146982d8e8b7db94a", "6daa3956c7895a1631dc", "a0dc59047d4095cb247c", "e429685f7ddacf28af47", "5b2066edd1095a68101d", "93aec863d81aa5e5dfa0", "7fec235c7b15bd23659f", "2cb216a6eecdfddcd565", "cd14a9826472bbe460b1"]`

```json
{
  "raw_top_k": [
    {
      "fixture_id": null,
      "preview": "CONTEXT: Use namespace evals/memory-retrieval for benchmark runs and fixtures.",
      "rank": 1,
      "result_id": "{'key': {'String': '3df07c756a6bbf9cfdc4'}, 'table': 'memories'}"
    },
    {
      "fixture_id": null,
      "preview": "USER: Prefer terse benchmark notes and short rationale lines.",
      "rank": 2,
      "result_id": "{'key': {'String': 'cd14a9826472bbe460b1'}, 'table': 'memories'}"
    },
    {
      "fixture_id": null,
      "preview": "PROJECT: Recall fusion blends lexical evidence and graph evidence for benchmark queries.",
      "rank": 3,
      "result_id": "{'key': {'String': 'dc89f19abd4583e5243b'}, 'table': 'memories'}"
    },
    {
      "fixture_id": null,
      "preview": "DECISION: Represent temporal windows in ISO 8601 UTC for deterministic parsing.",
      "rank": 4,
      "result_id": "{'key': {'String': 'a0dc59047d4095cb247c'}, 'table': 'memories'}"
    },
    {
      "fixture_id": null,
      "preview": "TASK: This temporary memory expires before the February validity checkpoint.",
      "rank": 5,
      "result_id": "{'key': {'String': '85c06b44737f24df2f0f'}, 'table': 'memories'}"
    },
    {
      "fixture_id": null,
      "preview": "CONTEXT: Never use production, private, or local personal memory content in fixtures.",
      "rank": 6,
      "result_id": "{'key': {'String': '6fb4e4a7fa60c0e5d9ab'}, 'table': 'memories'}"
    },
    {
      "fixture_id": null,
      "preview": "RESEARCH: Temporal validity should be checked with explicit valid_from and valid_until windows.",
      "rank": 7,
      "result_id": "{'key': {'String': '995e3bb0f34eb3312e31'}, 'table': 'memories'}"
    },
    {
      "fixture_id": null,
      "preview": "DECISION: Golden queries must cite stable memory ids exactly as written.",
      "rank": 8,
      "result_id": "{'key': {'String': '6daa3956c7895a1631dc'}, 'table': 'memories'}"
    },
    {
      "fixture_id": null,
      "preview": "TASK: Billing retries stay open for a 15 minute retry window.",
      "rank": 9,
      "result_id": "{'key': {'String': '7fec235c7b15bd23659f'}, 'table': 'memories'}"
    },
    {
      "fixture_id": null,
      "preview": "USER: Keep benchmark notes concise and factual.",
      "rank": 10,
      "result_id": "{'key': {'String': '2cb216a6eecdfddcd565'}, 'table': 'memories'}"
    }
  ]
}
```

### q_get_valid_filtered_auth_namespace

- classification: get_valid semantics
- query_type: get_valid_filtered
- negative: False
- result_count: 2
- expected_rank: None
- failure_type: wrong_rank
- expected_fixture_ids: `["mem_task_auth_timeout", "mem_decision_cache_ttl"]`
- expected_server_ids: `["93aec863d81aa5e5dfa0", "aaf146982d8e8b7db94a"]`

```json
{
  "raw_top_k": [
    {
      "fixture_id": null,
      "preview": "DECISION: Use a 15 minute cache ttl for auth token refresh state.",
      "rank": 1,
      "result_id": "{'key': {'String': 'aaf146982d8e8b7db94a'}, 'table': 'memories'}"
    },
    {
      "fixture_id": null,
      "preview": "TASK: Retry auth requests for 15 minutes when the token refresh path stalls.",
      "rank": 2,
      "result_id": "{'key': {'String': '93aec863d81aa5e5dfa0'}, 'table': 'memories'}"
    }
  ]
}
```

### q_negative_no_match_nonsense

- classification: negative-query behavior
- query_type: negative_no_match
- negative: True
- result_count: 14
- expected_rank: None
- failure_type: wrong_rank
- expected_fixture_ids: `[]`
- expected_server_ids: `[]`

```json
{
  "raw_top_k": [
    {
      "fixture_id": "mem_task_temporal_cutoff",
      "preview": "TASK: This temporary memory expires before the February validity checkpoint.",
      "rank": 1,
      "result_id": "85c06b44737f24df2f0f",
      "score": 0.0065573640167713165
    },
    {
      "fixture_id": "mem_decision_cache_ttl",
      "preview": "DECISION: Use a 15 minute cache ttl for auth token refresh state.",
      "rank": 2,
      "result_id": "aaf146982d8e8b7db94a",
      "score": 0.00645161047577858
    },
    {
      "fixture_id": "mem_user_pref_terse_notes",
      "preview": "USER: Prefer terse benchmark notes and short rationale lines.",
      "rank": 3,
      "result_id": "cd14a9826472bbe460b1",
      "score": 0.006349204573780298
    },
    {
      "fixture_id": "mem_task_auth_timeout",
      "preview": "TASK: Retry auth requests for 15 minutes when the token refresh path stalls.",
      "rank": 4,
      "result_id": "93aec863d81aa5e5dfa0",
      "score": 0.0062499875202775
    },
    {
      "fixture_id": "mem_user_pref_concise_answers",
      "preview": "USER: Keep benchmark notes concise and factual.",
      "rank": 5,
      "result_id": "2cb216a6eecdfddcd565",
      "score": 0.006153843831270933
    },
    {
      "fixture_id": "mem_task_billing_retry_window",
      "preview": "TASK: Billing retries stay open for a 15 minute retry window.",
      "rank": 6,
      "result_id": "7fec235c7b15bd23659f",
      "score": 0.006060594227164984
    },
    {
      "fixture_id": "mem_project_recall_fusion_plan",
      "preview": "PROJECT: Recall fusion blends lexical evidence and graph evidence for benchmark queries.",
      "rank": 7,
      "result_id": "dc89f19abd4583e5243b",
      "score": 0.00597014743834734
    },
    {
      "fixture_id": "mem_research_rrf_fusion",
      "preview": "RESEARCH: RRF fusion should combine vector, BM25, and graph signals for recall.",
      "rank": 8,
      "result_id": "5b2066edd1095a68101d",
      "score": 0.005882350727915764
    },
    {
      "fixture_id": "mem_context_stdio_harness",
      "preview": "CONTEXT: The stdio harness should prefer local binaries before cargo run.",
      "rank": 9,
      "result_id": "3862bc3de76e0a14cb9b",
      "score": 0.0057970997877418995
    },
    {
      "fixture_id": "mem_context_no_private_data",
      "preview": "CONTEXT: Never use production, private, or local personal memory content in fixtures.",
      "rank": 10,
      "result_id": "6fb4e4a7fa60c0e5d9ab",
      "score": 0.005714283790439367
    }
  ]
}
```

### q_negative_no_match_missing_prefix

- classification: negative-query behavior
- query_type: negative_no_match
- negative: True
- result_count: 4
- expected_rank: None
- failure_type: wrong_rank
- expected_fixture_ids: `[]`
- expected_server_ids: `[]`

```json
{
  "raw_top_k": [
    {
      "fixture_id": "mem_decision_cache_ttl",
      "preview": "DECISION: Use a 15 minute cache ttl for auth token refresh state.",
      "rank": 1,
      "result_id": "aaf146982d8e8b7db94a",
      "score": 1.0
    },
    {
      "fixture_id": "mem_task_auth_timeout",
      "preview": "TASK: Retry auth requests for 15 minutes when the token refresh path stalls.",
      "rank": 2,
      "result_id": "93aec863d81aa5e5dfa0",
      "score": 1.0
    },
    {
      "fixture_id": "mem_decision_query_ids",
      "preview": "DECISION: Golden queries must cite stable memory ids exactly as written.",
      "rank": 3,
      "result_id": "6daa3956c7895a1631dc",
      "score": 1.0
    },
    {
      "fixture_id": "mem_decision_temporal_utc",
      "preview": "DECISION: Represent temporal windows in ISO 8601 UTC for deterministic parsing.",
      "rank": 4,
      "result_id": "a0dc59047d4095cb247c",
      "score": 1.0
    }
  ]
}
```

## Conclusion

The corrected memory baseline is trustworthy enough to show non-zero retrieval quality and no blockers, but it does **not** meet the strict hit-rate threshold. The follow-up should normalize/remap `get_valid` record-shaped IDs in the harness before treating these two positive misses as server retrieval failures. Thresholds were not lowered or hidden.
