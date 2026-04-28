# Benchmark V2 Plan

## TL;DR

> **Quick Summary**: Turn the current benchmark harness from a trustworthy V1 diagnostic/baseline tool into Benchmark V2: a reproducible, tiered, comparable evaluation loop with expanded memory/code retrieval scenarios.
>
> **Deliverables**:
> - Fixture tiers: `small`, `medium`, `stress` with documented size/runtime targets
> - Versioned fixture/report/baseline schemas and intentional baseline-refresh workflow
> - Unified reports with baseline diff, readiness taxonomy, failure buckets, Recall@k, MRR, and NDCG
> - Expanded memory retrieval scenarios and code retrieval scenarios
> - Warning/blocking threshold policy for local benchmark interpretation, without CI blocking gates
> - Updated benchmark V2 runbook documenting Qwen3 deferral
>
> **Estimated Effort**: Large
> **Parallel Execution**: YES — 5 waves
> **Critical Path**: T0 → T1 → T5/T6 → T8/T9 → T12 → Final Verification

---

## Context

### Original Request

After completing `benchmark-optimization`, user asked: `根据目前的 benchmark 结果来看，what's next ? 有什么是我们可以做的?`

User selected priorities `1/3 可做`: stabilize the benchmark system and expand benchmark datasets/scenario coverage. User then confirmed generating this formal work plan with: `好`.

### Current State

- Benchmark V1 is usable, trustworthy, and diagnosable.
- Completed optimization reached memory retrieval `hit_rate=0.8`, `mrr=0.75`, `blocker_count=0`.
- V1 limitations remain: mini fixture is tiny, benchmark is baseline-only, default remains `e5_small`, no Qwen3/multi-model matrix exists, code retrieval has a follow-up threshold exception, and stale scripts are only documented as avoid/stale.

### Research Findings

- `evals/README.md:1-23` defines V1 baseline-only scope, deterministic JSON fixtures, metrics, readiness diagnostics, ID remapping, and exclusions.
- `evals/README.md:78-93` documents no score gates in V1 and mentions future V2 threshold/multi-model work.
- `.sisyphus/plans/benchmark-optimization.md:67-113` established important guardrails: no `src/` server changes, no API contract changes, no direct DB writes, no CI gates, no public dataset import, no multi-model matrix, no MemPalace rewrite.
- Metis review recommended explicitly adding determinism, baseline-refresh control, runtime budgets, threshold semantics, label QA, fixture drift checks, ranking tie handling, temporal boundary handling, namespace leakage checks, ID collision checks, and Qwen3 deferral.

---

## Work Objectives

### Core Objective

Create Benchmark V2 as a reproducible, comparable evaluation loop for Memory MCP retrieval quality. V2 should make regressions visible, explainable, and reviewable without changing production server behavior or introducing CI blocking gates.

### Concrete Deliverables

- Tiered deterministic benchmark fixtures and golden queries.
- Schema-versioned fixture, baseline, and report formats.
- Explicit baseline refresh workflow that prevents accidental overwrite.
- Unified report JSON/Markdown outputs for memory and code retrieval.
- Expanded memory retrieval and code retrieval scenario coverage.
- Local warning/blocking threshold policy.
- Benchmark V2 runbook in `evals/README.md`.

### Definition of Done

- [ ] `python3 -m evals.lib.metrics --self-test` passes.
- [ ] `python3 evals/memory_retrieval_benchmark.py --self-test` passes.
- [ ] `python3 evals/code_retrieval_benchmark.py --self-test` passes.
- [ ] Small-tier memory and code benchmarks run successfully with `blocker_count=0`.
- [ ] Medium-tier fixtures and golden queries exist and are documented.
- [ ] Stress-tier definitions exist; stress execution may be optional/manual if runtime is high.
- [ ] Reports include schema version, fixture tier, baseline version, threshold result, failure buckets, readiness taxonomy, Recall@k, MRR, NDCG, and baseline diff.
- [ ] Baseline refresh requires an explicit flag/mode and writes clear metadata.
- [ ] Qwen3/multi-model work remains documented as deferred.

### Must Have

- Preserve current V1 diagnostics: raw top-k, failure taxonomy, readiness fallback, reason-code classification, ID remapping.
- Keep default model as `e5_small`.
- Keep V2 focused on local benchmark quality and regression visibility.
- Add dataset label QA: every new golden query must explain expected IDs and failure mode covered.
- Add deterministic scoring policy for ties and ambiguous results.
- Add runtime targets per fixture tier.
- Add a compatibility map for legacy fields (`version`, legacy code baseline query source, existing evidence namespaces) before changing schemas.
- Define exact threshold formulas by metric key, comparator, tier, and severity.

### Must NOT Have

- No production MCP server changes in `src/`.
- No `store_memory` API contract changes.
- No direct DB writes from benchmarks.
- No GitHub Actions / CI blocking gate changes in this plan.
- No public dataset import: no LongMemEval / LoCoMo / ConvoMem / MemBench ingestion.
- No Qwen3 default model change or multi-model benchmark matrix in this plan.
- No MemPalace architecture rewrite, ChromaDB migration, or external vector DB adoption.
- No stale `test_mcp.sh` or `query_stats.sh` validation flow.
- No hidden auto-fix behavior in benchmarks; diagnose and report only.

---

## Verification Strategy

> **ZERO HUMAN INTERVENTION** - ALL verification is agent-executed. No acceptance criterion may require manual inspection.

### Test Decision

- **Infrastructure exists**: YES
- **Automated tests**: Tests-after with benchmark self-tests and representative tier runs
- **Framework**: Python CLI self-tests plus existing benchmark commands
- **Agent-Executed QA**: Mandatory for every task

### QA Policy

Evidence must be saved under `.sisyphus/evidence/benchmark-v2/`.

Primary commands:

```bash
python3 -m evals.lib.metrics --self-test
python3 evals/memory_retrieval_benchmark.py --self-test
python3 evals/code_retrieval_benchmark.py --self-test
python3 evals/memory_retrieval_benchmark.py
python3 evals/code_retrieval_benchmark.py
```

---

## Execution Strategy

### Parallel Execution Waves

