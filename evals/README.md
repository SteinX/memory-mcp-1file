# 📊 Memory Retrieval Benchmarks (V2)

This directory contains the benchmarks for the Memory MCP server. Benchmark V2 is a reproducible, tiered, comparable evaluation loop designed for local regression visibility and dataset quality.

## 🎯 Benchmark V2 Scope
Benchmark V2 focuses on providing a trustworthy local runbook for evaluating retrieval quality.
- **Reproducible**: Tiered deterministic fixtures and golden queries with explicit schemas.
- **Comparable**: Local interpretation policy (`local-v2-threshold-policy`) with baseline diffing.
- **Explainable**: Failure taxonomy distinguishing expected no-match, empty results, and true misses.
- **Non-blocking**: Thresholds are for local interpretation only and do not change CI behavior or production server APIs.

### 🧪 Benchmark Scenarios
- **Memory Retrieval**: Evaluates `recall` and `search_memory` performance using golden queries covering long-memory recall, namespace boundaries, temporal boundaries, and ID mismatch cases.
- **Code Retrieval**: Evaluates codebase indexing and `recall_code` performance covering symbol definitions, caller/callee relationships, and similar-function interference.
- **Readiness & Contract Diagnostics**: Reports preserve `reason_code` taxonomy and classify impact as `informational`, `degraded`, or `blocking`.

### 🚫 Exclusions & Non-Goals
- **CI Gates**: V2 does not block CI/CD pipelines.
- **Multi-Model Deferral**: Qwen3 and multi-model comparison are **deferred** until dataset/baseline quality is stable. Default model remains `e5_small`.
- **Production Changes**: V2 is a test-only harness and does not modify `src/` or MCP tool contracts.

---

## 🚀 Execution Commands

### ✅ Evaluation Library (Self-Test)
Verify the evaluation library logic, report schemas, and tier resolution:
```bash
python3 -m evals.lib.metrics --self-test
```

### 📊 Memory Retrieval Benchmark
```bash
# Self-Test (Logic only)
python3 evals/memory_retrieval_benchmark.py --self-test

# Run default (small tier)
python3 evals/memory_retrieval_benchmark.py

# Run specific tier
python3 evals/memory_retrieval_benchmark.py --tier medium
```

### 💻 Code Retrieval Benchmark
```bash
# Self-Test (Logic only)
python3 evals/code_retrieval_benchmark.py --self-test

# Run default (small tier)
python3 evals/code_retrieval_benchmark.py

# Run specific tier
python3 evals/code_retrieval_benchmark.py --tier medium
```

---

## 💾 Runbook: Tiers & Baselines

### 🪜 Fixture Tiers
| Tier | Purpose | Target Runtime | Policy |
|---|---|---|---|
| `small` | Smoke/Regression (Default) | 5-10 mins | Required for local validation. |
| `medium` | Expanded edge cases & Label QA | 15-30 mins | Validation required; full execution explicit. |
| `stress` | Scale & Soak (Non-default) | 45-90+ mins | Manual/optional; skip if runtime is high. |

### 🔄 Baseline Refresh Workflow
Normal benchmark runs **never** overwrite canonical baseline artifacts. Refresh is an intentional, explicit workflow.
- **Default Evidence Path**: `.sisyphus/evidence/benchmark-v2/runs/`
- **Canonical Baseline Paths**: `.sisyphus/evidence/evals/memory-retrieval-baseline.json` (and `.md`)

To perform an intentional refresh:
```bash
python3 evals/memory_retrieval_benchmark.py \
  --refresh-baseline \
  --refresh-reason "Updated medium-tier golden queries" \
  --baseline-version v2-20240428 \
  --fixture-tier small
```

### 🆚 Baseline Diff Report
Compare current results against canonical baselines:
```bash
python3 -m evals.lib.metrics --baseline-diff
```
Diff outputs default to `.sisyphus/evidence/benchmark-v2/baseline-diff/`.

---

## 🧐 Report Interpretation

### 🚥 Local Threshold Policy (`local-v2-threshold-policy`)
Threshold statuses are local interpretation only:
- **`pass`**: All metrics satisfy the tier threshold.
- **`warn`**: Warning-severity failure (e.g., latency > 5s or NDCG < 0.75).
- **`blocker`**: Blocker-severity failure (e.g., hit_rate < 0.8 or MRR < 0.7).
- **`deferred`**: Threshold evaluation skipped (e.g., stress tier).

### 📉 Failure Taxonomy & Report Fields
Reports distinguish between different failure modes and use a standardized schema for compatibility.

#### Core Report Fields
- **`schema_version`**: The version of the benchmark report schema (e.g., `v2`).
- **`fixture_tier`**: The complexity tier of the dataset used (`small`, `medium`, `stress`).
- **`baseline_version`**: The identifier of the canonical baseline being compared against.
- **`threshold_policy`**: The policy name used for status evaluation (e.g., `local-v2-threshold-policy`).
- **`threshold_status`**: The final local status (`pass`, `warn`, `blocker`, `deferred`).
- **`readiness_summary`**: High-level diagnostic of harness and model readiness.
- **`failure_buckets`**: Breakdown of failures by mode (see Taxonomy below).
- **`baseline_diff_summary`**: Summary of performance changes relative to the baseline.
- **`metric_summary`**: Aggregated performance metrics (Hit Rate, MRR, NDCG, Latency).

#### Failure Mode Taxonomy
Failure modes tracked in `failure_buckets`:
- **`expected_no_match`**: Negative query correctly returned zero results.
- **`empty_results`**: Positive query returned zero results (Severe).
- **`low_confidence`**: Result returned but with very low relevance score.
- **`true_miss`**: Results returned, but the ground-truth ID was missing or ranked poorly.
- **`id_mismatch`**: Harness-side remapping failed (check re-map policy).

### 🏷️ Label QA Requirements
Every new golden query MUST include a `label_rationale` object:
- `rationale`: Explanation of why the label is correct.
- `category`: Scenario category (e.g., `long_memory_recall`).
- `expected_behavior`: What the model should specifically find.
- `tier`: The intended fixture tier (`small`, `medium`, `stress`).

---

## ⚠️ Important Warnings

### 1. Isolated `DATA_DIR`
**Always** use an isolated `DATA_DIR` for benchmark runs to avoid database lock contention.
```bash
export DATA_DIR=$(mktemp -d)
```

### 2. Avoid `docker exec` Lock Hazard
**NEVER** run `docker exec` against a running container to perform benchmark queries. SurrealDB uses exclusive file locks. Use `docker run --rm -i ... --stdio` instead.

### 3. Stale Harness Warning
The following scripts are **stale/deprecated** and should be avoided:
- `test_mcp.sh`
- `query_stats.sh`
They use outdated tool names and unsafe patterns.

---

## 🏺 Historical Continuity

### V2 Compatibility Map
| Legacy Field | V2 Canonical Target |
|---|---|
| `version` (V1) | `schema_version` |
| Legacy memory queries | Tiered files (`small`, `medium`, `stress`) |
| `task-2-recall-code-baseline.json` | `evals/golden/code_retrieval_queries_v2.json` |
| `.sisyphus/evidence/evals/` | `.sisyphus/evidence/benchmark-v2/` (New) |

### V1 Baseline Context (V1 Baseline)
Historical V1 implementation focused on baseline data collection without gates. V2 preserves this continuity by using `small` tier as the V1 baseline bridge.
