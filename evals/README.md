# 📊 Memory Retrieval Benchmarks (V1 Baseline)

This directory contains the benchmarks for the Memory MCP server. 

## 🎯 V1 Scope: Baseline-Only
The current V1 implementation focuses on **baseline data collection**.
- No pass/fail gates are enforced in this version.
- All results are recorded as reference points for future regressions.
- Focused on deterministic retrieval from JSON fixtures.

### 🧪 Benchmark Scope
- **Memory Retrieval**: Evaluates `recall` and `search_memory` performance using golden queries and deterministic fixtures.
- **Fixture Philosophy**: Uses high-fidelity, deterministic JSON fixtures (`evals/fixtures/`) to ensure repeatable results across environments.
- **Metrics**: Track Reciprocal Rank Fusion (RRF) scores, hit rates, and latency. Tracked metrics include **Recall@k**, **NDCG@k** (where k=5, 10), and **MRR**.
- **Readiness/Contract Diagnostics**: Reports preserve raw `summary.partial.reason_code` values and add compact impact classification (`informational`, `degraded`, or `blocking`) so `partial` is not treated as a failure without supporting evidence. Memory reports also surface `settle_readiness` fallback status, elapsed time, and impact.
- **Diagnostic Taxonomy**: Failures are classified into categories like `call_error`, `parse_error`, `embedding_not_ready`, `empty_results`, `id_mismatch` (harness-side remapping), and `wrong_rank`.
- **ID Remapping Logic**: Fixture IDs (from JSON) are mapped to server-generated IDs during seeding. The harness uses a `fixture_to_server_id_map` to evaluate results correctly. A score of zero due to ID mismatch is classified as a harness-side remapping concern, not a server failure.

### 🚫 Exclusions
- **CI Enforcement**: V1 does not block CI/CD pipelines.
- **Multi-Model** (`multi-model`): Testing is primarily against the default embedding model (`e5_small`).
- **LongMemEval**: Large-scale long-context evaluations are deferred to V2+.

---

## 🚀 Execution Commands

### ✅ Evaluation Library (Self-Test)
Use the metrics self-test to verify the evaluation library logic:
```bash
python3 -m evals.lib.metrics --self-test
```

### 🆚 Baseline Diff Report (Memory + Code)
Generate lightweight before/after/delta reports from archived/current baseline JSON artifacts:
```bash
python3 -m evals.lib.metrics --baseline-diff
```

- **Outputs**:
  - `.sisyphus/evidence/evals/memory-retrieval-baseline-diff.json`
  - `.sisyphus/evidence/evals/memory-retrieval-baseline-diff.md`
  - `.sisyphus/evidence/evals/code-retrieval-baseline-diff.json`
  - `.sisyphus/evidence/evals/code-retrieval-baseline-diff.md`
- **Pair selection policy**:
  - Memory diff defaults to latest archived pre-remap baseline (`memory-retrieval-baseline-pre-remap-*.json`) as **before** and current `memory-retrieval-baseline.json` as **after**.
  - Code diff defaults to archived `.sisyphus/evidence/task-2-recall-code-baseline.json` as **before** and current `code-retrieval-baseline.json` as **after**.
- **No gate policy**:
  - These diff artifacts are reporting/evidence only.
  - They do **not** enforce CI pass/fail behavior.

### 📊 Memory Retrieval Benchmark
Evaluates `recall` and `search_memory` performance using golden queries and deterministic fixtures.
- **Self-Test**: `python3 evals/memory_retrieval_benchmark.py --self-test`
- **Run Benchmark**: `python3 evals/memory_retrieval_benchmark.py`
- **Outputs**:
  - `.sisyphus/evidence/evals/memory-retrieval-baseline.json` (Structured metrics)
  - `.sisyphus/evidence/evals/memory-retrieval-baseline.md` (Human-readable summary)
