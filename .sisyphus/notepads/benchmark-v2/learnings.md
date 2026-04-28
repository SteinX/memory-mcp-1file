
## T0 - Legacy contract mapping (2026-04-28)

- `evals/README.md` now documents the Benchmark V2 compatibility bridge without changing harness behavior.
- Legacy memory golden query `version` maps to V2 `schema_version`; existing memory golden files stay legacy/small-tier inputs until tiered files are introduced.
- Canonical V2 code query source is `evals/golden/code_retrieval_queries_v2.json`; `.sisyphus/evidence/task-2-recall-code-baseline.json` remains a legacy bridge/backfill source only.
- New V2 evidence belongs under `.sisyphus/evidence/benchmark-v2/`; `.sisyphus/evidence/evals/` is historical/read-only context.

## T1 - V2 contract fields and determinism policy (2026-04-28)

- `evals/README.md` now defines the documentation-only V2 contract fields: `schema_version`, `fixture_tier`, `baseline_version`, `threshold_policy`, `runtime_target`, and `determinism_policy`.
- T1 keeps harness behavior unchanged: Python benchmark code still validates legacy `version` and default model behavior remains `e5_small` until later wiring tasks.
- V2 `fixture_tier` is explicitly separate from existing per-memory fixture item `tier` labels like `gold` or `mini`.
- Determinism policy now reserves stable fixture/query ordering, deterministic tie-breaking, stable report ordering, and metric tolerances (`1e-9` math, `1e-6` serialized floating-point/report comparisons).

## T2 - Explicit baseline refresh policy and metadata wiring (2026-04-28)

- `evals/memory_retrieval_benchmark.py` now blocks silent canonical baseline overwrite by default: if canonical targets are requested without `--refresh-baseline`, output is redirected to `.sisyphus/evidence/benchmark-v2/runs/`.
- Explicit refresh mode now requires intent metadata (`--refresh-baseline`, `--refresh-reason`) and records V2 baseline fields in report manifest: `schema_version`, `fixture_tier`, `baseline_version`, `model`, `command`, `refresh_reason`, and canonical-write flags.
- `evals/lib/metrics.py` baseline diff payloads now include a machine-consumable `comparison` block with `changed`, `missing`, `improved`, `regressed`, `unchanged`, and per-metric `metric_status` while preserving V1-style metric triplets.
- `evals/README.md` now documents normal non-refresh behavior, explicit refresh command, and baseline diff machine-readable comparison fields for T2 policy clarity.

- `evals/lib/metrics.py` now keeps legacy input defaults on `.sisyphus/evidence/evals/` but moves the four baseline-diff output defaults to `.sisyphus/evidence/benchmark-v2/baseline-diff/` so V2 evidence stays isolated without changing baseline selection.


## T3 - Local threshold policy semantics (2026-04-28)

- `evals/README.md` now defines the central policy `local-v2-threshold-policy` for local-only Benchmark V2 report interpretation.
- T3 status semantics are explicit: `pass` means no threshold findings, `warn` means warning-only threshold failures, `blocker` means blocker-severity threshold failures, and `deferred` means threshold evaluation was intentionally not applied with a recorded reason.
- The policy matrix documents metric key, fixture tier, comparator, threshold value, severity, and denominator rule for `small`, `medium`, and `stress` tiers.
- Removed future-policy wording that suggested Benchmark V2 would add CI gates or PR blocking; thresholds remain local report interpretation only and do not change GitHub Actions, MCP APIs, production behavior, baseline refresh behavior, or benchmark execution semantics.
- `evals/memory_retrieval_benchmark.py` now emits the documented policy name in report metadata while preserving existing local-only enforcement metadata and benchmark behavior.

## T4 - Report contract extensions (2026-04-28)

