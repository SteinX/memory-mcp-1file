from __future__ import annotations

import argparse
import json
import shutil
import sys
import tempfile
import time
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Iterable, Sequence

PROJECT_ROOT = Path(__file__).resolve().parents[1]
if str(PROJECT_ROOT) not in sys.path:
    sys.path.insert(0, str(PROJECT_ROOT))

from evals.lib.mcp_client import McpClient, build_env, resolve_mcp_command
from evals.lib.metrics import aggregate_metrics, classify_reason_code, classify_reason_codes, compute_query_metrics, write_json_report, write_markdown_report


ROOT = PROJECT_ROOT
CODE_GOLDEN_V2_JSON = ROOT / "evals" / "golden" / "code_retrieval_queries_v2.json"
LEGACY_BASELINE_JSON = ROOT / ".sisyphus" / "evidence" / "task-2-recall-code-baseline.json"
EVIDENCE_DIR = ROOT / ".sisyphus" / "evidence" / "evals"
V2_EVIDENCE_DIR = ROOT / ".sisyphus" / "evidence" / "benchmark-v2"
OUTPUT_JSON = EVIDENCE_DIR / "code-retrieval-baseline.json"
OUTPUT_MD = EVIDENCE_DIR / "code-retrieval-baseline.md"
BENCHMARK_NAME = "code_retrieval_baseline"
V2_SCHEMA_VERSION = "2.0"
V2_DEFAULT_FIXTURE_TIER = "small"
V2_FIXTURE_TIERS = ("small", "medium", "stress")
V2_DEFAULT_BASELINE_VERSION = "v2-initial"
V2_THRESHOLD_POLICY = "local-v2-threshold-policy"
V2_DETERMINISM_POLICY = "stable_fixture_order+stable_tie_break+stable_report_order+tolerance_1e-9_1e-6"
V2_LABEL_QA_SENTINEL_QUERY_TYPES = {
    "negative_no_match",
    "deleted_symbol_expectation",
    "renamed_symbol_expectation",
}
REQUIRED_SOURCE_PATHS = (
    "src/graph/rrf.rs",
    "src/graph/ppr.rs",
    "src/server/logic/code/search.rs",
    "src/codebase/indexer.rs",
    "src/server/handler.rs",
)


def _tool_payload(response: dict[str, Any]) -> dict[str, Any]:
    result = response.get("result", {}) if isinstance(response, dict) else {}
    content = result.get("content", []) if isinstance(result, dict) else []
    if not content:
        return result if isinstance(result, dict) else {}
    first = content[0] if isinstance(content, list) and content else {}
    if not isinstance(first, dict):
        return result if isinstance(result, dict) else {}
    text = first.get("text")
    if not isinstance(text, str):
        return result if isinstance(result, dict) else {}
    try:
        parsed = json.loads(text)
    except json.JSONDecodeError:
        return result if isinstance(result, dict) else {}
    if isinstance(parsed, dict):
        parsed["_jsonrpc_response"] = response
        return parsed
    return {"_non_object_payload": parsed, "_jsonrpc_response": response}


def _call_tool(client: McpClient, name: str, arguments: dict[str, Any], timeout: float = 120.0) -> dict[str, Any]:
    return _tool_payload(client.call_tool(name, arguments, timeout=timeout))


def _find_duplicate_values(values: Iterable[str]) -> dict[str, int]:
    counts: dict[str, int] = {}
    for value in values:
        counts[value] = counts.get(value, 0) + 1
    return {value: count for value, count in counts.items() if count > 1}


def _assert_unique_values(values: Iterable[str], *, label: str) -> None:
    duplicates = _find_duplicate_values(values)
    if duplicates:
        details = ", ".join(f"{value} ({count}x)" for value, count in sorted(duplicates.items()))
        raise AssertionError(f"{label} must be unique; duplicate values: {details}")


def _path_contains_symbol(path: Path, symbol: str) -> bool:
    text = path.read_text(encoding="utf-8")
    return symbol in text


def _validate_code_label_drift(
    row: dict[str, Any],
    *,
    source: str,
    allow_repo_checks: bool = True,
) -> dict[str, int]:
    query_id = str(row.get("query_id") or "<missing-query-id>")
    query_type = str(row.get("query_type") or "")
    expected_paths = [str(path) for path in row.get("expected_paths", [])]
    expected_symbols = [str(symbol) for symbol in row.get("expected_symbols", [])]

    if query_type in V2_LABEL_QA_SENTINEL_QUERY_TYPES:
        if expected_paths or expected_symbols:
            raise AssertionError(
                f"Code query {query_id} in {source} is a negative/deleted/renamed sentinel and must keep empty expected_paths/expected_symbols"
            )
        return {"checked_paths": 0, "checked_symbols": 0}

    if not expected_paths and not expected_symbols:
        raise AssertionError(
            f"Code query {query_id} in {source} has no positive expected_paths or expected_symbols; "
            "use a negative/deleted/renamed query_type for empty labels"
        )

    if not allow_repo_checks:
        return {"checked_paths": 0, "checked_symbols": 0}

    checked_paths: list[Path] = []
    for rel_path in expected_paths:
        path = ROOT / rel_path
        if not path.exists():
            raise AssertionError(f"Code query {query_id} in {source} expected path does not exist: {rel_path}")
        if not path.is_file():
            raise AssertionError(f"Code query {query_id} in {source} expected path is not a file: {rel_path}")
        checked_paths.append(path)

    for symbol in expected_symbols:
        if not symbol.strip():
            raise AssertionError(f"Code query {query_id} in {source} contains blank expected symbol")
        if checked_paths and not any(_path_contains_symbol(path, symbol) for path in checked_paths):
            searched = [str(path.relative_to(ROOT)) for path in checked_paths]
            raise AssertionError(
                f"Code query {query_id} in {source} expected symbol {symbol!r} was not found in expected paths: {searched}"
            )

    return {"checked_paths": len(checked_paths), "checked_symbols": len(expected_symbols)}