- **Execution Details**:
  - Uses a temporary isolated `DATA_DIR` by default to prevent database lock contention.
  - Defaults to `EMBEDDING_MODEL=e5_small` for fast, lightweight baseline runs.
  - Automatically handles server startup, seeding, and readiness polling.

### 💻 Code Retrieval Benchmark
Evaluates codebase indexing and `recall_code` retrieval using a fixture project.
- **Self-Test**: `python3 evals/code_retrieval_benchmark.py --self-test`
- **Run Benchmark**: `python3 evals/code_retrieval_benchmark.py`
- **Outputs**:
  - `.sisyphus/evidence/evals/code-retrieval-baseline.json`
  - `.sisyphus/evidence/evals/code-retrieval-baseline.md`
- **Execution Details**:
  - Uses an isolated `DATA_DIR` and a temporary copy of the fixture project.
  - Validates full indexing lifecycle: server initialization, embedding readiness, background indexing, and query execution.
  - Surfaces structured blockers if indexing or retrieval fails.

---

## 💾 Baseline Policy & Gates

### V1 Baseline Policy (Current)
The current version is **baseline-only**.
- **No Score Gates**: Baselines are collected for reference; there are no pass/fail score thresholds enforced in V1.
- **Baseline-Not-Gate**: The purpose is to establish a known-good performance profile. Failure to run the benchmark is a "gate" failure, but the specific score achieved is currently informational.
- **Manual Review**: Regressions must be manually identified by comparing new runs against the `.sisyphus/evidence/evals/*-baseline.json` artifacts.
- **Archive Policy**: When major remapping or metric interpretation changes occur, existing baseline artifacts are archived (e.g., `*-pre-remap-*.json`) to preserve historical reference before a new baseline is established.
- **Comparability Note**: These local benchmarks use specific deterministic fixtures and are not directly comparable with external or public benchmark numbers (e.g., MemPalace) due to different corpus and query distributions.
- **Mini Long-Memory Fixture**: The `evals/fixtures/memory_corpus_mini_long_memory.json` is a **synthetic** fixture inspired by the **evaluation shape** (temporal, person/project/topic, namespace) of public long-memory benchmarks like MemPalace. It is designed for lightweight local validation only; performance scores on this fixture do **not** reflect or correlate with public MemPalace benchmark scores.

### Future Policy (Planned)
- **Threshold Enforcement**: V2+ will introduce automated comparison against baselines.
- **CI Gates**: Automated blocking of PRs that significantly degrade MRR (Mean Reciprocal Rank) or hit rates.
- **Multi-Model Regression**: Automated checks across different embedding models (Gemma, Qwen3, etc.).

---

## 💾 Evidence & Data

### Evidence Namespace
All benchmark runs must record evidence in:
`.sisyphus/evidence/evals/`

### ⚠️ Important Warnings

#### 1. Isolated `DATA_DIR`
**Always** use an isolated `DATA_DIR` for benchmark runs to avoid corrupting your production memory or hitting lock contention.
```bash
export DATA_DIR=$(mktemp -d)
```

#### 2. Avoid `docker exec` Lock Hazard
**NEVER** run `docker exec` against a running container to perform benchmark queries or stats collection.
- SurrealDB uses exclusive file locks on the database.
- Running a second process (via `exec`) that attempts to open the same database will fail with a storage lock error.
- **Safe Pattern**: Use `docker run --rm -i ... --stdio` for isolated, one-shot benchmark commands as described in `AGENTS.md`.

#### 3. Stale Harness Warning
Avoid using the following stale scripts:
- `test_mcp.sh`
- `query_stats.sh`
These scripts use outdated MCP tool names (e.g., `search` instead of `search_memory`) and unsafe `docker exec` patterns. They are retained only for historical reference and should not be used for current validation.

---

## 🛠️ Verification
To verify the benchmark environment readiness, ensure:
1. Deterministic fixtures exist in `evals/fixtures/`.
2. The `evals/lib/mcp_client.py` correctly points to the `memory-mcp` binary.
3. Python dependencies are installed.
