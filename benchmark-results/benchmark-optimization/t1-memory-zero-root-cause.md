# T1 Memory Zero-Score Root Cause Evidence

## Classification

- `failure_type`: `id_mismatch`
- Basis: existing `.sisyphus/evidence/evals/memory-retrieval-baseline.json`; no benchmark rerun and no baseline rewrite.

## Aggregate metrics from current baseline

| Metric | Value |
|---|---:|
| `hit_rate` | `0.0` |
| `mrr` | `0.0` |
| `precision_at_5` | `0.0` |
| `precision_at_10` | `0.0` |
| `positive_query_count` | `8` |
| `query_count` | `10` |
| `blocker_count` | `0` |
| `seed_completed` | `True` |
| `embedding_readiness.status` | `ready` |
| `query_diagnostics.call_error_count` | `0` |
| `query_diagnostics.parse_issue_count` | `0` |

## ID intersection proof

- Positive queries: `8`
- Positive queries with non-empty result sets: `8`
- Fixture-ID intersection with returned IDs: `[]`
- Server-ID intersection count after fixture-order mapping: `13`
- Sample expected fixture IDs: `['mem_context_namespace_evals', 'mem_context_no_private_data', 'mem_context_stdio_harness', 'mem_decision_cache_ttl', 'mem_decision_query_ids', 'mem_decision_temporal_utc', 'mem_project_benchmark_scope', 'mem_research_rrf_fusion']`
- Sample server-generated IDs: `['428b8ddb1059c8777a64', '2f9b4794f3f68215f8f2', '85f0b6b58553b07caa56', '20d6b02163e07050b73f', '12c2f5431112613a4fae', '78edb1f546ca57518785', '585db599a4a84a8e158c', 'd97ca23bf11ac5bec2a7']`
- Sample raw returned IDs: `['2f9b4794f3f68215f8f2', '585db599a4a84a8e158c', 'bed28bf66e249b7cc66c', '745e56ebf2ffc4582494', 'c3195d04e31748533a43', '620a3fce4ccc6a9ca7bd', '78edb1f546ca57518785', 'd97ca23bf11ac5bec2a7']`

The scorer compares golden fixture IDs such as `mem_task_auth_timeout` directly with server-generated result IDs such as `428b8ddb1059c8777a64`. The positive result sets are non-empty, but the raw fixture-ID intersection is empty.

## Fixture ID -> server ID -> returned ID samples

| Query | Fixture ID | Server ID from seed_progress | Returned ID | Returned rank |
|---|---|---|---|---:|
| `q_recall_fusion_auth_timeout` | `mem_task_auth_timeout` | `428b8ddb1059c8777a64` | `428b8ddb1059c8777a64` | 15 |
| `q_recall_fusion_auth_timeout` | `mem_decision_cache_ttl` | `2f9b4794f3f68215f8f2` | `2f9b4794f3f68215f8f2` | 1 |
| `q_recall_fusion_benchmark_scope` | `mem_project_benchmark_scope` | `12c2f5431112613a4fae` | `12c2f5431112613a4fae` | 4 |
| `q_recall_fusion_benchmark_scope` | `mem_decision_query_ids` | `d97ca23bf11ac5bec2a7` | `d97ca23bf11ac5bec2a7` | 2 |
| `q_recall_fusion_terse_notes` | `mem_user_pref_concise_answers` | `78edb1f546ca57518785` | `78edb1f546ca57518785` | 1 |
| `q_recall_fusion_terse_notes` | `mem_user_pref_terse_notes` | `620a3fce4ccc6a9ca7bd` | `620a3fce4ccc6a9ca7bd` | 2 |
| `q_search_bm25_cache_prefix` | `mem_decision_cache_ttl` | `2f9b4794f3f68215f8f2` | `2f9b4794f3f68215f8f2` | 1 |
| `q_search_bm25_temporal_windows` | `mem_research_temporal_windows` | `745e56ebf2ffc4582494` | `745e56ebf2ffc4582494` | 2 |

## Per-positive-query summary