def _fixture_tier_choices_text() -> str:
    return ", ".join(V2_FIXTURE_TIERS)


def _parse_fixture_tier(value: str) -> str:
    normalized = value.strip().lower()
    if normalized not in V2_FIXTURE_TIERS:
        raise argparse.ArgumentTypeError(
            f"invalid fixture tier '{value}'; valid choices: {_fixture_tier_choices_text()}"
        )
    return normalized


def _summarize_label_qa(query_set: Sequence[dict[str, Any]]) -> dict[str, Any]:
    sentinel_query_count = 0
    labeled_query_count = 0
    expected_path_total = 0
    expected_symbol_total = 0
    retrieval_modes: set[str] = set()
    for row in query_set:
        query_type = str(row.get("query_type") or "")
        if query_type in V2_LABEL_QA_SENTINEL_QUERY_TYPES:
            sentinel_query_count += 1
        else:
            labeled_query_count += 1
            expected_path_total += len(row.get("expected_paths", []))
            expected_symbol_total += len(row.get("expected_symbols", []))

        mode = row.get("mode")
        if isinstance(mode, str) and mode.strip():
            retrieval_modes.add(mode)

    return {
        "status": "validated",
        "query_count": len(query_set),
        "labeled_query_count": labeled_query_count,
        "sentinel_query_count": sentinel_query_count,
        "expected_path_total": expected_path_total,
        "expected_symbol_total": expected_symbol_total,
        "retrieval_modes": sorted(retrieval_modes),
    }


def _validate_query_set(query_set: list[Any], *, source: str, fixture_tier: str) -> list[dict[str, Any]]:
    if not isinstance(query_set, list):
        raise TypeError(f"Invalid query set shape in {source}: expected list")
    if not query_set:
        raise ValueError(f"Query set is empty in {source} for fixture tier '{fixture_tier}'")

    validated: list[dict[str, Any]] = []
    query_ids: list[str] = []
    drift_summary = {"checked_paths": 0, "checked_symbols": 0}
    for idx, row in enumerate(query_set, start=1):
        if not isinstance(row, dict):
            raise TypeError(f"Invalid query row #{idx} in {source}: expected object")

        query_id = row.get("query_id")
        query_text = row.get("query")
        query_type = row.get("query_type")
        expected_paths = row.get("expected_paths", [])
        expected_symbols = row.get("expected_symbols", [])
        rationale = row.get("rationale")

        if not isinstance(query_id, str) or not query_id.strip():
            raise ValueError(f"Invalid query_id at row #{idx} in {source}")
        query_ids.append(query_id)
        if not isinstance(query_text, str) or not query_text.strip():
            raise ValueError(f"Invalid query text for {query_id} in {source}")
        if not isinstance(query_type, str) or not query_type.strip():
            raise ValueError(f"Invalid query_type for {query_id} in {source}")
        if not isinstance(expected_paths, list):
            raise TypeError(f"Invalid expected_paths for {query_id} in {source}: expected list")
        if not isinstance(expected_symbols, list):
            raise TypeError(f"Invalid expected_symbols for {query_id} in {source}: expected list")
        if not isinstance(rationale, str) or not rationale.strip():
            raise ValueError(
                f"Missing explicit rationale for {query_id} in {source}. "
                "V2 code golden queries must define rationale for drift visibility."
            )

        checked = _validate_code_label_drift(row, source=source)
        drift_summary["checked_paths"] += checked["checked_paths"]
        drift_summary["checked_symbols"] += checked["checked_symbols"]

        validated.append(row)

    _assert_unique_values(query_ids, label=f"Code query IDs in {source}")

    return validated


def _load_legacy_query_set(path: Path = LEGACY_BASELINE_JSON) -> list[dict[str, Any]]:
    payload = json.loads(path.read_text(encoding="utf-8"))
    query_set = payload.get("query_set")
    if not isinstance(query_set, list):
        raise TypeError(f"Invalid legacy baseline query set shape in {path}: expected list at query_set")
    return _validate_query_set(query_set, source=str(path.relative_to(ROOT)), fixture_tier="small")


def _load_code_query_set(
    *,
    fixture_tier: str,
    canonical_path: Path = CODE_GOLDEN_V2_JSON,
    legacy_path: Path = LEGACY_BASELINE_JSON,
) -> tuple[list[dict[str, Any]], dict[str, Any]]:
    payload = json.loads(canonical_path.read_text(encoding="utf-8"))
    if str(payload.get("schema_version")) != V2_SCHEMA_VERSION:
        raise ValueError(
            f"Invalid schema_version in {canonical_path.relative_to(ROOT)}: "
            f"expected {V2_SCHEMA_VERSION}, got {payload.get('schema_version')}"
        )

    tiers = payload.get("tiers")
    if not isinstance(tiers, dict):
        raise TypeError(f"Invalid V2 query file in {canonical_path.relative_to(ROOT)}: expected object at tiers")

    tier_payload = tiers.get(fixture_tier)
    if not isinstance(tier_payload, dict):
        raise ValueError(
            f"Fixture tier '{fixture_tier}' missing from {canonical_path.relative_to(ROOT)}"
        )

    tier_queries = tier_payload.get("queries")
    bridge_used = False
    source = str(canonical_path.relative_to(ROOT))

    if isinstance(tier_queries, list):
        query_set = _validate_query_set(
            tier_queries,
            source=f"{source}::tiers.{fixture_tier}.queries",
            fixture_tier=fixture_tier,
        )
    elif fixture_tier == "small":
        query_set = _load_legacy_query_set(legacy_path)
        bridge_used = True
        source = str(legacy_path.relative_to(ROOT))
    else:
        raise TypeError(
            f"Invalid query list for fixture tier '{fixture_tier}' in {canonical_path.relative_to(ROOT)}"
        )

    return query_set, {
        "source": source,
        "canonical_source": str(canonical_path.relative_to(ROOT)),
        "legacy_bridge_source": str(legacy_path.relative_to(ROOT)),
        "legacy_bridge_used": bridge_used,
        "fixture_tier": fixture_tier,
    }