```text
Wave 1 (Contracts and scaffolding)
├── T0: Map legacy contracts and canonical sources
├── T1: Define V2 schemas, tier contract, determinism policy
├── T2: Add explicit baseline refresh and baseline version policy
├── T3: Define threshold policy semantics
└── T4: Add report contract extensions

Wave 2 (Dataset expansion)
├── T5: Expand memory retrieval fixtures and golden queries
├── T6: Expand code retrieval fixtures and golden queries
└── T7: Add label QA and fixture-drift checks

Wave 3 (Harness integration)
├── T8: Wire fixture tiers into memory benchmark
├── T9: Wire fixture tiers into code benchmark
└── T10: Integrate threshold/readiness/failure summaries into reports

Wave 4 (Docs and validation)
├── T11: Update evals README V2 runbook
└── T12: Full V2 self-test and representative benchmark bundle

Wave FINAL
├── F1: Plan compliance audit (oracle)
├── F2: Code quality review (unspecified-high)
├── F3: Real benchmark QA (unspecified-high)
└── F4: Scope fidelity check (deep)
```

### Dependency Matrix

| Task | Blocked By | Blocks |
|---|---|---|
| T0 | None | T1, T2, T3, T4, T6, T9, T11 |
| T1 | T0 | T5, T6, T7, T8, T9, T11 |
| T2 | T0 | T4, T10, T11 |
| T3 | T0 | T10, T11 |
| T4 | T1, T2 | T10, T12 |
| T5 | T1 | T7, T8, T12 |
| T6 | T1 | T7, T9, T12 |
| T7 | T5, T6 | T12 |
| T8 | T1, T5 | T10, T12 |
| T9 | T1, T6 | T10, T12 |
| T10 | T3, T4, T8, T9 | T11, T12 |
| T11 | T1, T2, T3, T10 | T12 |
| T12 | T7, T10, T11 | Final Verification |

### Agent Dispatch Summary

| Wave | Tasks | Recommended Agents |
|---|---|---|
| 1 | T0-T4 | T0 `deep`, T1 `deep`, T2 `unspecified-high`, T3 `deep`, T4 `unspecified-high` |
| 2 | T5-T7 | T5 `unspecified-high`, T6 `unspecified-high`, T7 `deep` |
| 3 | T8-T10 | T8 `unspecified-high`, T9 `unspecified-high`, T10 `deep` |
| 4 | T11-T12 | T11 `writing`, T12 `unspecified-high` |
| Final | F1-F4 | `oracle`, `unspecified-high`, `unspecified-high`, `deep` |

---

## TODOs

> Implementation + verification belong in the same task. Every task must capture evidence under `.sisyphus/evidence/benchmark-v2/`.