- `evals/lib/metrics.py` now emits V2 report contract fields at JSON top-level while preserving prior payload shape: `schema_version`, `fixture_tier`, `baseline_version`, `threshold_policy`, `threshold_status`, `readiness_summary`, `failure_buckets`, `baseline_diff_summary`, `metric_summary`, and `deterministic_local_metadata`.
- Shared Markdown report generation now includes a `Benchmark V2 summary` section with human-readable schema/tier/baseline/policy/status/readiness/failure buckets/baseline diff summary/metric summary so memory and code reports stay consistent without removing legacy sections.
- `evals/memory_retrieval_benchmark.py` now computes T3-compatible local-only threshold evaluation and stores `threshold_evaluation` in aggregate metrics; resulting `threshold_status` maps to `pass`/`warn`/`blocker`/`deferred` without affecting command exit gates or baseline refresh behavior.
- `evals/code_retrieval_benchmark.py` now publishes the same V2 report metadata contract and emits explicit `deferred` threshold evaluation with a clear reason because full T3 matrix evaluation is not applied to code benchmark reports yet.
- Self-tests were strengthened to assert V2 contract presence in generated JSON/Markdown outputs, not just legacy query-count/title checks.
- Verification run: `python3 -m evals.lib.metrics --self-test` ✅ and `python3 evals/memory_retrieval_benchmark.py --self-test` ✅.

## T6 - Code retrieval fixture/query expansion and canonical V2 ownership (2026-04-28)

- Canonical V2 code golden query ownership now lives under `evals/golden/code_retrieval_queries_v2.json`; the benchmark no longer depends on `.sisyphus/evidence/task-2-recall-code-baseline.json` as the only source.
- `evals/code_retrieval_benchmark.py` now loads tiered code queries from the canonical V2 file (`small`/`medium`/`stress`) and validates explicit per-query `rationale`, `query_id`, `query`, `query_type`, `expected_paths`, and `expected_symbols`.
- Legacy bridge behavior is explicit and limited: fallback to `.sisyphus/evidence/task-2-recall-code-baseline.json` is only used for `small` tier if canonical small queries are unavailable; report metadata now records bridge usage (`legacy_bridge.used`).
- Medium-tier code scenarios now explicitly cover symbol-definition lookup, caller/callee relationship expectations, file-path lookup, similar-function interference, deleted/renamed symbol expectations, and hybrid-vs-vector disagreement pairs.
- Stress-tier manifest is present but remains non-default/manual (`enabled_by_default: false`) to preserve local-runtime defaults and avoid scope creep.
- Verification: `python3 evals/code_retrieval_benchmark.py --self-test` passed, including medium-tier query load validation (15 small + 14 medium queries).

## T5 - Medium/stress memory fixture expansion (2026-04-28)

- Added medium-tier synthetic long-memory fixture and golden query files:
  - `evals/fixtures/memory_corpus_medium_long_memory.json` (13 memories)
  - `evals/golden/memory_retrieval_queries_medium_long_memory.json` (7 queries)
- Medium golden queries now include `label_rationale` metadata per query with explicit scenario category, tier, why-label-is-correct text, expected behavior, expected IDs, and no-match flag.
- Required medium scenario coverage is now validated in harness self-test via `_validate_medium_long_memory_caps`: `long_memory_recall`, `namespace_boundary`, `temporal_boundary`, `negative_no_match`, `partial_readiness`, `id_mismatch_alias`, and `record_shaped_ids`.
- Added stress-tier manifest-only definitions (no heavy default execution):
  - `evals/fixtures/memory_corpus_stress_manifest.json`
  - `evals/golden/memory_retrieval_queries_stress_manifest.json`
- `evals/memory_retrieval_benchmark.py --self-test` now validates baseline + mini + medium fixtures and stress-manifest presence while preserving default small-tier benchmark run behavior.

## T7 - Label QA and fixture drift validation (2026-04-28)

- Memory benchmark validation now has shared QA helpers for fixture identity integrity, duplicate diagnostics with offending value counts, explicit metadata alias collision detection, and `label_rationale` consistency.
- Medium memory self-test now exercises invalid in-memory copies for duplicate IDs, alias collision, missing `label_rationale`, drifted `label_rationale.expected_ids`, and negative/no-match inconsistency; validation fails with query/memory-specific `AssertionError` messages and does not mutate fixture/golden files.
- Code retrieval query validation now checks positive expected paths against repository files and verifies expected symbols appear in those expected files; negative/deleted/renamed sentinel queries must keep empty expected labels.
- Code self-test now includes invalid-copy rejection assertions for duplicate query IDs, missing rationale, missing expected path, missing expected symbol, and non-empty sentinel labels.
- Verification: `python3 evals/memory_retrieval_benchmark.py --self-test` ✅ and `python3 evals/code_retrieval_benchmark.py --self-test` ✅; LSP diagnostics on both modified eval scripts reported no diagnostics.

## T9 - Fixture tier wiring for code benchmark (2026-04-28)