def _fixture_project(parent: Path) -> Path:
    fixture = parent / "task-9-code-retrieval-fixture"
    for rel in REQUIRED_SOURCE_PATHS:
        src = ROOT / rel
        dst = fixture / rel
        dst.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(src, dst)
    return fixture


def _paths_from_results(payload: dict[str, Any], limit: int = 10) -> list[str]:
    paths: list[str] = []
    for row in payload.get("results", []):
        if not isinstance(row, dict):
            continue
        path = row.get("file_path")
        if isinstance(path, str) and path:
            paths.append(path)
        if len(paths) >= limit:
            break
    return paths


def _normalize_result_ids_for_expected(paths: Sequence[str], expected_paths: Sequence[str]) -> list[str]:
    expected = [str(path) for path in expected_paths]
    normalized: list[str] = []
    for path in paths:
        matched = next((exp for exp in expected if path.endswith(exp) or path.endswith("/" + exp)), None)
        normalized.append(matched or path)
    return normalized


def _classify_code_failure_type(
    *,
    query_type: str,
    expected_paths: Sequence[str],
    result_paths: Sequence[str],
    expected_rank: int | None,
    query_error: str | None,
    reason_code: str | None = None,
) -> str:
    if query_error:
        return "call_error"

    if reason_code in {"degraded", "partial", "stale", "generation_mismatch"}:
        if not result_paths:
            return "embedding_not_ready"

    expects_no_match = query_type in V2_LABEL_QA_SENTINEL_QUERY_TYPES or not expected_paths
    if expects_no_match:
        if not result_paths:
            return "expected_no_match"
        return "true_miss"

    if expected_rank is not None:
        return "none"

    if not result_paths:
        return "empty_results"

    return "true_miss"


def _extract_reason_code(payload: dict[str, Any]) -> str | None:
    summary = payload.get("summary")
    if not isinstance(summary, dict):
        return None
    partial = summary.get("partial")
    if not isinstance(partial, dict):
        return None
    reason_code = partial.get("reason_code")
    return reason_code if isinstance(reason_code, str) and reason_code else None


def _blocker(
    *,
    phase: str,
    command_or_tool: str,
    message: str,
    stderr_tail: Sequence[str],
    reason_code: str | None,
) -> dict[str, Any]:
    blocker = {
        "phase": phase,
        "command_or_tool": command_or_tool,
        "message": message,
        "stderr_tail": [str(line) for line in stderr_tail if str(line).strip()],
        "summary_partial_reason_code": reason_code,
    }
    if reason_code:
        blocker["reason_code_classification"] = classify_reason_code(reason_code, evidence={"retrieval_blocked": True})
    return blocker


def _wait_embedding_ready(
    client: McpClient,
    *,
    timeout_s: float,
    poll_interval_s: float,
    observed_reason_codes: set[str],
) -> tuple[dict[str, Any], dict[str, Any] | None]:
    deadline = time.monotonic() + timeout_s
    latest: dict[str, Any] = {}
    while time.monotonic() < deadline:
        latest = _call_tool(client, "get_status", {}, timeout=30)
        rc = _extract_reason_code(latest)
        if rc:
            observed_reason_codes.add(rc)
        status = latest.get("status")
        embedding_raw = latest.get("embedding")
        embedding: dict[str, Any] = embedding_raw if isinstance(embedding_raw, dict) else {}
        if status == "ready" or embedding.get("status") == "ready" or status == "healthy":
            return latest, None
        time.sleep(poll_interval_s)
    return latest, _blocker(
        phase="embedding_readiness",
        command_or_tool="get_status",
        message=f"embedding readiness timeout after {timeout_s:.1f}s",
        stderr_tail=client.stderr_tail(80),
        reason_code=_extract_reason_code(latest),
    )


def _wait_structural_ready(
    client: McpClient,
    *,
    project_id: str,
    timeout_s: float,
    poll_interval_s: float,
    observed_reason_codes: set[str],
) -> tuple[dict[str, Any], dict[str, Any] | None]:
    deadline = time.monotonic() + timeout_s
    latest: dict[str, Any] = {}
    while time.monotonic() < deadline:
        latest = _call_tool(client, "project_info", {"action": "status", "project_id": project_id}, timeout=45)
        rc = _extract_reason_code(latest)
        if rc:
            observed_reason_codes.add(rc)

        lifecycle = latest.get("contract", {}).get("generation_basis", {}).get("lifecycle", {})
        structural = lifecycle.get("structural", {}) if isinstance(lifecycle, dict) else {}
        state = str(latest.get("status", latest.get("state", ""))).lower()
        if structural.get("is_ready") is True or state in {"completed", "ready", "healthy"}:
            return latest, None
        if "failed" in state:
            return latest, _blocker(
                phase="index_readiness",
                command_or_tool="project_info(action=status)",
                message=f"index reached failure state: {state}",
                stderr_tail=client.stderr_tail(80),
                reason_code=_extract_reason_code(latest),
            )
        time.sleep(poll_interval_s)

    return latest, _blocker(
        phase="index_readiness",
        command_or_tool="project_info(action=status)",
        message=f"structural index readiness timeout after {timeout_s:.1f}s",
        stderr_tail=client.stderr_tail(80),
        reason_code=_extract_reason_code(latest),
    )