- [x] T0. **Map legacy contracts and canonical sources**

  **What to do**:
  - Create a compatibility mapping before any V2 schema work: legacy `version` field to V2 `schema_version`, existing memory golden files to tiered V2 query files, and current code benchmark baseline source to canonical V2 code golden files.
  - Decide and document the single canonical code benchmark query source that later tasks must wire to, noting that current code retrieval may still load from `.sisyphus/evidence/task-2-recall-code-baseline.json`.
  - Define the evidence namespace policy: new V2 evidence goes under `.sisyphus/evidence/benchmark-v2/`; old `.sisyphus/evidence/evals/` remains historical/read-only context.
  - Add a runtime budget table for `small`, `medium`, and `stress` with target minutes and skip/optional policy.

  **Must NOT do**:
  - Do not migrate files or change harness behavior in this task; this is contract mapping only.
  - Do not delete historical evidence or legacy baseline artifacts.

  **Recommended Agent Profile**:
  - **Category**: `deep` — this prevents schema/source ambiguity and downstream rework.
  - **Skills**: []
  - **Skills Evaluated but Omitted**: `git-master` — no commit operation; `mcp-builder` — no MCP server work.

  **Parallelization**:
  - **Can Run In Parallel**: NO
  - **Parallel Group**: Wave 1 prerequisite
  - **Blocks**: T1, T2, T3, T4, T6, T9, T11
  - **Blocked By**: None

  **References**:
  - `evals/code_retrieval_benchmark.py` — Current code query/baseline source behavior that must be canonicalized before T6/T9.
  - `.sisyphus/evidence/task-2-recall-code-baseline.json` — Legacy code benchmark query source mentioned by review.
  - `evals/golden/memory_retrieval_queries.json` — Existing memory golden query source.
  - `evals/README.md` — Current V1 documentation and old evidence namespace context.

  **Acceptance Criteria**:
  - [ ] Plan/benchmark docs or constants include a legacy-to-V2 mapping table.
  - [ ] Canonical V2 code query source is named explicitly.
  - [ ] Evidence namespace policy distinguishes historical `.sisyphus/evidence/evals/` from new `.sisyphus/evidence/benchmark-v2/`.
  - [ ] Runtime budget table defines target minutes and skip policy for small/medium/stress.

  **QA Scenarios**:
  ```text
  Scenario: Legacy compatibility mapping is explicit
    Tool: Bash
    Preconditions: T0 mapping is written
    Steps:
      1. Search benchmark-owned files/docs for `legacy`, `schema_version`, `version`, `canonical`, and `.sisyphus/evidence/benchmark-v2`.
      2. Save matching lines.
    Expected Result: Mapping clearly explains old-to-new fields, canonical code query source, and evidence namespace policy.
    Failure Indicators: Ambiguous code query source or missing old/new field mapping.
    Evidence: .sisyphus/evidence/benchmark-v2/task-0-compat-mapping.txt

  Scenario: No behavior change occurs in T0
    Tool: Bash
    Preconditions: T0 changes visible
    Steps:
      1. Run `git diff --name-only` and inspect paths.
      2. Confirm changed files are docs/constants only and not `src/**`.
    Expected Result: No production path changes and no harness behavior change unless only adding inert constants/docs.
    Failure Indicators: Server code, CI, dependency, or behavior-changing benchmark logic appears before contracts are defined.
    Evidence: .sisyphus/evidence/benchmark-v2/task-0-no-behavior-change.txt
  ```

  **Commit**: YES if user later asks to commit
  - Message: `test(evals): map benchmark v2 contracts`

- [x] T1. **Define V2 schemas, tier contract, and determinism policy**

  **What to do**:
  - Add a versioned Benchmark V2 contract for fixture tiers, report schema, baseline schema, deterministic sorting, metric tolerance, and runtime target fields.
  - Keep this contract in `evals/` benchmark-owned files only; prefer lightweight Python constants/helpers or markdown tables over new infrastructure.
  - Define `small`, `medium`, and `stress` tiers with expected purpose, rough size range, and whether they are required or optional for local runs.

  **Must NOT do**:
  - Do not change `src/`, MCP API contracts, dependency manifests, CI files, or model defaults.
  - Do not introduce Qwen3/multi-model execution.

  **Recommended Agent Profile**:
  - **Category**: `deep` — contract design affects all later benchmark tasks.
  - **Skills**: []
  - **Skills Evaluated but Omitted**: `mcp-builder` — not building an MCP server; `writing` — task includes docs, but the core is benchmark contract design.

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 1 with T2, T3, T4
  - **Blocks**: T5, T6, T7, T8, T9, T11
  - **Blocked By**: None

  **References**:
  - `evals/README.md:1-23` — Current V1 benchmark scope, deterministic fixture statement, and exclusions.
  - `evals/lib/metrics.py` — Existing metrics/report helper location to extend without adding dependencies.
  - `evals/memory_retrieval_benchmark.py` — Current fixture paths, diagnostics, and memory benchmark CLI behavior.
  - `evals/code_retrieval_benchmark.py` — Current code benchmark CLI behavior and report output pattern.

  **Acceptance Criteria**:
  - [ ] V2 schema/contract includes `schema_version`, `fixture_tier`, `baseline_version`, `threshold_policy`, deterministic ordering rule, and runtime target metadata.
  - [ ] `small`, `medium`, and `stress` tiers are defined in a benchmark-owned file or README section.
  - [ ] Existing V1 self-tests still pass.

  **QA Scenarios**:
  ```text
  Scenario: Contract is machine-readable or explicitly documented
    Tool: Bash
    Preconditions: Repository checkout after T1 changes
    Steps:
      1. Run `python3 -m evals.lib.metrics --self-test`.
      2. Search benchmark-owned files for `schema_version`, `fixture_tier`, `baseline_version`, and `threshold_policy`.
      3. Save command output and search output.
    Expected Result: Self-test exits 0 and all required contract fields are present outside `src/`.
    Failure Indicators: Missing contract fields, self-test failure, or changes under forbidden paths.
    Evidence: .sisyphus/evidence/benchmark-v2/task-1-contract-fields.txt

  Scenario: Forbidden production paths remain untouched
    Tool: Bash
    Preconditions: T1 changes staged or visible in working tree
    Steps:
      1. Run `git diff --name-only -- 'src/**' '.github/workflows/**' 'Cargo.toml' 'Cargo.lock' 'package.json' 'pyproject.toml' 'requirements.txt'`.
      2. Save output.
    Expected Result: Output is empty.
    Failure Indicators: Any forbidden path appears.
    Evidence: .sisyphus/evidence/benchmark-v2/task-1-forbidden-paths.txt
  ```

  **Commit**: YES if user later asks to commit
  - Message: `test(evals): define benchmark v2 contracts`

- [x] T2. **Add explicit baseline refresh and baseline version policy**

  **What to do**:
  - Design a baseline metadata policy for Benchmark V2: version, source command, fixture tier, timestamp, model name, commit hash when available, and intentional refresh reason.
  - Ensure accidental baseline overwrite is impossible or clearly rejected unless an explicit refresh flag/mode is used.
  - Extend baseline diff output so baseline mismatches are readable and machine-consumable.

  **Must NOT do**:
  - Do not make benchmark runs silently overwrite baseline artifacts.
  - Do not add CI enforcement or external storage.

  **Recommended Agent Profile**:
  - **Category**: `unspecified-high` — requires careful implementation planning across benchmark helpers and scripts.
  - **Skills**: []
  - **Skills Evaluated but Omitted**: `git-master` — no commit work inside task; `xlsx` — reports are JSON/Markdown, not spreadsheets.

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 1 with T1, T3, T4
  - **Blocks**: T4, T10, T11
  - **Blocked By**: None

  **References**:
  - `evals/lib/metrics.py` — Existing baseline diff/report helpers.
  - `.sisyphus/evidence/evals/` — Prior baseline artifacts and diff evidence pattern.
  - `evals/README.md:78-93` — Current V1 no-gates baseline policy and future V2 mention.

  **Acceptance Criteria**:
  - [ ] Baseline metadata schema is defined and included in generated V2 baseline/report artifacts.
  - [ ] Baseline refresh requires an explicit flag/mode or documented command; normal benchmark runs do not overwrite baseline artifacts.
  - [ ] Baseline diff includes clear changed/missing/improved/regressed fields.

  **QA Scenarios**:
  ```text
  Scenario: Normal run does not refresh baseline
    Tool: Bash
    Preconditions: Existing baseline artifact present
    Steps:
      1. Record checksum of the baseline artifact.
      2. Run the default memory benchmark command for the small tier.
      3. Record checksum again and compare.
    Expected Result: Baseline checksum is unchanged; run writes report/diff separately.
    Failure Indicators: Baseline file changes without explicit refresh mode.
    Evidence: .sisyphus/evidence/benchmark-v2/task-2-no-silent-refresh.txt

  Scenario: Explicit refresh records reason and metadata
    Tool: Bash
    Preconditions: Benchmark refresh flag/mode implemented
    Steps:
      1. Run refresh command with reason `benchmark-v2-contract-test` against a temp baseline path.
      2. Inspect resulting JSON for `baseline_version`, `fixture_tier`, `model`, `command`, and `refresh_reason`.
    Expected Result: Metadata is present and reason exactly matches `benchmark-v2-contract-test`.
    Failure Indicators: Missing metadata, missing reason, or refresh command mutates canonical baseline during temp test.
    Evidence: .sisyphus/evidence/benchmark-v2/task-2-explicit-refresh.txt
  ```

  **Commit**: YES if user later asks to commit
  - Message: `test(evals): add baseline refresh policy`

- [x] T3. **Define local threshold policy semantics**

  **What to do**:
  - Create an explicit threshold policy for local V2 interpretation: `pass`, `warn`, `blocker`, and `deferred` semantics.
  - Define which metrics participate in blocker vs warning decisions for each tier.
  - Define an exact formula matrix: metric key, tier, comparator (`>=`, `<=`, `==`), threshold value, severity (`warn`/`blocker`), and denominator rule.
  - Keep CI gates out of scope; thresholds are local report interpretation only.

  **Must NOT do**:
  - Do not add GitHub Actions or fail repository CI.
  - Do not silently change thresholds without a central diff-visible policy.

  **Recommended Agent Profile**:
  - **Category**: `deep` — threshold semantics can mislead future decisions if ambiguous.
  - **Skills**: []
  - **Skills Evaluated but Omitted**: `web-design-guidelines` — no UI/accessibility work.

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 1 with T1, T2, T4
  - **Blocks**: T10, T11
  - **Blocked By**: None

  **References**:
  - `evals/lib/metrics.py` — Existing blocker/readiness/failure summary logic.
  - `evals/README.md:78-93` — Current no-gate policy; V2 threshold work is future-facing.
  - `.sisyphus/plans/benchmark-optimization.md:67-113` — Guardrails against CI gates and over-scoping.

  **Acceptance Criteria**:
  - [ ] Threshold policy is centralized and diff-visible.
  - [ ] Policy distinguishes local `warn` from local `blocker` and explicitly says no CI gate is added.
  - [ ] Threshold matrix names exact metric keys and formulas for every tier with thresholds.
  - [ ] Reports can display threshold status without causing hidden side effects.

  **QA Scenarios**:
  ```text
  Scenario: Threshold policy appears in generated report
    Tool: Bash
    Preconditions: T3 policy and report integration available
    Steps:
      1. Run `python3 -m evals.lib.metrics --self-test`.
      2. Run a representative report-generation self-test or benchmark command.
      3. Inspect output JSON/Markdown for `threshold_status` and `threshold_policy`.
    Expected Result: Threshold fields are present and classify results without CI interaction.
    Failure Indicators: Missing threshold fields or any CI config change.
    Evidence: .sisyphus/evidence/benchmark-v2/task-3-threshold-report.txt

  Scenario: CI gates were not added
    Tool: Bash
    Preconditions: T3 changes visible
    Steps:
      1. Run `git diff --name-only -- '.github/workflows/**'`.
      2. Save output.
    Expected Result: Output is empty.
    Failure Indicators: Any workflow file appears.
    Evidence: .sisyphus/evidence/benchmark-v2/task-3-no-ci-gates.txt
  ```

  **Commit**: YES if user later asks to commit
  - Message: `test(evals): define benchmark thresholds`

- [x] T4. **Add report contract extensions**

  **What to do**:
  - Extend unified report output to include V2 contract fields from T1/T2/T3.
  - Ensure both JSON and Markdown reports include metric summary, baseline diff, readiness taxonomy, failure buckets, threshold status, tier, schema version, and baseline version.
  - Preserve V1 report readability and existing self-test coverage.

  **Must NOT do**:
  - Do not remove V1 diagnostics or rename fields without compatibility notes.
  - Do not add third-party report dependencies.

  **Recommended Agent Profile**:
  - **Category**: `unspecified-high` — practical report integration across helper and benchmark scripts.
  - **Skills**: []
  - **Skills Evaluated but Omitted**: `theme-factory` — no visual artifact styling required.

  **Parallelization**:
  - **Can Run In Parallel**: PARTIAL
  - **Parallel Group**: Wave 1, but final integration depends on T1/T2/T3 decisions
  - **Blocks**: T10, T12
  - **Blocked By**: T1, T2

  **References**:
  - `evals/lib/metrics.py` — Report generation and metric helpers.
  - `evals/memory_retrieval_benchmark.py` — Current memory report output and diagnostics.
  - `evals/code_retrieval_benchmark.py` — Current code report output and diagnostics.

  **Acceptance Criteria**:
  - [ ] JSON report contains V2 contract fields.
  - [ ] Markdown report contains human-readable V2 summary.
  - [ ] `python3 -m evals.lib.metrics --self-test` passes.

  **QA Scenarios**:
  ```text
  Scenario: JSON and Markdown reports contain V2 fields
    Tool: Bash
    Preconditions: T4 implementation complete
    Steps:
      1. Run memory benchmark with output paths under `.sisyphus/evidence/benchmark-v2/task-4/`.
      2. Inspect JSON for `schema_version`, `fixture_tier`, `baseline_version`, `threshold_status`, `failure_buckets`, and `readiness_summary`.
      3. Inspect Markdown for the same concepts in readable headings or table rows.
    Expected Result: Required fields appear in both outputs.
    Failure Indicators: Missing JSON fields, missing Markdown summary, or self-test failure.
    Evidence: .sisyphus/evidence/benchmark-v2/task-4-report-contract.txt

  Scenario: Existing self-test compatibility remains intact
    Tool: Bash
    Preconditions: T4 implementation complete
    Steps:
      1. Run `python3 -m evals.lib.metrics --self-test`.
      2. Save output.
    Expected Result: Exit code 0.
    Failure Indicators: Any self-test failure or traceback.
    Evidence: .sisyphus/evidence/benchmark-v2/task-4-self-test.txt
  ```

  **Commit**: YES if user later asks to commit
  - Message: `test(evals): extend benchmark reports`

- [x] T5. **Expand memory retrieval fixtures and golden queries**

  **What to do**:
  - Add medium-tier memory fixtures and golden queries covering long-memory recall, namespace boundaries, temporal/get_valid boundaries, negative no-match queries, partial readiness, ID mismatch/alias cases, and record-shaped IDs.
  - Add stress-tier definitions or fixture manifests without requiring heavy execution by default.
  - Each golden query must include a label rationale: expected IDs, scenario category, and why the label is correct.

  **Must NOT do**:
  - Do not import public memory benchmark datasets.
  - Do not use production data or personally identifying data.
  - Do not change production memory APIs.

  **Recommended Agent Profile**:
  - **Category**: `unspecified-high` — dataset expansion with careful labels and edge-case coverage.
  - **Skills**: []
  - **Skills Evaluated but Omitted**: `xlsx` — fixtures remain JSON/JSONL, not spreadsheets.

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 2 with T6
  - **Blocks**: T7, T8, T12
  - **Blocked By**: T1

  **References**:
  - `evals/fixtures/memory_corpus_mini_long_memory.json` — Current synthetic mini fixture style and safe local data pattern.
  - `evals/golden/memory_retrieval_queries_mini_long_memory.json` — Current mini query structure.
  - `evals/golden/memory_retrieval_queries.json` — Current canonical memory golden query style.
  - `evals/memory_retrieval_benchmark.py` — Current fixture loading and ID remapping behavior.

  **Acceptance Criteria**:
  - [ ] Medium memory fixture and golden query files exist.
  - [ ] Scenario coverage includes long-memory, namespace, temporal, negative, readiness, ID mismatch/alias, and record-shaped ID cases.
  - [ ] Every new query includes label rationale metadata.
  - [ ] Memory benchmark self-test validates fixture/query loading.

  **QA Scenarios**:
  ```text
  Scenario: Medium memory fixture loads and validates
    Tool: Bash
    Preconditions: T5 fixture/query files exist
    Steps:
      1. Run `python3 evals/memory_retrieval_benchmark.py --self-test`.
      2. Save output and confirm it reports medium fixture/query counts.
    Expected Result: Exit code 0 and medium fixture/query counts are non-zero.
    Failure Indicators: JSON parse error, missing counts, or self-test failure.
    Evidence: .sisyphus/evidence/benchmark-v2/task-5-memory-medium-load.txt

  Scenario: Negative queries do not create false blockers
    Tool: Bash
    Preconditions: Medium memory golden queries include negative no-match cases
    Steps:
      1. Run medium-tier memory benchmark or dry-run validation.
      2. Inspect report for negative query classifications.
    Expected Result: Negative queries are classified distinctly from retrieval failures and do not inflate blocker count incorrectly.
    Failure Indicators: Negative no-match query counted as ordinary expected-ID miss.
    Evidence: .sisyphus/evidence/benchmark-v2/task-5-negative-query.txt
  ```

  **Commit**: YES if user later asks to commit
  - Message: `test(evals): expand memory benchmark fixtures`

- [x] T6. **Expand code retrieval fixtures and golden queries**

  **What to do**:
  - Add medium-tier code retrieval scenarios for symbol definitions, caller/callee relationships, file-path queries, similar-function interference, deleted/renamed symbol expectations, and hybrid-vs-vector disagreement cases.
  - First canonicalize code retrieval golden-query ownership: migrate away from legacy `.sisyphus/evidence/task-2-recall-code-baseline.json` as the authoritative query source, or document a compatibility bridge that reads it only as legacy input.
  - Add explicit ground-truth rationale for each query so future codebase drift is visible.
  - Define stress-tier code fixture/query manifest without making it default.

  **Must NOT do**:
  - Do not rewrite the project source tree to satisfy benchmark labels.
  - Do not add a multi-model matrix or Qwen3 code retrieval comparison.

  **Recommended Agent Profile**:
  - **Category**: `unspecified-high` — dataset work plus code-retrieval label quality.
  - **Skills**: []
  - **Skills Evaluated but Omitted**: `frontend-design` — no UI work.

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 2 with T5
  - **Blocks**: T7, T9, T12
  - **Blocked By**: T1

  **References**:
  - `evals/code_retrieval_benchmark.py` — Current code retrieval benchmark and self-test structure.
  - `.sisyphus/evidence/benchmark-optimization/code-ranking-follow-up.md` — Prior threshold exception and code retrieval follow-up context.
  - `evals/README.md` — Current code benchmark command/runbook.

  **Acceptance Criteria**:
  - [ ] A canonical V2 code golden query source exists under `evals/`.
  - [ ] Legacy `.sisyphus/evidence/task-2-recall-code-baseline.json` is not the only authoritative source for V2 code queries.
  - [ ] Medium code fixture/query set exists with explicit rationale labels.
  - [ ] Scenarios cover symbol definition, caller/callee, path, similar-function interference, deleted/renamed expectation, and hybrid/vector disagreement.
  - [ ] Code benchmark self-test validates fixture/query loading.

  **QA Scenarios**:
  ```text
  Scenario: Medium code queries load and validate
    Tool: Bash
    Preconditions: T6 fixture/query files exist
    Steps:
      1. Run `python3 evals/code_retrieval_benchmark.py --self-test`.
      2. Save output and confirm medium query count is reported.
    Expected Result: Exit code 0 and medium query count is non-zero.
    Failure Indicators: Missing file, parse error, or self-test failure.
    Evidence: .sisyphus/evidence/benchmark-v2/task-6-code-medium-load.txt

  Scenario: Drift-prone labels are explicit
    Tool: Bash
    Preconditions: Medium code golden queries exist
    Steps:
      1. Inspect medium code golden file for rationale fields on deleted/renamed and caller/callee scenarios.
      2. Save extracted query IDs and rationale text.
    Expected Result: Every drift-prone query has non-empty rationale and expected behavior.
    Failure Indicators: Missing rationale or ambiguous expected IDs.
    Evidence: .sisyphus/evidence/benchmark-v2/task-6-label-rationale.txt
  ```

  **Commit**: YES if user later asks to commit
  - Message: `test(evals): expand code benchmark fixtures`

- [x] T7. **Add label QA and fixture-drift checks**

  **What to do**:
  - Add validation that every new query has category, expected IDs or explicit no-match expectation, rationale, and tier.
  - Add fixture-drift checks for duplicate IDs, alias collisions, namespace leakage, temporal boundary ambiguity, and code label paths/symbols that no longer exist when they are expected to exist.
  - Make validation produce clear error messages and evidence-friendly output.

  **Must NOT do**:
  - Do not auto-fix fixtures silently.
  - Do not delete or rewrite existing labels without explicit rationale in the diff.

  **Recommended Agent Profile**:
  - **Category**: `deep` — QA rules encode future benchmark trustworthiness.
  - **Skills**: []
  - **Skills Evaluated but Omitted**: `receiving-code-review` — not responding to review feedback.

  **Parallelization**:
  - **Can Run In Parallel**: NO
  - **Parallel Group**: Wave 2 after T5 and T6
  - **Blocks**: T12
  - **Blocked By**: T5, T6

  **References**:
  - `evals/memory_retrieval_benchmark.py` — Existing fixture validation and ID normalization.
  - `evals/code_retrieval_benchmark.py` — Existing code query loading/self-test validation.
  - `evals/lib/metrics.py` — Existing failure taxonomy helpers.

  **Acceptance Criteria**:
  - [ ] Validation rejects missing rationale/category/tier/expected behavior.
  - [ ] Validation detects duplicate memory IDs and alias collisions.
  - [ ] Validation reports code label drift for expected paths/symbols when checkable.
  - [ ] Validation errors are diagnostic and do not silently mutate files.

  **QA Scenarios**:
  ```text
  Scenario: Valid fixtures pass label QA
    Tool: Bash
    Preconditions: T5 and T6 fixtures exist
    Steps:
      1. Run memory and code self-tests.
      2. Save validation output.
    Expected Result: Both self-tests exit 0 and label QA reports pass counts.
    Failure Indicators: Missing label metadata or false-positive validation failure.
    Evidence: .sisyphus/evidence/benchmark-v2/task-7-valid-label-qa.txt

  Scenario: Invalid temp fixture is rejected without mutation
    Tool: Bash
    Preconditions: Label QA helper can validate custom/temp fixture paths
    Steps:
      1. Copy a small fixture/query file to a temp path under `.sisyphus/evidence/benchmark-v2/task-7/`.
      2. Remove one rationale/category field from the temp copy.
      3. Run validation against the temp copy.
      4. Verify canonical fixture files are unchanged.
    Expected Result: Validation exits non-zero or reports an explicit error for the temp copy only; canonical fixtures remain unchanged.
    Failure Indicators: Invalid fixture passes, canonical file mutates, or error message is vague.
    Evidence: .sisyphus/evidence/benchmark-v2/task-7-invalid-label-qa.txt
  ```

  **Commit**: YES if user later asks to commit
  - Message: `test(evals): add benchmark label QA`

- [x] T8. **Wire fixture tiers into memory benchmark**

  **What to do**:
  - Add CLI support for memory benchmark fixture tiers, with `small` as the safe default and `medium`/`stress` selectable explicitly.
  - Ensure tier selection affects fixture paths, golden query paths, report metadata, runtime target metadata, and evidence naming.
  - Preserve V1 default command behavior as much as possible; if default behavior changes, document it in T11.

  **Must NOT do**:
  - Do not make stress tier the default.
  - Do not change default model from `e5_small`.
  - Do not mutate baseline artifacts during normal tier runs.

  **Recommended Agent Profile**:
  - **Category**: `unspecified-high` — harness integration with CLI/report behavior.
  - **Skills**: []
  - **Skills Evaluated but Omitted**: `mcp-builder` — no MCP protocol/tool implementation changes.

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 3 with T9
  - **Blocks**: T10, T12
  - **Blocked By**: T1, T5

  **References**:
  - `evals/memory_retrieval_benchmark.py` — Current CLI, fixture loading, report generation, ID remapping.
  - `evals/fixtures/memory_corpus_mini_long_memory.json` — Current small/mini fixture path pattern.
  - `evals/golden/memory_retrieval_queries_mini_long_memory.json` — Current small/mini golden query path pattern.

  **Acceptance Criteria**:
  - [ ] Memory benchmark accepts explicit tier selection.
  - [ ] Default remains safe and local-friendly.
  - [ ] Report metadata records selected fixture tier.
  - [ ] Memory self-test covers tier path resolution.

  **QA Scenarios**:
  ```text
  Scenario: Small-tier memory benchmark runs successfully
    Tool: Bash
    Preconditions: T8 implementation complete
    Steps:
      1. Run `python3 evals/memory_retrieval_benchmark.py --tier small --output-json .sisyphus/evidence/benchmark-v2/task-8/memory-small.json --output-md .sisyphus/evidence/benchmark-v2/task-8/memory-small.md`.
      2. Inspect JSON for `fixture_tier: small` and `blocker_count`.
    Expected Result: Command exits 0, JSON says `small`, and blocker count is 0 or threshold policy explains non-blocking warnings.
    Failure Indicators: Missing tier metadata, command failure, or silent baseline mutation.
    Evidence: .sisyphus/evidence/benchmark-v2/task-8-memory-small.txt

  Scenario: Invalid tier fails clearly
    Tool: Bash
    Preconditions: T8 CLI supports tier argument
    Steps:
      1. Run `python3 evals/memory_retrieval_benchmark.py --tier does-not-exist`.
      2. Capture exit code and stderr/stdout.
    Expected Result: Command fails with clear invalid-tier message listing valid tiers.
    Failure Indicators: Traceback without helpful message or fallback to default tier.
    Evidence: .sisyphus/evidence/benchmark-v2/task-8-invalid-tier.txt
  ```

  **Commit**: YES if user later asks to commit
  - Message: `test(evals): add memory benchmark tiers`

- [x] T9. **Wire fixture tiers into code benchmark**

  **What to do**:
  - Add CLI support for code benchmark fixture/query tiers, with `small` as default and `medium`/`stress` explicit.
  - Ensure report metadata captures tier, baseline version, model, retrieval mode if available, and label QA status.
  - Preserve existing self-test and current local command behavior where possible.

  **Must NOT do**:
  - Do not alter production source to make labels pass.
  - Do not add Qwen3 or multi-model execution paths.

  **Recommended Agent Profile**:
  - **Category**: `unspecified-high` — code benchmark CLI/report integration.
  - **Skills**: []
  - **Skills Evaluated but Omitted**: `xcodebuildmcp-cli` — not an Apple/Xcode project task.

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 3 with T8
  - **Blocks**: T10, T12
  - **Blocked By**: T1, T6

  **References**:
  - `evals/code_retrieval_benchmark.py` — Current CLI, fixture copy behavior, MCP recall_code usage, self-test.
  - `evals/README.md` — Current code benchmark run command and stale script warning.
  - `.sisyphus/evidence/benchmark-optimization/code-ranking-follow-up.md` — Prior threshold exception context to keep out of hidden pass/fail behavior.

  **Acceptance Criteria**:
  - [ ] Code benchmark accepts explicit tier selection.
  - [ ] Code benchmark reads V2 tiered golden queries from the canonical source defined in T0/T6, not only from legacy evidence baseline.
  - [ ] Report metadata records selected fixture tier.
  - [ ] Self-test validates tier path resolution.
  - [ ] Stress tier is never default.

  **QA Scenarios**:
  ```text
  Scenario: Small-tier code benchmark runs successfully
    Tool: Bash
    Preconditions: T9 implementation complete
    Steps:
      1. Run `python3 evals/code_retrieval_benchmark.py --tier small --output-json .sisyphus/evidence/benchmark-v2/task-9/code-small.json --output-md .sisyphus/evidence/benchmark-v2/task-9/code-small.md`.
      2. Inspect JSON for `fixture_tier: small` and metric summary fields.
    Expected Result: Command exits 0 and report contains tier plus metrics.
    Failure Indicators: Missing tier metadata, command failure, or model-default change.
    Evidence: .sisyphus/evidence/benchmark-v2/task-9-code-small.txt

  Scenario: Invalid code tier fails clearly
    Tool: Bash
    Preconditions: T9 CLI supports tier argument
    Steps:
      1. Run `python3 evals/code_retrieval_benchmark.py --tier does-not-exist`.
      2. Capture exit code and stderr/stdout.
    Expected Result: Command fails with clear invalid-tier message listing valid tiers.
    Failure Indicators: Traceback without helpful message or fallback to default tier.
    Evidence: .sisyphus/evidence/benchmark-v2/task-9-invalid-tier.txt
  ```

  **Commit**: YES if user later asks to commit
  - Message: `test(evals): add code benchmark tiers`

- [x] T10. **Integrate threshold, readiness, and failure summaries into reports**

  **What to do**:
  - Combine T3/T4/T8/T9 outputs so both benchmark reports consistently show threshold result, readiness taxonomy, failure buckets, baseline diff, and tier metadata.
  - Ensure empty-result, low-confidence, expected-no-match, and true miss are distinguishable in report summaries.
  - Add summary tests or self-test assertions for representative pass/warn/blocker cases.

  **Must NOT do**:
  - Do not let report generation mutate input fixtures or baselines.
  - Do not hide blocker cases behind generic “failed” buckets.

  **Recommended Agent Profile**:
  - **Category**: `deep` — cross-harness consistency and failure semantics need careful reasoning.
  - **Skills**: []
  - **Skills Evaluated but Omitted**: `webapp-testing` — no browser UI.

  **Parallelization**:
  - **Can Run In Parallel**: NO
  - **Parallel Group**: Wave 3 integration after T8/T9
  - **Blocks**: T11, T12
  - **Blocked By**: T3, T4, T8, T9

  **References**:
  - `evals/lib/metrics.py` — Shared metric/report/failure helper layer.
  - `evals/memory_retrieval_benchmark.py` — Memory readiness and failure_type diagnostics.
  - `evals/code_retrieval_benchmark.py` — Code retrieval report path.

  **Acceptance Criteria**:
  - [ ] Memory and code reports use the same top-level V2 report contract fields.
  - [ ] Reports distinguish expected no-match, empty result, low confidence, and true miss where applicable.
  - [ ] Self-tests cover at least one pass, warning, and blocker policy case.

  **QA Scenarios**:
  ```text
  Scenario: Memory and code reports share V2 summary fields
    Tool: Bash
    Preconditions: T10 integration complete
    Steps:
      1. Run small-tier memory and code benchmarks into `.sisyphus/evidence/benchmark-v2/task-10/`.
      2. Parse both JSON outputs for `threshold_status`, `readiness_summary`, `failure_buckets`, `baseline_diff`, and `fixture_tier`.
    Expected Result: Both reports contain all fields with compatible types.
    Failure Indicators: Field missing from one harness or inconsistent field type.
    Evidence: .sisyphus/evidence/benchmark-v2/task-10-shared-fields.txt

  Scenario: Failure classes are not collapsed
    Tool: Bash
    Preconditions: T10 report summaries implemented
    Steps:
      1. Run self-test or fixture validation that includes expected no-match and true miss examples.
      2. Inspect failure buckets.
    Expected Result: Expected no-match and true miss appear as distinct categories.
    Failure Indicators: All failures collapse into `miss` or `unknown`.
    Evidence: .sisyphus/evidence/benchmark-v2/task-10-failure-classes.txt
  ```

  **Commit**: YES if user later asks to commit
  - Message: `test(evals): unify benchmark summaries`

- [x] T11. **Update evals README V2 runbook**

  **What to do**:
  - Update `evals/README.md` with Benchmark V2 purpose, non-goals, tier definitions, run commands, baseline refresh workflow, threshold interpretation, report fields, label QA, and Qwen3/multi-model deferral.
  - Keep stale scripts marked stale/avoid; do not reintroduce them as validation paths.
  - Explain that V2 is for local regression visibility and dataset quality, not production server behavior changes.

  **Must NOT do**:
  - Do not update root `README.md` unless behavior outside `evals/` changes.
  - Do not claim Qwen3 quality conclusions.
  - Do not document CI gates as existing.

  **Recommended Agent Profile**:
  - **Category**: `writing` — operator-facing documentation/runbook work.
  - **Skills**: []
  - **Skills Evaluated but Omitted**: `doc-coauthoring` — this is a focused README update, not a full coauthored doc workflow.

  **Parallelization**:
  - **Can Run In Parallel**: NO
  - **Parallel Group**: Wave 4
  - **Blocks**: T12
  - **Blocked By**: T1, T2, T3, T10

  **References**:
  - `evals/README.md` — Existing benchmark documentation and stale script warnings.
  - This plan's `Context` and `Work Objectives` sections — User-confirmed direction: stabilize benchmark system + expand datasets/scenarios, with Qwen3 deferred.
  - `.sisyphus/plans/benchmark-optimization.md` — Prior guardrails and V1 verification commands.

  **Acceptance Criteria**:
  - [ ] README documents small/medium/stress tiers and commands.
  - [ ] README documents baseline refresh as explicit/intentional.
  - [ ] README documents threshold status as local interpretation, not CI gate.
  - [ ] README states Qwen3/multi-model comparison is deferred until dataset/baseline quality is stable.
  - [ ] Stale scripts remain warning-only/avoid-only.

  **QA Scenarios**:
  ```text
  Scenario: README contains V2 runbook essentials
    Tool: Bash
    Preconditions: T11 documentation complete
    Steps:
      1. Search `evals/README.md` for `Benchmark V2`, `small`, `medium`, `stress`, `baseline refresh`, `threshold`, and `Qwen3`.
      2. Save matching lines.
    Expected Result: All terms appear in accurate V2 sections with no CI-gate claim.
    Failure Indicators: Missing runbook section or misleading Qwen3/CI language.
    Evidence: .sisyphus/evidence/benchmark-v2/task-11-readme-v2.txt

  Scenario: Stale scripts are not revived
    Tool: Bash
    Preconditions: T11 documentation complete
    Steps:
      1. Search `evals/README.md` for `test_mcp.sh` and `query_stats.sh`.
      2. Confirm they appear only in stale/avoid/deprecated context.
    Expected Result: Both scripts remain marked stale/avoid and are not listed as validation commands.
    Failure Indicators: Stale scripts appear in main command path or verification commands.
    Evidence: .sisyphus/evidence/benchmark-v2/task-11-stale-scripts.txt
  ```

  **Commit**: YES if user later asks to commit
  - Message: `docs(evals): document benchmark v2 runbook`

- [x] T12. **Run full V2 self-test and representative benchmark bundle**

  **What to do**:
  - Run all Benchmark V2 self-tests and representative small-tier memory/code benchmark commands.
  - Run medium-tier validation and either execute medium benchmarks or document runtime-aware skip if execution is intentionally not default.
  - Run an intentional baseline refresh dry-run against a temporary output path to prove default no-refresh and explicit-refresh paths both work.
  - Capture final evidence bundle under `.sisyphus/evidence/benchmark-v2/final-bundle/` with commands, outputs, report paths, and summary.
  - Confirm forbidden scopes remain untouched.

  **Must NOT do**:
  - Do not commit changes unless user explicitly asks.
  - Do not mark stress tier as required if runtime budget says optional/manual.
  - Do not treat manual human inspection as acceptance evidence.

  **Recommended Agent Profile**:
  - **Category**: `unspecified-high` — execution-focused validation across benchmark scripts and docs.
  - **Skills**: []
  - **Skills Evaluated but Omitted**: `git-master` — no commit requested; `playwright` — no browser testing.

  **Parallelization**:
  - **Can Run In Parallel**: NO
  - **Parallel Group**: Wave 4 final integration task
  - **Blocks**: Final Verification
  - **Blocked By**: T7, T10, T11

  **References**:
  - `evals/README.md` — Commands and interpretation runbook.
  - `evals/lib/metrics.py` — Metrics self-test.
  - `evals/memory_retrieval_benchmark.py` — Memory benchmark self-test and tier runs.
  - `evals/code_retrieval_benchmark.py` — Code benchmark self-test and tier runs.

  **Acceptance Criteria**:
  - [ ] Metrics, memory, and code self-tests pass.
  - [ ] Small-tier memory and code reports are generated.
  - [ ] Medium-tier validation passes; if medium run is skipped, skip reason is runtime-budget based and documented in evidence.
  - [ ] Intentional baseline refresh dry-run artifact exists and does not mutate canonical baselines.
  - [ ] Forbidden tracked path diff is empty for `src/**`, `.github/workflows/**`, root README, and dependency manifests.
  - [ ] Evidence bundle includes command transcript and report paths.

  **QA Scenarios**:
  ```text
  Scenario: Full self-test bundle passes
    Tool: Bash
    Preconditions: T1-T11 complete
    Steps:
      1. Run `python3 -m evals.lib.metrics --self-test`.
      2. Run `python3 evals/memory_retrieval_benchmark.py --self-test`.
      3. Run `python3 evals/code_retrieval_benchmark.py --self-test`.
      4. Save full transcript.
    Expected Result: All commands exit 0.
    Failure Indicators: Any non-zero exit or traceback.
    Evidence: .sisyphus/evidence/benchmark-v2/final-bundle/self-tests.txt

  Scenario: Representative small-tier reports are produced
    Tool: Bash
    Preconditions: T1-T11 complete
    Steps:
      1. Run small-tier memory benchmark with JSON/Markdown outputs under final-bundle.
      2. Run small-tier code benchmark with JSON/Markdown outputs under final-bundle.
      3. Inspect both JSON files for V2 fields and both Markdown files for readable summaries.
    Expected Result: Four report files exist and include V2 schema/tier/threshold/baseline fields.
    Failure Indicators: Missing report file or missing V2 fields.
    Evidence: .sisyphus/evidence/benchmark-v2/final-bundle/small-tier-reports.txt

  Scenario: Forbidden scope remains clean
    Tool: Bash
    Preconditions: T1-T11 complete
    Steps:
      1. Run `git diff --name-only -- 'src/**' '.github/workflows/**' 'README.md' 'Cargo.toml' 'Cargo.lock' 'package.json' 'pyproject.toml' 'requirements.txt'`.
      2. Save output.
    Expected Result: Output is empty.
    Failure Indicators: Any forbidden path appears.
    Evidence: .sisyphus/evidence/benchmark-v2/final-bundle/forbidden-scope.txt
  ```

  **Commit**: YES if user later asks to commit
  - Message: `test(evals): validate benchmark v2 bundle`

---

## Final Verification Wave

> 4 review agents run in PARALLEL. ALL must APPROVE. Present consolidated results to user and get explicit okay before completing.

- [x] F1. **Plan Compliance Audit** — `oracle`
  Verify all Must Have and Must NOT Have items, evidence files, deliverables, and references. Output: `Must Have [N/N] | Must NOT Have [N/N] | Tasks [N/N] | VERDICT`.

- [x] F2. **Code Quality Review** — `unspecified-high`
  Run Python compile/self-tests, inspect changed files for overbroad changes, hidden auto-fix behavior, stale scripts, and production server changes. Output: `Build [PASS/FAIL] | Tests [N/N] | Files [N clean/N issues] | VERDICT`.

- [x] F3. **Real Benchmark QA** — `unspecified-high`
  Execute every QA scenario from every task, capture evidence, run representative benchmark tiers, and verify generated JSON/Markdown outputs. Output: `Scenarios [N/N pass] | Reports [N/N] | VERDICT`.

- [x] F4. **Scope Fidelity Check** — `deep`
  Compare actual diff against this plan. Reject server changes, CI gates, public dataset imports, Qwen3 default/model matrix, MemPalace rewrite, or stale script validation flow. Output: `Tasks [N/N compliant] | Scope Creep [CLEAN/N issues] | VERDICT`.

---

## Commit Strategy

- Group commits by benchmark concern: contracts/policy, memory fixtures, code fixtures, harness integration, docs/validation.
- Do not commit unless the user explicitly asks.

---

## Success Criteria

### Verification Commands

```bash
python3 -m evals.lib.metrics --self-test
python3 evals/memory_retrieval_benchmark.py --self-test
python3 evals/code_retrieval_benchmark.py --self-test
python3 evals/memory_retrieval_benchmark.py
python3 evals/code_retrieval_benchmark.py
```

### Final Checklist

- [ ] All Must Have items present.
- [ ] All Must NOT Have items absent.
- [ ] All self-tests pass.
- [ ] Representative tier runs produce V2 reports.
- [ ] Documentation explains baseline refresh, thresholds, fixture tiers, and Qwen3 deferral.