| Query | Type | Result count | Fixture-ID intersection | Server-ID intersection | Returned sample |
|---|---|---:|---|---|---|
| `q_recall_fusion_auth_timeout` | `recall_fusion` | 15 | `[]` | `['2f9b4794f3f68215f8f2', '428b8ddb1059c8777a64']` | `['2f9b4794f3f68215f8f2', '585db599a4a84a8e158c', 'bed28bf66e249b7cc66c', '745e56ebf2ffc4582494', 'c3195d04e31748533a43']` |
| `q_recall_fusion_benchmark_scope` | `recall_fusion` | 14 | `[]` | `['12c2f5431112613a4fae', 'd97ca23bf11ac5bec2a7']` | `['829ddcbaf1174b348466', 'd97ca23bf11ac5bec2a7', '78edb1f546ca57518785', '12c2f5431112613a4fae', '25f6db9c8703b41cd88d']` |
| `q_recall_fusion_terse_notes` | `recall_fusion` | 14 | `[]` | `['620a3fce4ccc6a9ca7bd', '78edb1f546ca57518785']` | `['78edb1f546ca57518785', '620a3fce4ccc6a9ca7bd', '829ddcbaf1174b348466', '745e56ebf2ffc4582494', '85f0b6b58553b07caa56']` |
| `q_search_bm25_cache_prefix` | `search_bm25` | 4 | `[]` | `['2f9b4794f3f68215f8f2']` | `['2f9b4794f3f68215f8f2', '428b8ddb1059c8777a64', 'd97ca23bf11ac5bec2a7', 'bed28bf66e249b7cc66c']` |
| `q_search_bm25_temporal_windows` | `search_bm25` | 3 | `[]` | `['745e56ebf2ffc4582494', 'bed28bf66e249b7cc66c']` | `['bed28bf66e249b7cc66c', '745e56ebf2ffc4582494', 'c3195d04e31748533a43']` |
| `q_search_vector_billing_retry` | `search_vector` | 14 | `[]` | `['585db599a4a84a8e158c']` | `['585db599a4a84a8e158c', 'bed28bf66e249b7cc66c', '2f9b4794f3f68215f8f2', '745e56ebf2ffc4582494', '620a3fce4ccc6a9ca7bd']` |
| `q_get_valid_temporal_checkpoint` | `get_valid_temporal` | 15 | `[]` | `['12c2f5431112613a4fae', '20d6b02163e07050b73f', '2f9b4794f3f68215f8f2', '428b8ddb1059c8777a64', '585db599a4a84a8e158c', '620a3fce4ccc6a9ca7bd', '78edb1f546ca57518785', '829ddcbaf1174b348466', '85f0b6b58553b07caa56', 'bed28bf66e249b7cc66c', 'd97ca23bf11ac5bec2a7', 'db579f0f12dbb3041716']` | `["{'key': {'String': '829ddcbaf1174b348466'}, 'table': 'memories'}", "{'key': {'String': '620a3fce4ccc6a9ca7bd'}, 'table': 'memories'}", "{'key': {'String': '25f6db9c8703b41cd88d'}, 'table': 'memories'}", "{'key': {'String': 'bed28bf66e249b7cc66c'}, 'table': 'memories'}", "{'key': {'String': 'c3195d04e31748533a43'}, 'table': 'memories'}"]` |
| `q_get_valid_filtered_auth_namespace` | `get_valid_filtered` | 2 | `[]` | `['2f9b4794f3f68215f8f2', '428b8ddb1059c8777a64']` | `["{'key': {'String': '2f9b4794f3f68215f8f2'}, 'table': 'memories'}", "{'key': {'String': '428b8ddb1059c8777a64'}, 'table': 'memories'}"]` |

## Ruled-out categories

- `empty_results`: ruled out because all 8 positive queries returned non-empty result sets.
- `embedding_not_ready`: ruled out because `embedding_readiness.status=ready` and `seed_completed=True`.
- `fixture_query_mismatch`: ruled out for the zero-score root cause because golden fixture IDs exist in the fixture, and their mapped server IDs appear in returned result sets.
- `tool_error`: ruled out because `blocker_count=0`, `call_error_count=0`, and `parse_issue_count=0`.

## Conclusion

Memory retrieval returned server-generated memory IDs, but the benchmark scored against fixture-authored IDs. The current zero metrics are therefore a harness scoring ID mismatch, not proof that retrieval returned no relevant memories.