def _symbol_probe(
    client: McpClient,
    *,
    project_id: str,
    expected_symbols: Sequence[str],
    query_timeout_s: float,
    observed_reason_codes: set[str],
) -> dict[str, Any]:
    if not expected_symbols:
        return {"searched": False, "symbol_count": 0, "symbol_graph_edges": 0}

    symbol_query = str(expected_symbols[0])
    search_payload = _call_tool(
        client,
        "search_symbols",
        {"query": symbol_query, "project_id": project_id, "limit": 5},
        timeout=query_timeout_s,
    )
    rc = _extract_reason_code(search_payload)
    if rc:
        observed_reason_codes.add(rc)
    results = search_payload.get("results", [])
    first_symbol = results[0] if isinstance(results, list) and results and isinstance(results[0], dict) else {}
    symbol_id = first_symbol.get("symbol_id") or first_symbol.get("id")

    graph_payload: dict[str, Any] | None = None
    if isinstance(symbol_id, str) and symbol_id:
        graph_payload = _call_tool(
            client,
            "symbol_graph",
            {"symbol_id": symbol_id, "action": "related", "project_id": project_id, "depth": 1},
            timeout=query_timeout_s,
        )
        rc = _extract_reason_code(graph_payload)
        if rc:
            observed_reason_codes.add(rc)

    edges = 0
    if isinstance(graph_payload, dict):
        rels = graph_payload.get("relationships")
        if isinstance(rels, list):
            edges = len(rels)

    return {
        "searched": True,
        "query": symbol_query,
        "symbol_count": len(results) if isinstance(results, list) else 0,
        "first_symbol_id": symbol_id,
        "symbol_graph_edges": edges,
    }


def _to_markdown_title() -> str:
    return "Code Retrieval Baseline"


def _canonical_baseline_targets() -> tuple[Path, Path]:
    return OUTPUT_JSON.resolve(), OUTPUT_MD.resolve()


def _is_canonical_target_pair(output_json: Path, output_md: Path) -> bool:
    canonical_json, canonical_md = _canonical_baseline_targets()
    return output_json.resolve() == canonical_json and output_md.resolve() == canonical_md


def _non_refresh_report_paths(benchmark_name: str, fixture_tier: str) -> tuple[Path, Path]:
    timestamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    base = V2_EVIDENCE_DIR / "runs" / f"{benchmark_name}-{fixture_tier}-{timestamp}"
    return base.with_suffix(".json"), base.with_suffix(".md")


def _threshold_evaluation_deferred(*, fixture_tier: str) -> dict[str, Any]:
    return {
        "policy_name": V2_THRESHOLD_POLICY,
        "enforcement": "local-only",
        "status": "deferred",
        "reason": "full local-v2 threshold matrix is currently defined for memory retrieval reports; code retrieval keeps deferred interpretation",
        "fixture_tier": fixture_tier,
    }


def _assert_raises_with_message(fn: Any, expected_substring: str, *, label: str) -> str:
    try:
        fn()
    except (AssertionError, ValueError, TypeError) as exc:
        message = str(exc)
        if expected_substring not in message:
            raise AssertionError(
                f"{label} raised without expected diagnostic {expected_substring!r}: {message}"
            ) from exc
        return message
    raise AssertionError(f"{label} did not reject invalid input")


def _self_test_code_label_qa_rejections(query_set: list[dict[str, Any]], *, source: str) -> dict[str, str]:
    duplicate_queries = [dict(query) for query in query_set]
    duplicate_queries.append(dict(query_set[0]))

    missing_rationale_queries = [dict(query) for query in query_set]
    missing_rationale_queries[0] = dict(missing_rationale_queries[0])
    missing_rationale_queries[0].pop("rationale", None)

    missing_path_queries = [dict(query) for query in query_set]
    missing_path_query = dict(missing_path_queries[0])
    missing_path_query["expected_paths"] = ["src/does/not/exist.rs"]
    missing_path_queries[0] = missing_path_query

    missing_symbol_queries = [dict(query) for query in query_set]
    missing_symbol_query = dict(missing_symbol_queries[0])
    missing_symbol_query["expected_symbols"] = ["definitely_missing_symbol_for_label_qa"]
    missing_symbol_queries[0] = missing_symbol_query

    sentinel_drift_queries = [dict(query) for query in query_set]
    sentinel_index = next(
        index
        for index, query in enumerate(sentinel_drift_queries)
        if str(query.get("query_type")) in {"negative_no_match", "deleted_symbol_expectation", "renamed_symbol_expectation"}
    )
    sentinel_query = dict(sentinel_drift_queries[sentinel_index])
    sentinel_query["expected_paths"] = ["src/graph/rrf.rs"]
    sentinel_drift_queries[sentinel_index] = sentinel_query

    return {
        "duplicate_query_ids": _assert_raises_with_message(
            lambda: _validate_query_set(duplicate_queries, source=source, fixture_tier="self-test-invalid"),
            "duplicate values",
            label="code duplicate query ID validation",
        ),
        "missing_rationale": _assert_raises_with_message(
            lambda: _validate_query_set(missing_rationale_queries, source=source, fixture_tier="self-test-invalid"),
            "Missing explicit rationale",
            label="code missing rationale validation",
        ),
        "missing_expected_path": _assert_raises_with_message(
            lambda: _validate_query_set(missing_path_queries, source=source, fixture_tier="self-test-invalid"),
            "expected path does not exist",
            label="code expected path drift validation",
        ),
        "missing_expected_symbol": _assert_raises_with_message(
            lambda: _validate_query_set(missing_symbol_queries, source=source, fixture_tier="self-test-invalid"),
            "expected symbol",
            label="code expected symbol drift validation",
        ),
        "sentinel_empty_label_contract": _assert_raises_with_message(
            lambda: _validate_query_set(sentinel_drift_queries, source=source, fixture_tier="self-test-invalid"),
            "must keep empty expected_paths/expected_symbols",
            label="code negative/deleted/renamed sentinel validation",
        ),
    }