- `evals/code_retrieval_benchmark.py` now supports `--tier` as a compatibility alias for `--fixture-tier` while keeping `small` as default and preserving default model behavior (`e5_small`).
- Fixture tier parsing is explicit and normalized; invalid values fail with a clear error listing valid choices: `small, medium, stress`.
- Self-test now validates tier source/path resolution for all three tiers (`small`, `medium`, `stress`) against canonical V2 source `evals/golden/code_retrieval_queries_v2.json` and records per-tier counts/source metadata.
- Benchmark report metadata now includes selected fixture tier, baseline version, model, retrieval modes, canonical query/catalog source, and label QA summary status (`validated`).
- QA evidence captured under `.sisyphus/evidence/benchmark-v2/task-9/`: self-test logs, small-tier run outputs, and invalid-tier stderr/stdout transcripts.

## T8 - Memory fixture tier wiring (2026-04-28)

- `evals/memory_retrieval_benchmark.py` now treats fixture tier as a real input selector, not metadata-only: `small` maps to mini long-memory files, `medium` maps to medium long-memory files, and `stress` maps to stress manifests.
- Added `--tier` as an alias for `--fixture-tier` while keeping `small` as the default and preserving default model `e5_small`.
- Invalid tier handling is explicit and operator-visible via argparse error with valid choices (`small, medium, stress`), and QA evidence is captured under `.sisyphus/evidence/benchmark-v2/task-8/invalid-tier.*`.
- Report/runtime metadata now follows selected tier across manifest/top-level fields, runtime target policy, fixture source paths, and non-refresh evidence naming (e.g. `memory_retrieval_baseline-stress-<timestamp>.json`).
- During QA, small-tier initially failed because seed validation still used legacy V1 baseline caps; fixed by tier-aware validation inside `seed_memory_fixtures` so small/medium use their dedicated validators.
- Stress tier remains non-default and manifest-only in benchmark execution (`seed_progress.status=deferred`) so normal runs do not attempt heavy stress seeding or mutate canonical baselines.


## T10 - Benchmark V2 readiness exploration (2026-04-28T12:06:04.201594Z)

- Explored only benchmark-owned implementation points; no source/test/fixture/doc/plan files were edited. Allowed append-only notepad update performed here.
- Shared report emission lives in `evals/lib/metrics.py`: `write_json_report` emits top-level `threshold_status`, `readiness_summary`, `failure_buckets`, `baseline_diff_summary`, `schema_version`, `fixture_tier`, and `deterministic_local_metadata`; `write_markdown_report` renders the same V2 summary sections.
- `baseline_diff` machinery is centralized in `evals/lib/metrics.py` via `_build_baseline_diff`, `_baseline_diff_summary`, `write_baseline_diff_json`, `write_baseline_diff_markdown`, and `generate_baseline_diff_artifacts`; normal reports default `baseline_diff_summary` to deferred unless aggregate metrics supply it.
- Memory harness readiness/report inputs: `evals/memory_retrieval_benchmark.py::_classify_failure_type` distinguishes `call_error`, `parse_error`, `embedding_not_ready`, `expected_no_match`, `empty_results`, `id_mismatch`, `wrong_rank`, and `none`; `execute_query` stores that as per-query `failure_type`; `run_benchmark` aggregates reason codes/readiness fallback and calls `_evaluate_threshold_status` for actual local V2 threshold status.
- Memory expected no-match vs empty result are distinguishable: negative queries with zero results become `expected_no_match`, while positive queries with zero results become `empty_results`. Low confidence is not a distinct summary category: raw top-k scores can be captured by `_extract_item_score`/`_extract_raw_top_k`, but `_classify_failure_type` does not use score/confidence thresholds. True miss is not separately named; positive non-hit with results is currently `wrong_rank` unless ID remapping failed (`id_mismatch`).
- Code harness readiness/report inputs: `evals/code_retrieval_benchmark.py::_threshold_evaluation_deferred` always sets threshold interpretation to `deferred`; `run_benchmark` records blockers, observed reason codes, readiness timeout, baseline diff deferred summary, and V2 manifest fields, but per-query rows do not currently include `failure_type`.
- T10 gap: shared `failure_buckets` counts only per-query `failure_type`, so memory reports have meaningful buckets while code reports are likely empty even when queries miss, return empty results, hit sentinel no-match cases, or have `query_error`. Code negative/deleted/renamed sentinels are validated at label QA level, but runtime summaries do not classify expected no-match vs true miss/empty/wrong-rank in the shared taxonomy.
- Recommended short checks for T10 (no long benchmark): run `python3 -m evals.lib.metrics --self-test`, `python3 evals/memory_retrieval_benchmark.py --self-test`, and `python3 evals/code_retrieval_benchmark.py --self-test`; inspect generated/self-test JSON for presence of V2 top-level fields and confirm code self-test `failure_buckets` behavior remains intentional or is updated by T10.

