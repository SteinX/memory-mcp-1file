from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any, Iterable

HERE = Path(__file__).resolve().parent
FIXTURE_PATH = HERE / "fixtures" / "bootstrap_memory_corpus.json"
GOLDEN_PATH = HERE / "golden" / "bootstrap_queries.json"

VALID_PREFIXES = (
    "PROJECT:",
    "EPIC:",
    "TASK:",
    "RESEARCH:",
    "DECISION:",
    "CONTEXT:",
    "USER:",
)
REQUIRED_BOOTSTRAP_SECTIONS = {
    "active_tasks",
    "stable_context",
    "recovery",
    "project",
    "memory_health",
    "selection_summary",
    "contract",
    "summary",
}


def load_json(path: Path) -> Any:
    with path.open("r", encoding="utf-8") as handle:
        return json.load(handle)


def require(condition: bool, message: str) -> None:
    if not condition:
        raise AssertionError(message)


def validate_fixture(memories: list[dict[str, Any]]) -> dict[str, Any]:
    require(memories, "bootstrap fixture must not be empty")
    prefixes = set()
    for memory in memories:
        content = str(memory.get("content", ""))
        prefix = next((prefix for prefix in VALID_PREFIXES if content.startswith(prefix)), None)
        require(prefix is not None, f"memory {memory.get('id')} has no legal prefix")
        prefixes.add(prefix)
        require(memory.get("memory_type") in {"episodic", "semantic", "procedural"}, "invalid memory_type")
        require(memory.get("namespace") == "bootstrap-eval", "fixture namespace must be bootstrap-eval")
    require("TASK:" in prefixes, "fixture must include active TASK")
    require("DECISION:" in prefixes, "fixture must include DECISION")
    require("USER:" in prefixes, "fixture must include USER")
    require("RESEARCH:" in prefixes, "fixture must include RESEARCH")
    return {"memory_count": len(memories), "prefixes": sorted(prefixes)}


def validate_golden(queries: list[dict[str, Any]]) -> dict[str, Any]:
    require(queries, "bootstrap golden queries must not be empty")
    tools = {query.get("tool") for query in queries}
    require("memory_bootstrap" in tools, "golden queries must cover memory_bootstrap")
    require("memory_search_trace" in tools, "golden queries must cover memory_search_trace")
    for query in queries:
        require(query.get("id"), "query id required")
        require(query.get("arguments"), f"{query.get('id')} arguments required")
        require(query.get("expected_sections"), f"{query.get('id')} expected_sections required")
        rationale = query.get("label_rationale") or {}
        require(rationale.get("rationale"), f"{query.get('id')} label rationale required")
        require(rationale.get("expected_behavior"), f"{query.get('id')} expected behavior required")
    return {"query_count": len(queries), "tools": sorted(str(tool) for tool in tools)}


def run_self_test() -> int:
    fixture_summary = validate_fixture(load_json(FIXTURE_PATH))
    golden_summary = validate_golden(load_json(GOLDEN_PATH))
    print(
        json.dumps(
            {
                "status": "ok",
                "fixture": fixture_summary,
                "golden": golden_summary,
                "required_response_sections": sorted(REQUIRED_BOOTSTRAP_SECTIONS),
            },
            sort_keys=True,
        )
    )
    return 0


def parse_args(argv: Iterable[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Bootstrap/audit/trace eval fixture self-test.")
    parser.add_argument("--self-test", action="store_true", help="Validate fixture and golden query schema.")
    return parser.parse_args(list(argv) if argv is not None else None)


def main(argv: Iterable[str] | None = None) -> int:
    args = parse_args(argv)
    if args.self_test:
        return run_self_test()
    return run_self_test()


if __name__ == "__main__":
    raise SystemExit(main())