def run_self_test() -> int:
    tier_source_meta: dict[str, dict[str, Any]] = {}
    tier_query_counts: dict[str, int] = {}
    tier_label_qa: dict[str, dict[str, Any]] = {}
    tier_query_sets: dict[str, list[dict[str, Any]]] = {}
    for fixture_tier in V2_FIXTURE_TIERS:
        tier_query_set, source_meta = _load_code_query_set(fixture_tier=fixture_tier)
        if source_meta.get("canonical_source") != str(CODE_GOLDEN_V2_JSON.relative_to(ROOT)):
            raise AssertionError(f"self-test tier source mismatch for {fixture_tier}: {source_meta}")
        tier_source_meta[fixture_tier] = source_meta
        tier_query_sets[fixture_tier] = tier_query_set
        tier_query_counts[fixture_tier] = len(tier_query_set)
        tier_label_qa[fixture_tier] = _summarize_label_qa(tier_query_set)

    query_set = tier_query_sets[V2_DEFAULT_FIXTURE_TIER]
    source_meta = tier_source_meta[V2_DEFAULT_FIXTURE_TIER]
    medium_query_set = tier_query_sets["medium"]
    medium_source_meta = tier_source_meta["medium"]
    label_qa_rejections = _self_test_code_label_qa_rejections(
        medium_query_set,
        source=f"{medium_source_meta['source']}::self-test-invalid",
    )
    print(
        f"self-test: loaded baseline query count={len(query_set)} "
        f"from {source_meta['source']} (canonical={source_meta['canonical_source']})"
    )
    print("self-test: validated tier query counts=" + json.dumps(tier_query_counts, sort_keys=True))

    sample_rows: list[dict[str, Any]] = []
    for idx, query in enumerate(query_set[:4], start=1):
        expected_paths = [str(p) for p in query.get("expected_paths", [])]
        query_type = str(query.get("query_type"))
        if expected_paths:
            mock_results = [expected_paths[0], "src/graph/ppr.rs", "src/server/handler.rs"]
        else:
            mock_results = []
        metrics_row = compute_query_metrics(
            mock_results,
            expected_paths,
            query_type=query_type,
            latency_ms=100.0 + (idx * 5.0),
            negative=query_type in V2_LABEL_QA_SENTINEL_QUERY_TYPES,
        )
        sample_rows.append(
            {
                "query_id": query.get("query_id", f"q{idx}"),
                "query_type": query.get("query_type"),
                "result_paths_top_10": mock_results,
                "failure_type": _classify_code_failure_type(
                    query_type=query_type,
                    expected_paths=expected_paths,
                    result_paths=mock_results,
                    expected_rank=metrics_row.get("expected_rank"),
                    query_error=None,
                ),
                **metrics_row,
            }
        )
    sample_rows.extend(
        [
            {
                "query_id": "self_test_expected_no_match",
                "query_type": "negative_no_match",
                "expected_paths": [],
                "result_paths_top_10": [],
                "failure_type": "expected_no_match",
                **compute_query_metrics([], [], query_type="negative_no_match", latency_ms=145.0, negative=True),
            },
            {
                "query_id": "self_test_empty_positive",
                "query_type": "natural_language",
                "expected_paths": ["src/graph/rrf.rs"],
                "result_paths_top_10": [],
                "failure_type": "empty_results",
                **compute_query_metrics([], ["src/graph/rrf.rs"], query_type="natural_language", latency_ms=150.0),
            },
            {
                "query_id": "self_test_true_miss_positive",
                "query_type": "natural_language",
                "expected_paths": ["src/graph/rrf.rs"],
                "result_paths_top_10": ["src/server/handler.rs"],
                "failure_type": "true_miss",
                **compute_query_metrics(["src/server/handler.rs"], ["src/graph/rrf.rs"], query_type="natural_language", latency_ms=155.0),
            },
            {
                "query_id": "self_test_call_error",
                "query_type": "natural_language",
                "expected_paths": ["src/graph/rrf.rs"],
                "result_paths_top_10": [],
                "failure_type": "call_error",
                "query_error": "synthetic failure",
                **compute_query_metrics([], ["src/graph/rrf.rs"], query_type="natural_language", latency_ms=160.0),
            },
        ]
    )

    aggregate = aggregate_metrics(sample_rows)
    aggregate["runtime_minutes"] = 0.0
    aggregate["threshold_evaluation"] = _threshold_evaluation_deferred(fixture_tier=V2_DEFAULT_FIXTURE_TIER)
    aggregate["baseline_diff_summary"] = {
        "status": "deferred",
        "reason": "baseline diff summary is generated by python3 -m evals.lib.metrics --baseline-diff",
    }
    aggregate["readiness_summary"] = {
        "taxonomy": {"reason_codes": [], "reason_code_classification": {}},
        "readiness_fallback": None,
        "threshold_status_reason": aggregate["threshold_evaluation"]["reason"],
    }
    with tempfile.TemporaryDirectory(prefix="task-9-code-self-test-") as tmp:
        tmp_path = Path(tmp)
        json_path = tmp_path / "self-test.json"
        md_path = tmp_path / "self-test.md"
        write_json_report(
            json_path,
            "code_retrieval_benchmark_self_test",
            {
                "source": source_meta["source"],
                "query_catalog_source": source_meta["canonical_source"],
                "legacy_bridge": {
                    "path": source_meta["legacy_bridge_source"],
                    "used": source_meta["legacy_bridge_used"],
                },
                "query_count": len(query_set),
                "medium_query_count": len(medium_query_set),
                "tier_query_counts": tier_query_counts,
                "tier_source_resolution": tier_source_meta,
                "label_qa_rejections": label_qa_rejections,
                "label_qa": tier_label_qa,
                "validated_fixture_tiers": list(V2_FIXTURE_TIERS),
                "schema_version": V2_SCHEMA_VERSION,
                "fixture_tier": V2_DEFAULT_FIXTURE_TIER,
                "baseline_version": V2_DEFAULT_BASELINE_VERSION,
                "model": "e5_small",
                "retrieval_modes": tier_label_qa[V2_DEFAULT_FIXTURE_TIER]["retrieval_modes"],
                "threshold_policy": {"name": V2_THRESHOLD_POLICY, "enforcement": "local-only"},
                "runtime_target": {
                    "target_minutes": "5-10",
                    "required_by_default": True,
                    "optional_policy": "small tier default",
                },
                "determinism_policy": {"name": V2_DETERMINISM_POLICY},
            },
            aggregate,
            sample_rows,
            warnings=["offline self-test"],
            blockers=[],
            stderr_tail=[],
            environment={"mode": "self-test"},
        )
        write_markdown_report(
            md_path,
            "Code Retrieval Self-Test",
            aggregate,
            sample_rows,
            manifest={
                "source": source_meta["source"],
                "query_catalog_source": source_meta["canonical_source"],
                "legacy_bridge": {
                    "path": source_meta["legacy_bridge_source"],
                    "used": source_meta["legacy_bridge_used"],
                },
                "query_count": len(query_set),
                "medium_query_count": len(medium_query_set),
                "tier_query_counts": tier_query_counts,
                "tier_source_resolution": tier_source_meta,
                "label_qa_rejections": label_qa_rejections,
                "label_qa": tier_label_qa,
                "validated_fixture_tiers": list(V2_FIXTURE_TIERS),
                "schema_version": V2_SCHEMA_VERSION,
                "fixture_tier": V2_DEFAULT_FIXTURE_TIER,
                "baseline_version": V2_DEFAULT_BASELINE_VERSION,
                "model": "e5_small",
                "retrieval_modes": tier_label_qa[V2_DEFAULT_FIXTURE_TIER]["retrieval_modes"],
                "threshold_policy": {"name": V2_THRESHOLD_POLICY, "enforcement": "local-only"},
                "runtime_target": {
                    "target_minutes": "5-10",
                    "required_by_default": True,
                    "optional_policy": "small tier default",
                },
                "determinism_policy": {"name": V2_DETERMINISM_POLICY},
            },
            environment={"mode": "self-test"},
            stderr_tail=[],
            warnings=["offline self-test"],
            blockers=[],
        )

        parsed = json.loads(json_path.read_text(encoding="utf-8"))
        if parsed.get("aggregate_metrics", {}).get("query_count") != len(sample_rows):
            raise AssertionError("self-test JSON report query_count mismatch")
        if parsed.get("warnings") != ["offline self-test"]:
            raise AssertionError("self-test JSON report warnings mismatch")
        if parsed.get("blockers") != []:
            raise AssertionError("self-test JSON report blockers mismatch")
        if parsed.get("stderr_tail") != []:
            raise AssertionError("self-test JSON report stderr_tail mismatch")
        if parsed.get("schema_version") is None:
            raise AssertionError("self-test JSON report missing schema_version")
        if parsed.get("fixture_tier") is None:
            raise AssertionError("self-test JSON report missing fixture_tier")
        if parsed.get("threshold_status") is None:
            raise AssertionError("self-test JSON report missing threshold_status")
        if not isinstance(parsed.get("failure_buckets"), dict):
            raise AssertionError("self-test JSON report missing failure_buckets")
        expected_failure_buckets = {"none", "expected_no_match", "empty_results", "true_miss", "call_error"}
        missing_failure_buckets = expected_failure_buckets - set(parsed["failure_buckets"])
        if missing_failure_buckets:
            raise AssertionError(f"self-test JSON report missing failure buckets: {sorted(missing_failure_buckets)}")
        if not isinstance(parsed.get("readiness_summary"), dict):
            raise AssertionError("self-test JSON report missing readiness_summary")
        if "Code Retrieval Self-Test" not in md_path.read_text(encoding="utf-8"):
            raise AssertionError("self-test markdown title missing")
        markdown = md_path.read_text(encoding="utf-8")
        if "## Benchmark V2 summary" not in markdown:
            raise AssertionError("self-test markdown V2 summary missing")
        if "| expected_no_match |" not in markdown or "| true_miss |" not in markdown:
            raise AssertionError("self-test markdown failure bucket taxonomy missing")

    if _classify_code_failure_type(
        query_type="negative_no_match",
        expected_paths=[],
        result_paths=[],
        expected_rank=None,
        query_error=None,
    ) != "expected_no_match":
        raise AssertionError("code expected_no_match classification drifted")
    if _classify_code_failure_type(
        query_type="natural_language",
        expected_paths=["src/graph/rrf.rs"],
        result_paths=[],
        expected_rank=None,
        query_error=None,
    ) != "empty_results":
        raise AssertionError("code empty_results classification drifted")
    if _classify_code_failure_type(
        query_type="natural_language",
        expected_paths=["src/graph/rrf.rs"],
        result_paths=["src/server/handler.rs"],
        expected_rank=None,
        query_error=None,
    ) != "true_miss":
        raise AssertionError("code true_miss classification drifted")

    if not _is_canonical_target_pair(OUTPUT_JSON, OUTPUT_MD):
        raise AssertionError("code canonical target pair detection drifted")
    redirected_json, redirected_md = _non_refresh_report_paths(BENCHMARK_NAME, V2_DEFAULT_FIXTURE_TIER)
    if redirected_json.parent != V2_EVIDENCE_DIR / "runs" or redirected_md.parent != V2_EVIDENCE_DIR / "runs":
        raise AssertionError("code non-refresh output directory drifted")
    if redirected_json.suffix != ".json" or redirected_md.suffix != ".md":
        raise AssertionError("code non-refresh output suffix drifted")
    if _is_canonical_target_pair(redirected_json, redirected_md):
        raise AssertionError("code non-refresh output unexpectedly resolves to canonical targets")

    print("self-test passed")
    return 0