## Verification audit - T8/T9 (2026-04-28T12:07:50Z)

- Scope: Read-only verification of T8/T9 implementation and safe CLI/self-test checks; no source edits.
- T8 verdict: PASS. `evals/memory_retrieval_benchmark.py` accepts explicit `--fixture-tier` + `--tier` alias, defaults to `small`, validates invalid tier values, resolves per-tier fixture/query paths, records `fixture_tier` + `runtime_target` in report metadata, and self-test validates tier path resolution (`tier_resolution_summary`).
- T9 verdict: PASS. `evals/code_retrieval_benchmark.py` accepts explicit `--fixture-tier` + `--tier` alias, defaults to `small`, reads canonical V2 source `evals/golden/code_retrieval_queries_v2.json`, records tier/source metadata in report, self-test validates per-tier source/path resolution, and stress is non-default.
- Command verification (current working tree):
  - `python3 evals/memory_retrieval_benchmark.py --self-test` -> passed.
  - `python3 evals/code_retrieval_benchmark.py --self-test` -> passed.
  - `python3 evals/memory_retrieval_benchmark.py --tier invalid` -> rejects with valid choices `small, medium, stress`.
  - `python3 evals/code_retrieval_benchmark.py --tier invalid` -> rejects with valid choices `small, medium, stress`.
- Existing evidence corroboration: `.sisyphus/evidence/benchmark-v2/task-8/memory-small.json`, `.sisyphus/evidence/benchmark-v2/task-8/self-test.stdout`, `.sisyphus/evidence/benchmark-v2/task-8/invalid-tier.stderr`, `.sisyphus/evidence/benchmark-v2/task-9/code-small.json`, `.sisyphus/evidence/benchmark-v2/task-9/self-test.stdout`, `.sisyphus/evidence/benchmark-v2/task-9/invalid-tier.stderr`.

## T10 - V2 report summary taxonomy integration (2026-04-28T12:17:12.984364Z)

- `evals/lib/metrics.py` self-test now asserts concrete JSON/Markdown `failure_buckets` output for `none`, `expected_no_match`, `empty_results`, and `true_miss`, covering the shared writer contract rather than only field presence.
- `evals/memory_retrieval_benchmark.py` now keeps blocker/id-remap classifications intact while distinguishing positive non-hit rows with results as `true_miss` and score-bearing very weak results as `low_confidence`; positive zero-result rows remain `empty_results`, and successful negative rows remain `expected_no_match`.
- `evals/code_retrieval_benchmark.py` now assigns per-query `failure_type` so code reports populate shared `failure_buckets`: successful hits use `none`, expected empty sentinel rows use `expected_no_match`, positive empty rows use `empty_results`, non-empty misses use `true_miss`, and query exceptions use `call_error`.
- Self-tests now cover representative pass/warning/blocker-style report cases via synthetic rows and validate JSON/Markdown summary taxonomy for both benchmark harnesses without starting MCP or mutating baselines.
- Verification passed: `python3 -m evals.lib.metrics --self-test`, `python3 evals/memory_retrieval_benchmark.py --self-test`, `python3 evals/code_retrieval_benchmark.py --self-test`; LSP diagnostics reported no issues for the three modified Python files.

## T11 - Update evals README V2 runbook (2026-04-28)

- Updated `evals/README.md` to establish the **Benchmark V2 Runbook** as the primary operator-facing guide.
- Consolidated previous task findings (T0-T10) into structured sections: Fixture Tiers, Baseline Refresh Workflow, Local Threshold Policy, and Failure Taxonomy.
- Tier instructions are explicit: `small` (default), `medium` (explicit), and `stress` (manual) with target runtimes.
- Baseline refresh workflow documents intentionality: requires `--refresh-baseline` and `--refresh-reason`.
- Failure taxonomy distinguishes `expected_no_match`, `empty_results`, `low_confidence`, and `true_miss`.
- Qwen3/multi-model comparison and CI gates remain documented as deferred or local-only.
- Stale scripts (`test_mcp.sh`, `query_stats.sh`) are clearly marked as stale/deprecated.
- Verification: README grep checks confirm required terms/sections exist and stale scripts remain avoided.

