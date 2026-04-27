# Code Retrieval Benchmark Migration Map

## Goal
Map `.sisyphus/evidence/graphify-next-actions/task-7-run-release-benchmark.py` into a future `evals/code_retrieval_benchmark.py` while keeping benchmark helpers reusable under `evals/lib/`.

## Source -> Target
- Source benchmark path: `.sisyphus/evidence/graphify-next-actions/task-7-run-release-benchmark.py`
- Target benchmark script: `evals/code_retrieval_benchmark.py`
- Shared helper area: `evals/lib/`

## Shared components to extract
These should move into `evals/lib/` and be reused by both memory and code benchmarks:
- stdio MCP client / command selection
- isolated `DATA_DIR`
- evidence JSON / Markdown helpers
- metrics helpers
- stderr / progress handling

## Code-specific components that stay in the code benchmark
These are specific to code retrieval and should remain in `evals/code_retrieval_benchmark.py` or nearby code-benchmark fixtures:
- fixture project creation / copying
- `index_project`
- `project_info`
- `recall_code`
- `search_symbols`
- `symbol_graph`
- task-2 recall-code baseline inputs

## Scope separation
### Memory benchmark tools
These belong to the memory benchmark only:
- `recall`
- `search_memory`
- `get_valid`
- `store_memory`
- `knowledge_graph`

### Code benchmark tools
These belong to the code benchmark only:
- `recall_code`
- `search_symbols`
- `symbol_graph`
- `index_project`
- `project_info`

## Migration shape
1. Preserve the existing task-7 benchmark behavior as the reference for process lifecycle and evidence writing.
2. Extract the generic process / reporting pieces into `evals/lib/`.
3. Keep project fixture setup and code tool calls in the code-specific script.
4. Reuse the same evidence conventions and isolated data directory handling.
5. Keep memory benchmark tool scope separate so the code benchmark does not inherit memory-only calls.

## Non-goals
- Do not rewrite the existing source benchmark script.
- Do not implement the future target script here.
- Do not add benchmark execution logic in this document.