def run_benchmark(args: argparse.Namespace) -> int:
    EVIDENCE_DIR.mkdir(parents=True, exist_ok=True)
    V2_EVIDENCE_DIR.mkdir(parents=True, exist_ok=True)
    query_set, source_meta = _load_code_query_set(fixture_tier=args.fixture_tier)
    label_qa_summary = _summarize_label_qa(query_set)
    resolved_command = resolve_mcp_command()

    blockers: list[dict[str, Any]] = []
    warnings: list[str] = []
    observed_reason_codes: set[str] = set()
    per_query: list[dict[str, Any]] = []
    raw: dict[str, Any] = {}
    stderr_tail: list[str] = []

    run_started = time.time()
    with tempfile.TemporaryDirectory(prefix="task-9-code-retrieval-data-") as data_dir, tempfile.TemporaryDirectory(
        prefix="task-9-code-retrieval-project-"
    ) as project_tmp:
        fixture_project = _fixture_project(Path(project_tmp))
        env = build_env({"DATA_DIR": data_dir, "EMBEDDING_MODEL": args.embedding_model, "RUST_LOG": "warn"})

        with McpClient.start(
            command=resolved_command,
            root=ROOT,
            env_overrides=env,
            timeout=60,
            client_name="code-retrieval-benchmark",
            client_version="0.1.0",
        ) as client:
            startup_status = _call_tool(client, "get_status", {}, timeout=45)
            raw["startup_status"] = startup_status
            rc = _extract_reason_code(startup_status)
            if rc:
                observed_reason_codes.add(rc)

            embedding_status, embedding_blocker = _wait_embedding_ready(
                client,
                timeout_s=args.embedding_timeout_s,
                poll_interval_s=args.poll_interval_s,
                observed_reason_codes=observed_reason_codes,
            )
            raw["embedding_readiness"] = embedding_status
            if embedding_blocker:
                blockers.append(embedding_blocker)

            index_payload = _call_tool(
                client,
                "index_project",
                {"path": str(fixture_project), "force": True, "confirm_failed_restart": True},
                timeout=120,
            )
            raw["index_project"] = index_payload
            project_id = str(index_payload.get("project_id") or fixture_project.name)

            index_status, index_blocker = _wait_structural_ready(
                client,
                project_id=project_id,
                timeout_s=args.index_timeout_s,
                poll_interval_s=args.poll_interval_s,
                observed_reason_codes=observed_reason_codes,
            )
            raw["index_readiness"] = index_status
            if index_blocker:
                blockers.append(index_blocker)

            # Blocker-first behavior for readiness timeout: emit structured blocker output.
            if blockers:
                warnings.append("readiness blocker encountered; query loop skipped")
            else:
                for idx, query in enumerate(query_set, start=1):
                    t0 = time.perf_counter()
                    query_id = str(query.get("query_id", f"query_{idx}"))
                    expected_paths = [str(path) for path in query.get("expected_paths", [])]
                    query_type = str(query.get("query_type", "unknown"))
                    query_args = {
                        "query": query.get("query", ""),
                        "project_id": project_id,
                        "limit": int(query.get("limit", 10)),
                        "mode": str(query.get("mode", "hybrid")),
                    }

                    payload: dict[str, Any]
                    query_error: str | None = None
                    try:
                        payload = _call_tool(client, "recall_code", query_args, timeout=args.query_timeout_s)
                    except Exception as exc:  # noqa: BLE001
                        payload = {"error": str(exc)}
                        query_error = str(exc)
                        blockers.append(
                            _blocker(
                                phase="query_execution",
                                command_or_tool="recall_code",
                                message=f"query {query_id} failed: {exc}",
                                stderr_tail=client.stderr_tail(80),
                                reason_code=None,
                            )
                        )

                    latency_ms = round((time.perf_counter() - t0) * 1000.0, 2)
                    rc = _extract_reason_code(payload)
                    if rc:
                        observed_reason_codes.add(rc)

                    top_paths = _paths_from_results(payload, limit=10)
                    normalized_result_ids = _normalize_result_ids_for_expected(top_paths, expected_paths)
                    metrics_row = compute_query_metrics(
                        normalized_result_ids,
                        expected_paths,
                        query_type=query_type,
                        latency_ms=latency_ms,
                        negative=query_type == "negative_no_match",
                    )

                    symbol_probe: dict[str, Any]
                    try:
                        symbol_probe = _symbol_probe(
                            client,
                            project_id=project_id,
                            expected_symbols=[str(v) for v in query.get("expected_symbols", [])],
                            query_timeout_s=args.query_timeout_s,
                            observed_reason_codes=observed_reason_codes,
                        )
                    except Exception as exc:  # noqa: BLE001
                        symbol_probe = {"searched": True, "probe_error": str(exc), "symbol_count": 0, "symbol_graph_edges": 0}
                        blockers.append(
                            _blocker(
                                phase="symbol_probe",
                                command_or_tool="search_symbols/symbol_graph",
                                message=f"query {query_id} symbol probe failed: {exc}",
                                stderr_tail=client.stderr_tail(80),
                                reason_code=None,
                            )
                        )

                    failure_type = _classify_code_failure_type(
                        query_type=query_type,
                        expected_paths=expected_paths,
                        result_paths=top_paths,
                        expected_rank=metrics_row.get("expected_rank"),
                        query_error=query_error,
                        reason_code=rc,
                    )

                    per_query.append(
                        {
                            "run_order": idx,
                            "query_id": query_id,
                            "query": query.get("query"),
                            "mode": query.get("mode", "hybrid"),
                            "limit": query.get("limit", 10),
                            "expected_paths": expected_paths,
                            "expected_symbols": [str(v) for v in query.get("expected_symbols", [])],
                            "result_paths_top_10": top_paths,
                            "summary_partial_reason_code": rc,
                            "reason_code_classification": classify_reason_code(
                                rc,
                                evidence={
                                    "retrieval_blocked": query_error is not None,
                                    "failure_type": failure_type,
                                },
                            ) if rc else None,
                            "failure_type": failure_type,
                            "query_error": query_error,
                            "symbol_probe": symbol_probe,
                            **metrics_row,
                        }
                    )
                    raw[f"query_{idx:02d}_{query_id}"] = payload

            raw["project_info_stats"] = _call_tool(client, "project_info", {"action": "stats", "project_id": project_id}, timeout=60)
            stderr_tail = client.stderr_tail(80)

    aggregate = aggregate_metrics(per_query)
    aggregate["runtime_minutes"] = round((time.time() - run_started) / 60.0, 6)
    aggregate["baseline_query_count"] = len(query_set)
    aggregate["readiness_timeout"] = bool(any(b.get("phase") in {"embedding_readiness", "index_readiness"} for b in blockers))
    aggregate["observed_summary_partial_reason_codes"] = sorted(observed_reason_codes)
    aggregate["reason_code_classification"] = classify_reason_codes(
        aggregate["observed_summary_partial_reason_codes"],
        evidence={
            "blocker_count": len(blockers),
            "readiness_timeout": aggregate["readiness_timeout"],
        },
    )
    aggregate["threshold_evaluation"] = _threshold_evaluation_deferred(fixture_tier=args.fixture_tier)
    aggregate["baseline_diff_summary"] = {
        "status": "deferred",
        "reason": "baseline diff summary is generated by python3 -m evals.lib.metrics --baseline-diff",
    }

    environment = {
        "root": str(ROOT),
        "server_command": resolved_command,
        "fixture_tier": args.fixture_tier,
        "baseline_version": args.baseline_version,
        "model": args.embedding_model,
        "retrieval_modes": label_qa_summary["retrieval_modes"],
        "label_qa_status": label_qa_summary["status"],
        "query_source": source_meta["source"],
        "query_catalog_source": source_meta["canonical_source"],
        "legacy_bridge": {
            "path": source_meta["legacy_bridge_source"],
            "used": source_meta["legacy_bridge_used"],
        },
        "required_source_paths": list(REQUIRED_SOURCE_PATHS),
        "data_dir_strategy": "temporary isolated DATA_DIR",
        "fixture_project_strategy": "temporary copied fixture project",
        "started_at_utc": datetime.fromtimestamp(run_started, tz=timezone.utc).isoformat(),
        "duration_seconds": round(time.time() - run_started, 2),
    }

    manifest = {
        "task": "9. Migrate code retrieval benchmark to evals",
        "query_set_source": source_meta["source"],
        "query_catalog_source": source_meta["canonical_source"],
        "legacy_bridge": {
            "path": source_meta["legacy_bridge_source"],
            "used": source_meta["legacy_bridge_used"],
        },
        "query_count": len(query_set),
        "tools_used": ["index_project", "project_info(status)", "recall_code", "search_symbols", "symbol_graph"],
        "schema_version": V2_SCHEMA_VERSION,
        "fixture_tier": args.fixture_tier,
        "baseline_version": args.baseline_version,
        "model": args.embedding_model,
        "retrieval_modes": label_qa_summary["retrieval_modes"],
        "label_qa": label_qa_summary,
        "threshold_policy": {"name": V2_THRESHOLD_POLICY, "enforcement": "local-only"},
        "runtime_target": {
            "target_minutes": "5-10",
            "required_by_default": True,
            "optional_policy": "small tier default",
        },
        "determinism_policy": {"name": V2_DETERMINISM_POLICY},
    }

    output_json = Path(args.output_json)
    output_md = Path(args.output_md)
    effective_output_json = output_json
    effective_output_md = output_md
    if _is_canonical_target_pair(output_json, output_md):
        effective_output_json, effective_output_md = _non_refresh_report_paths(BENCHMARK_NAME, args.fixture_tier)
        warnings.append(
            "canonical code baseline targets are protected for normal runs; output was redirected to benchmark-v2 runs"
        )

    write_json_report(
        effective_output_json,
        "code_retrieval_baseline",
        manifest,
        aggregate,
        per_query,
        warnings=warnings,
        blockers=blockers,
        stderr_tail=stderr_tail,
        environment={**environment, "raw": raw},
    )
    write_markdown_report(
        effective_output_md,
        _to_markdown_title(),
        aggregate,
        per_query,
        manifest=manifest,
        environment={**environment, "raw": raw},
        stderr_tail=stderr_tail,
        warnings=warnings,
        blockers=blockers,
    )

    print(
        json.dumps(
                {
                    "output_json": str(effective_output_json),
                    "output_md": str(effective_output_md),
                    "query_count": len(query_set),
                    "ran_queries": len(per_query),
                    "blocker_count": len(blockers),
                "observed_reason_codes": sorted(observed_reason_codes),
            },
            indent=2,
        )
    )
    return 0 if not blockers else 2