### 2024-04-28: T11 Documented V2 Report Fields
- Added explicit documentation for Benchmark V2 report fields in `evals/README.md`.
- Covered `schema_version`, `fixture_tier`, `baseline_version`, `threshold_status`, `readiness_summary`, `failure_buckets`, `baseline_diff_summary`, and `metric_summary`.
- This ensures operators understand the structured output from `evals/lib/metrics.py` without deep-diving into code.


## T12 - Final V2 self-test and benchmark bundle (2026-04-28T12:36:00Z)

- Created final evidence bundle directory: `.sisyphus/evidence/benchmark-v2/final-bundle/`.
- Captured required self-tests in `self-tests.txt` and `self-tests-status.json`; all three commands exited 0 (`metrics`, `memory`, `code`).
- Ran representative small-tier benchmark executions with explicit outputs:
  - `memory-small.json` / `memory-small.md`
  - `code-small.json` / `code-small.md`
  Both commands exited 0.
- Verified required V2 JSON fields are present in both small-tier reports (`schema_version`, `fixture_tier`, `baseline_version`, `threshold_policy`, `threshold_status`, `readiness_summary`, `failure_buckets`, `baseline_diff_summary`, `metric_summary`) and recorded proof in `v2-field-check.txt`.
- Verified Markdown reports contain readable `Benchmark V2 summary` sections for memory and code.
- Executed medium-tier validation runs for both harnesses (`memory-medium.*`, `code-medium.*`); both exited 0, so runtime-budget skip was not needed.
- Proved baseline refresh dry-run behavior with non-canonical output paths (`memory-refresh-dry-run.json/.md`) and canonical checksum guard; `baseline-refresh-dry-run-status.json` records `canonical_mutated=false`.
- Ran forbidden-scope diff check and saved transcript to `forbidden-scope.txt`; output is empty for `src/**`, `.github/workflows/**`, root `README.md`, and dependency manifests.

## F3 - Real benchmark QA verification wave (2026-04-28)

- Added fresh F3 evidence bundle under `.sisyphus/evidence/benchmark-v2/final-wave/f3/` with independent reruns of required self-tests plus small/medium memory+code benchmarks.
- All required self-tests exited 0 and are captured in `self-tests-status.json` with transcripts (`self-test-metrics.txt`, `self-test-memory.txt`, `self-test-code.txt`).
- Fresh benchmark artifacts were generated for small and medium tiers (`memory-*.json/.md`, `code-*.json/.md`) and all four runs reported `blocker_count=0`.
- Contract QA confirmed required V2 JSON fields across all fresh reports (`v2-field-check.json`) and Markdown readability sections (`## Benchmark V2 summary`) across all four Markdown reports.
- T12 consistency check (`final-bundle-consistency.json`) confirmed final-bundle artifacts still exist and align on schema/tier/baseline/policy/status with fresh F3 runs.
- F3 final summary recorded as `Scenarios [8/8 pass] | Reports [8/8] | VERDICT: APPROVE` in `f3-summary.md` and `f3-summary.json`.

## F2 - Code baseline output guard fix (2026-04-28T13:15:26.467990Z)

- Root cause fixed in `evals/code_retrieval_benchmark.py`: normal/default runs no longer write canonical legacy baseline files (`.sisyphus/evidence/evals/code-retrieval-baseline.json/.md`).
- Added canonical-target guard helpers (`_is_canonical_target_pair`, `_non_refresh_report_paths`) and redirected only when both default canonical targets are used; explicit `--output-json` / `--output-md` paths remain authoritative and unchanged.
- Added self-test assertions to lock the guard policy and output path shape under `.sisyphus/evidence/benchmark-v2/runs/`.
- Verification evidence captured at `.sisyphus/evidence/benchmark-v2/f2-fix/code-default-no-canonical-mutation.json` with `canonical_mutated=false` and redirected output path under benchmark-v2 runs.
- Explicit-output compatibility check passed via `python3 evals/code_retrieval_benchmark.py --output-json ... --output-md ...` producing exactly the requested files.