def parse_args(argv: Iterable[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Code retrieval benchmark migrated to eval helpers.")
    parser.add_argument("--self-test", action="store_true", help="Run offline self-test without starting MCP.")
    parser.add_argument("--embedding-model", default="e5_small", help="Embedding model for benchmark process env.")
    parser.add_argument("--embedding-timeout-s", type=float, default=180.0)
    parser.add_argument("--index-timeout-s", type=float, default=240.0)
    parser.add_argument("--query-timeout-s", type=float, default=120.0)
    parser.add_argument("--poll-interval-s", type=float, default=2.0)
    parser.add_argument("--output-json", default=str(OUTPUT_JSON))
    parser.add_argument("--output-md", default=str(OUTPUT_MD))
    parser.add_argument(
        "--fixture-tier",
        "--tier",
        dest="fixture_tier",
        type=_parse_fixture_tier,
        default=V2_DEFAULT_FIXTURE_TIER,
        metavar="TIER",
        help=f"Fixture/query tier selection (default: {V2_DEFAULT_FIXTURE_TIER}). Valid choices: {_fixture_tier_choices_text()}.",
    )
    parser.add_argument("--baseline-version", default=V2_DEFAULT_BASELINE_VERSION)
    return parser.parse_args(list(argv) if argv is not None else None)


def main(argv: Iterable[str] | None = None) -> int:
    args = parse_args(argv)
    if args.self_test:
        return run_self_test()
    return run_benchmark(args)


if __name__ == "__main__":
    raise SystemExit(main())
