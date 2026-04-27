from __future__ import annotations

import argparse
import json
import sys
import tempfile
import time
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Iterable, Protocol, Sequence

PROJECT_ROOT = Path(__file__).resolve().parents[1]
if str(PROJECT_ROOT) not in sys.path:
    sys.path.insert(0, str(PROJECT_ROOT))

try:
    from evals.lib.mcp_client import McpClient, build_env, resolve_mcp_command
    from evals.lib.metrics import aggregate_metrics, compute_query_metrics, write_json_report, write_markdown_report
except ModuleNotFoundError:  # pragma: no cover - supports `python3 evals/...` direct script invocation
    from lib.mcp_client import McpClient, build_env, resolve_mcp_command
    from lib.metrics import aggregate_metrics, compute_query_metrics, write_json_report, write_markdown_report


HERE = Path(__file__).resolve().parent
ROOT = HERE.parent
FIXTURE_CORPUS_PATH = HERE / "fixtures" / "memory_corpus.json"
FIXTURE_GRAPH_PATH = HERE / "fixtures" / "memory_graph.json"
GOLDEN_QUERIES_PATH = HERE / "golden" / "memory_retrieval_queries.json"
EVIDENCE_DIR = ROOT / ".sisyphus" / "evidence" / "evals"
OUTPUT_JSON = EVIDENCE_DIR / "memory-retrieval-baseline.json"
OUTPUT_MD = EVIDENCE_DIR / "memory-retrieval-baseline.md"
BENCHMARK_NAME = "memory_retrieval_baseline"

EXPECTED_MEMORY_COUNT = 15
EXPECTED_GRAPH_ENTITY_COUNT = 5
EXPECTED_GRAPH_RELATION_COUNT = 8
EXPECTED_GOLDEN_QUERY_COUNT = 10
MEMORY_CORPUS_MIN_COUNT = 15
MEMORY_CORPUS_MAX_COUNT = 30
GRAPH_ENTITY_MAX_COUNT = 5
GRAPH_RELATION_MAX_COUNT = 8
FIXTURE_SCHEMA_VERSION = 1

REQUIRED_QUERY_TYPES = {
    "recall_fusion",
    "search_vector",
    "search_bm25",
    "get_valid_temporal",
    "get_valid_filtered",
    "negative_no_match",
}


class McpClientLike(Protocol):
    def call_tool(self, name: str, arguments: dict[str, Any] | None = None, timeout: float = 30.0) -> dict[str, Any]:
        ...


def _utc_now() -> str:
    return time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())


def _new_phase(name: str) -> dict[str, Any]:
    return {"phase": name, "status": "in_progress", "started_at": _utc_now()}


def _complete_phase(phase: dict[str, Any], **details: Any) -> dict[str, Any]:
    phase["status"] = "completed"
    phase["finished_at"] = _utc_now()
    phase.update(details)
    return phase


def _block_phase(phase: dict[str, Any], message: str, **details: Any) -> dict[str, Any]:
    phase["status"] = "blocked"
    phase["finished_at"] = _utc_now()
    phase["error"] = message
    phase.update(details)
    return phase


def _load_json(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def load_memory_corpus(path: Path = FIXTURE_CORPUS_PATH) -> list[dict[str, Any]]:
    payload = _load_json(path)
    if payload.get("version") != FIXTURE_SCHEMA_VERSION:
        raise AssertionError(f"Invalid memory fixture version in {path}: expected {FIXTURE_SCHEMA_VERSION}")
    memories = payload.get("memories", [])
    if not isinstance(memories, list):
        raise TypeError(f"Invalid fixture shape in {path}: expected 'memories' list")
    return memories


def load_graph_fixture(path: Path = FIXTURE_GRAPH_PATH) -> dict[str, list[dict[str, Any]]]:
    payload = _load_json(path)
    if payload.get("version") != FIXTURE_SCHEMA_VERSION:
        raise AssertionError(f"Invalid graph fixture version in {path}: expected {FIXTURE_SCHEMA_VERSION}")
    entities = payload.get("entities", [])
    relations = payload.get("relations", [])
    if not isinstance(entities, list) or not isinstance(relations, list):
        raise TypeError(f"Invalid graph fixture shape in {path}: expected entities/relations lists")
    return {"entities": entities, "relations": relations}


def load_golden_queries(path: Path = GOLDEN_QUERIES_PATH) -> list[dict[str, Any]]:
    payload = _load_json(path)
    if payload.get("version") != FIXTURE_SCHEMA_VERSION:
        raise AssertionError(f"Invalid golden query version in {path}: expected {FIXTURE_SCHEMA_VERSION}")
    queries = payload.get("queries", [])
    if not isinstance(queries, list):
        raise TypeError(f"Invalid golden query shape in {path}: expected 'queries' list")
    return queries


def settle_after_seeding(
    client: McpClientLike | None,
    *,
    readiness_timeout_s: float = 180.0,
    poll_interval_s: float = 2.0,
    fallback_sleep_s: float = 3.0,
    no_signal_fallback_after: int = 3,
    max_status_errors_before_fallback: int = 3,
    timeout_per_call_s: float = 30.0,
) -> dict[str, Any]:
    start = time.monotonic()
    snapshots: list[dict[str, Any]] = []
    status_errors: list[str] = []

    if client is None:
        if fallback_sleep_s > 0:
            time.sleep(fallback_sleep_s)
        return {
            "status": "fallback_sleep_only",
            "reason": "no_client",
            "elapsed_s": round(time.monotonic() - start, 3),
            "poll_attempts": 0,
            "readiness_signal": "unavailable",
            "fallback_sleep_s": fallback_sleep_s,
            "poll_snapshots": snapshots,
            "status_errors": status_errors,
        }

    deadline = start + max(0.0, readiness_timeout_s)
    attempts = 0
    no_signal_count = 0

    while time.monotonic() < deadline:
        attempts += 1
        try:
            decoded = _decode_tool_response(client.call_tool("get_status", {}, timeout=timeout_per_call_s))
            payload = decoded["payload"]
            signal: bool | None = None
            signal_source = "none"

            if isinstance(payload.get("ready"), bool):
                signal = bool(payload["ready"])
                signal_source = "ready"
            else:
                status_value = payload.get("status")
                if isinstance(status_value, str):
                    normalized = status_value.strip().lower()
                    if normalized in {"ready", "ok", "initialized", "complete", "completed"}:
                        signal = True
                        signal_source = "status"
                    elif normalized in {"starting", "initializing", "indexing", "warming", "not_ready", "in_progress"}:
                        signal = False
                        signal_source = "status"

            snapshots.append(
                {
                    "attempt": attempts,
                    "ready": signal,
                    "signal_source": signal_source,
                    "status": payload.get("status") if isinstance(payload, dict) else None,
                    "parse_errors": decoded.get("parse_errors", []),
                }
            )

            if signal is True:
                return {
                    "status": "ready",
                    "reason": "readiness_signal",
                    "elapsed_s": round(time.monotonic() - start, 3),
                    "poll_attempts": attempts,
                    "readiness_signal": signal_source,
                    "fallback_sleep_s": fallback_sleep_s,
                    "poll_snapshots": snapshots,
                    "status_errors": status_errors,
                }

            if signal is None:
                no_signal_count += 1
                if no_signal_count >= max(1, no_signal_fallback_after):
                    if fallback_sleep_s > 0:
                        time.sleep(fallback_sleep_s)
                    return {
                        "status": "fallback_after_no_signal",
                        "reason": "no_direct_readiness_signal",
                        "elapsed_s": round(time.monotonic() - start, 3),
                        "poll_attempts": attempts,
                        "readiness_signal": signal_source,
                        "fallback_sleep_s": fallback_sleep_s,
                        "poll_snapshots": snapshots,
                        "status_errors": status_errors,
                    }
            else:
                no_signal_count = 0

        except Exception as exc:  # noqa: BLE001
            status_errors.append(str(exc))
            if len(status_errors) >= max(1, max_status_errors_before_fallback):
                if fallback_sleep_s > 0:
                    time.sleep(fallback_sleep_s)
                return {
                    "status": "fallback_after_status_errors",
                    "reason": "status_poll_error_threshold",
                    "elapsed_s": round(time.monotonic() - start, 3),
                    "poll_attempts": attempts,
                    "readiness_signal": "error",
                    "fallback_sleep_s": fallback_sleep_s,
                    "poll_snapshots": snapshots,
                    "status_errors": status_errors,
                }

        if poll_interval_s > 0:
            time.sleep(poll_interval_s)

    return {
        "status": "timeout",
        "reason": "readiness_timeout",
        "elapsed_s": round(time.monotonic() - start, 3),
        "poll_attempts": attempts,
        "readiness_signal": "unready_or_ambiguous",
        "readiness_timeout_s": readiness_timeout_s,
        "fallback_sleep_s": fallback_sleep_s,
        "poll_snapshots": snapshots,
        "status_errors": status_errors,
    }


def wait_for_embedding_ready(
    client: McpClientLike,
    *,
    readiness_timeout_s: float = 600.0,
    poll_interval_s: float = 2.0,
    timeout_per_call_s: float = 30.0,
) -> dict[str, Any]:
    start = time.monotonic()
    snapshots: list[dict[str, Any]] = []
    status_errors: list[str] = []
    reason_codes: list[str] = []

    deadline = start + max(0.0, readiness_timeout_s)
    while time.monotonic() < deadline:
        try:
            decoded = _decode_tool_response(client.call_tool("get_status", {}, timeout=timeout_per_call_s))
            payload = decoded["payload"]
            result = decoded["result"]
            status_value = str(payload.get("status", "")).strip().lower()
            embedding_raw = payload.get("embedding")
            embedding = embedding_raw if isinstance(embedding_raw, dict) else {}
            embedding_status = str(embedding.get("status", "")).strip().lower()
            reason_code = _extract_summary_partial_reason_code(payload, result)
            if isinstance(reason_code, str) and reason_code:
                reason_codes.append(reason_code)

            snapshots.append(
                {
                    "status": status_value,
                    "embedding_status": embedding_status,
                    "embedding_phase": embedding.get("phase"),
                    "embedding_progress_percent": embedding.get("progress_percent"),
                    "parse_errors": decoded.get("parse_errors", []),
                    "reason_code": reason_code,
                }
            )

            if status_value in {"ready", "ok", "healthy"} or embedding_status == "ready":
                return {
                    "status": "ready",
                    "reason": "embedding_ready",
                    "elapsed_s": round(time.monotonic() - start, 3),
                    "poll_attempts": len(snapshots),
                    "reason_codes": sorted(set(reason_codes)),
                    "poll_snapshots": snapshots,
                    "status_errors": status_errors,
                }
        except Exception as exc:  # noqa: BLE001
            status_errors.append(str(exc))

        if poll_interval_s > 0:
            time.sleep(poll_interval_s)

    return {
        "status": "timeout",
        "reason": "embedding_readiness_timeout",
        "elapsed_s": round(time.monotonic() - start, 3),
        "poll_attempts": len(snapshots),
        "reason_codes": sorted(set(reason_codes)),
        "poll_snapshots": snapshots,
        "status_errors": status_errors,
        "readiness_timeout_s": readiness_timeout_s,
    }


def seed_graph_fixture(
    client: McpClientLike,
    graph_fixture: dict[str, list[dict[str, Any]]],
    *,
    timeout_s: float = 30.0,
) -> dict[str, Any]:
    entities = graph_fixture["entities"]
    relations = graph_fixture["relations"]

    entity_ids: list[str] = []
    relation_ids: list[str] = []
    call_log: list[dict[str, Any]] = []

    for entity in entities:
        args = {
            "action": "create_entity",
            "name": entity["name"],
            "entity_type": entity.get("entity_type"),
            "description": entity.get("description"),
        }
        decoded = _decode_tool_response(client.call_tool("knowledge_graph", args, timeout=timeout_s))
        payload = decoded["payload"]
        entity_id = str(payload.get("entity_id") or payload.get("id") or entity.get("id") or entity["name"])
        entity_ids.append(entity_id)
        call_log.append({"kind": "entity", "name": entity.get("name"), "entity_id": entity_id, "parse_errors": decoded["parse_errors"]})

    for relation in relations:
        args = {
            "action": "create_relation",
            "from_entity": relation["from_entity"],
            "to_entity": relation["to_entity"],
            "relation_type": relation["relation_type"],
            "weight": relation.get("weight"),
        }
        decoded = _decode_tool_response(client.call_tool("knowledge_graph", args, timeout=timeout_s))
        payload = decoded["payload"]
        relation_id = str(
            payload.get("relation_id")
            or payload.get("id")
            or f"{relation['from_entity']}->{relation['relation_type']}->{relation['to_entity']}"
        )
        relation_ids.append(relation_id)
        call_log.append(
            {
                "kind": "relation",
                "from_entity": relation.get("from_entity"),
                "to_entity": relation.get("to_entity"),
                "relation_type": relation.get("relation_type"),
                "relation_id": relation_id,
                "parse_errors": decoded["parse_errors"],
            }
        )

    return {
        "entity_ids": entity_ids,
        "relation_ids": relation_ids,
        "entity_count": len(entity_ids),
        "relation_count": len(relation_ids),
        "calls": call_log,
    }


def seed_memory_corpus(
    client: McpClientLike,
    memories: Sequence[dict[str, Any]],
    *,
    timeout_s: float = 30.0,
) -> dict[str, Any]:
    memory_ids: list[str] = []
    call_log: list[dict[str, Any]] = []
    for memory in memories:
        decoded = _decode_tool_response(client.call_tool("store_memory", dict(memory), timeout=timeout_s))
        payload = decoded["payload"]
        memory_id = str(payload.get("id") or memory.get("id"))
        memory_ids.append(memory_id)
        call_log.append(
            {
                "memory_id": memory_id,
                "memory_type": memory.get("memory_type"),
                "namespace": memory.get("namespace"),
                "user_id": memory.get("user_id"),
                "agent_id": memory.get("agent_id"),
                "run_id": memory.get("run_id"),
                "parse_errors": decoded["parse_errors"],
            }
        )
    return {"memory_ids": memory_ids, "memory_count": len(memory_ids), "calls": call_log}


def seed_memory_fixtures(
    client: McpClientLike,
    *,
    corpus_path: Path = FIXTURE_CORPUS_PATH,
    graph_path: Path = FIXTURE_GRAPH_PATH,
    golden_path: Path = GOLDEN_QUERIES_PATH,
    call_timeout_s: float = 30.0,
    readiness_timeout_s: float = 180.0,
    readiness_poll_interval_s: float = 2.0,
    readiness_fallback_sleep_s: float = 3.0,
) -> dict[str, Any]:
    progress: dict[str, Any] = {
        "status": "in_progress",
        "started_at": _utc_now(),
        "fixture_paths": {
            "memory_corpus": str(corpus_path),
            "memory_graph": str(graph_path),
            "golden_queries": str(golden_path),
        },
        "seed_order": ["knowledge_graph", "store_memory"],
        "phases": [],
        "blockers": [],
    }

    phase_load = _new_phase("load_and_validate_fixtures")
    progress["phases"].append(phase_load)
    try:
        memories = load_memory_corpus(corpus_path)
        graph_fixture = load_graph_fixture(graph_path)
        queries = load_golden_queries(golden_path)
        validation = _validate_fixture_caps(memories=memories, graph_fixture=graph_fixture, queries=queries)
        _complete_phase(phase_load, validation=validation)
    except Exception as exc:  # noqa: BLE001
        _block_phase(phase_load, str(exc), error_type=type(exc).__name__)
        progress["status"] = "blocked"
        progress["blockers"].append(
            {"phase": phase_load["phase"], "error_type": type(exc).__name__, "message": str(exc)}
        )
        progress["finished_at"] = _utc_now()
        return progress

    phase_graph = _new_phase("seed_graph")
    progress["phases"].append(phase_graph)
    try:
        graph_seed = seed_graph_fixture(client, graph_fixture, timeout_s=call_timeout_s)
        progress["graph"] = graph_seed
        _complete_phase(phase_graph, entity_count=graph_seed["entity_count"], relation_count=graph_seed["relation_count"])
    except Exception as exc:  # noqa: BLE001
        _block_phase(phase_graph, str(exc), error_type=type(exc).__name__)
        progress["status"] = "blocked"
        progress["blockers"].append(
            {"phase": phase_graph["phase"], "error_type": type(exc).__name__, "message": str(exc)}
        )
        progress["finished_at"] = _utc_now()
        return progress

    phase_memory = _new_phase("seed_memories")
    progress["phases"].append(phase_memory)
    try:
        memory_seed = seed_memory_corpus(client, memories, timeout_s=call_timeout_s)
        progress["memory"] = memory_seed
        progress["query_count"] = len(queries)
        _complete_phase(phase_memory, memory_count=memory_seed["memory_count"])
    except Exception as exc:  # noqa: BLE001
        _block_phase(phase_memory, str(exc), error_type=type(exc).__name__)
        progress["status"] = "blocked"
        progress["blockers"].append(
            {"phase": phase_memory["phase"], "error_type": type(exc).__name__, "message": str(exc)}
        )
        progress["finished_at"] = _utc_now()
        return progress

    phase_ready = _new_phase("settle_readiness")
    progress["phases"].append(phase_ready)
    try:
        settle = settle_after_seeding(
            client,
            readiness_timeout_s=readiness_timeout_s,
            poll_interval_s=readiness_poll_interval_s,
            fallback_sleep_s=readiness_fallback_sleep_s,
            timeout_per_call_s=call_timeout_s,
        )
        progress["settle"] = settle
        _complete_phase(phase_ready, settle_status=settle.get("status"), elapsed_s=settle.get("elapsed_s"))
    except Exception as exc:  # noqa: BLE001
        _block_phase(phase_ready, str(exc), error_type=type(exc).__name__)
        progress["status"] = "blocked"
        progress["blockers"].append(
            {"phase": phase_ready["phase"], "error_type": type(exc).__name__, "message": str(exc)}
        )
        progress["finished_at"] = _utc_now()
        return progress

    progress["status"] = "completed"
    progress["finished_at"] = _utc_now()
    return progress


def _safe_list(value: Any) -> list[Any]:
    return value if isinstance(value, list) else []


def _decode_tool_response(response: dict[str, Any]) -> dict[str, Any]:
    result = response.get("result", {}) if isinstance(response, dict) else {}
    if not isinstance(result, dict):
        result = {}

    content = _safe_list(result.get("content"))
    parse_errors: list[str] = []
    parsed_payload: dict[str, Any] | None = None

    for index, chunk in enumerate(content):
        if not isinstance(chunk, dict):
            continue
        text = chunk.get("text")
        if not isinstance(text, str):
            continue
        try:
            decoded = json.loads(text)
        except json.JSONDecodeError as exc:
            parse_errors.append(f"content[{index}] JSON decode failed: {exc}")
            continue
        if isinstance(decoded, dict):
            parsed_payload = decoded
            break
        parse_errors.append(f"content[{index}] parsed non-object payload: {type(decoded).__name__}")

    payload = parsed_payload or result
    if not isinstance(payload, dict):
        payload = {}

    return {
        "payload": payload,
        "result": result,
        "parse_errors": parse_errors,
    }


def _extract_summary_partial_reason_code(payload: dict[str, Any], result: dict[str, Any]) -> str | None:
    for source in (payload, result):
        summary = source.get("summary")
        if not isinstance(summary, dict):
            continue
        partial = summary.get("partial")
        if not isinstance(partial, dict):
            continue
        reason_code = partial.get("reason_code")
        if isinstance(reason_code, str) and reason_code:
            return reason_code
    return None


def _extract_contract(payload: dict[str, Any], result: dict[str, Any]) -> dict[str, Any] | None:
    for source in (payload, result):
        contract = source.get("contract")
        if isinstance(contract, dict):
            return contract
    return None


def _extract_warnings(payload: dict[str, Any], result: dict[str, Any]) -> list[Any]:
    for source in (payload, result):
        warnings = source.get("warnings")
        if isinstance(warnings, list):
            return list(warnings)
    return []


def _normalize_result_items(payload: dict[str, Any]) -> list[dict[str, Any]]:
    candidate_lists: list[list[Any]] = []
    for key in ("results", "memories", "items", "hits"):
        value = payload.get(key)
        if isinstance(value, list):
            candidate_lists.append(value)

    data = payload.get("data")
    if isinstance(data, dict):
        for key in ("results", "memories", "items", "hits"):
            value = data.get(key)
            if isinstance(value, list):
                candidate_lists.append(value)

    if not candidate_lists:
        for key in ("memory", "item"):
            value = payload.get(key)
            if isinstance(value, dict):
                candidate_lists.append([value])

    normalized: list[dict[str, Any]] = []
    for items in candidate_lists:
        for item in items:
            if isinstance(item, dict):
                normalized.append(item)
            elif item is not None:
                normalized.append({"id": item})
    return normalized


def _extract_item_id(item: dict[str, Any]) -> str | None:
    direct_keys = ("id", "memory_id", "entity_id")
    for key in direct_keys:
        value = item.get(key)
        if value is not None:
            return str(value)

    nested_keys = ("memory", "record", "item", "node")
    for key in nested_keys:
        nested = item.get(key)
        if isinstance(nested, dict):
            for nested_key in direct_keys:
                value = nested.get(nested_key)
                if value is not None:
                    return str(value)
    return None


def _normalize_result_ids(payload: dict[str, Any]) -> list[str]:
    result_ids: list[str] = []
    seen: set[str] = set()
    for item in _normalize_result_items(payload):
        result_id = _extract_item_id(item)
        if not result_id or result_id in seen:
            continue
        seen.add(result_id)
        result_ids.append(result_id)
    return result_ids


def _build_tool_call(query: dict[str, Any]) -> tuple[str, dict[str, Any]]:
    tool = str(query.get("tool") or "")
    query_type = str(query.get("query_type") or "")
    query_text = str(query.get("query") or "")
    mode = query.get("mode")
    filters = query.get("filters")

    if query_type == "recall_fusion":
        args: dict[str, Any] = {"query": query_text}
        if isinstance(filters, dict) and filters:
            args.update(filters)
        return "recall", args

    if query_type in {"search_vector", "search_bm25"}:
        args = {"query": query_text, "mode": "vector" if query_type == "search_vector" else "bm25"}
        if isinstance(filters, dict) and filters:
            args.update(filters)
        return "search_memory", args

    if query_type in {"get_valid_temporal", "get_valid_filtered"}:
        args = dict(filters) if isinstance(filters, dict) else {}
        return "get_valid", args

    if query_type == "negative_no_match":
        if tool == "search_memory":
            args = {"query": query_text, "mode": "bm25" if mode == "bm25" else "vector" if mode == "vector" else "bm25"}
            if isinstance(filters, dict) and filters:
                args.update(filters)
            return "search_memory", args
        args = {"query": query_text}
        if isinstance(filters, dict) and filters:
            args.update(filters)
        return "recall", args

    raise ValueError(f"Unsupported query_type: {query_type}")


def execute_query(client: McpClientLike, query: dict[str, Any]) -> dict[str, Any]:
    query_id = str(query.get("id"))
    query_type = str(query.get("query_type") or "")
    expected_ids = [str(value) for value in query.get("expected_ids", [])]
    negative = query_type == "negative_no_match" or not expected_ids

    tool_name, arguments = _build_tool_call(query)
    started = time.perf_counter()
    call_error: str | None = None
    parse_errors: list[str] = []
    payload: dict[str, Any] = {}
    result: dict[str, Any] = {}
    warnings: list[Any] = []
    summary_partial_reason_code: str | None = None
    contract: dict[str, Any] | None = None
    result_ids: list[str] = []

    try:
        raw_response = client.call_tool(tool_name, arguments)
        decoded = _decode_tool_response(raw_response)
        payload = decoded["payload"]
        result = decoded["result"]
        parse_errors = decoded["parse_errors"]
        result_ids = _normalize_result_ids(payload)
        summary_partial_reason_code = _extract_summary_partial_reason_code(payload, result)
        warnings = _extract_warnings(payload, result)
        contract = _extract_contract(payload, result)
    except Exception as exc:  # noqa: BLE001
        call_error = str(exc)

    latency_ms = (time.perf_counter() - started) * 1000.0
    expected_set = set(expected_ids)
    found_expected_ids = [result_id for result_id in result_ids if result_id in expected_set]
    metric_row = compute_query_metrics(
        result_ids,
        expected_ids,
        query_type=query_type,
        latency_ms=latency_ms,
        negative=negative,
    )

    row: dict[str, Any] = {
        **metric_row,
        "query_id": query_id,
        "query": query.get("query"),
        "tool": tool_name,
        "tool_arguments": arguments,
        "expected_ids": expected_ids,
        "found_expected_ids": found_expected_ids,
        "result_ids": result_ids,
        "result_count": len(result_ids),
        "summary_partial_reason_code": summary_partial_reason_code,
        "warnings": warnings,
        "contract": contract,
        "parse_errors": parse_errors,
    }
    if call_error is not None:
        row["call_error"] = call_error
    return row


def execute_queries(client: McpClientLike, queries: Sequence[dict[str, Any]]) -> dict[str, Any]:
    per_query = [execute_query(client, query) for query in queries]
    aggregate = aggregate_metrics(per_query)
    diagnostics = {
        "degraded_or_partial_count": sum(1 for row in per_query if row.get("summary_partial_reason_code")),
        "parse_issue_count": sum(1 for row in per_query if row.get("parse_errors")),
        "call_error_count": sum(1 for row in per_query if row.get("call_error")),
    }
    return {
        "aggregate_metrics": aggregate,
        "per_query": per_query,
        "diagnostics": diagnostics,
    }


class _SyntheticNegativeClient:
    def __init__(self, fake_result_ids: Sequence[str]):
        self._fake_result_ids = [str(value) for value in fake_result_ids]

    def call_tool(self, name: str, arguments: dict[str, Any] | None = None, timeout: float = 30.0) -> dict[str, Any]:
        payload = {
            "results": [{"id": result_id} for result_id in self._fake_result_ids],
            "warnings": ["synthetic-negative-query-evidence"],
            "contract": {"source": "synthetic-negative-client"},
        }
        return {
            "result": {
                "content": [{"type": "text", "text": json.dumps(payload)}],
            }
        }


def negative_query_metrics_snippet(queries: Sequence[dict[str, Any]]) -> dict[str, Any]:
    synthetic_client = _SyntheticNegativeClient(["fake_mem_negative_1", "fake_mem_negative_2"])
    rows: list[dict[str, Any]] = []
    for query in queries:
        if str(query.get("query_type")) != "negative_no_match":
            continue
        rows.append(execute_query(synthetic_client, dict(query)))
    return {
        "query_type": "negative_no_match",
        "query_count": len(rows),
        "aggregate_metrics": aggregate_metrics(rows),
        "per_query": rows,
    }


def write_negative_query_metrics_snippet(path: Path, queries: Sequence[dict[str, Any]]) -> dict[str, Any]:
    snippet = negative_query_metrics_snippet(queries)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(snippet, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    return snippet


def _fixture_manifest(
    *,
    memories: Sequence[dict[str, Any]] | None = None,
    graph_fixture: dict[str, list[dict[str, Any]]] | None = None,
    queries: Sequence[dict[str, Any]] | None = None,
    command: Sequence[str] | None = None,
) -> dict[str, Any]:
    memories = memories if memories is not None else load_memory_corpus()
    graph_fixture = graph_fixture if graph_fixture is not None else load_graph_fixture()
    queries = queries if queries is not None else load_golden_queries()
    query_types = sorted({str(query.get("query_type")) for query in queries})
    return {
        "fixture_paths": {
            "memory_corpus": str(FIXTURE_CORPUS_PATH.relative_to(ROOT)),
            "memory_graph": str(FIXTURE_GRAPH_PATH.relative_to(ROOT)),
            "golden_queries": str(GOLDEN_QUERIES_PATH.relative_to(ROOT)),
        },
        "fixture_counts": {
            "memories": len(memories),
            "graph_entities": len(graph_fixture["entities"]),
            "graph_relations": len(graph_fixture["relations"]),
            "golden_queries": len(queries),
        },
        "query_types": query_types,
        "required_query_types": sorted(REQUIRED_QUERY_TYPES),
        "tool_set": ["knowledge_graph", "store_memory", "recall", "search_memory", "get_valid"],
        "seed_order": ["knowledge_graph", "store_memory"],
        "command_selected": list(command) if command is not None else None,
        "data_dir_strategy": "temporary isolated DATA_DIR",
    }


def _environment(
    *,
    command: Sequence[str],
    data_dir: str | None,
    embedding_model: str,
    started_at: float,
    mode: str,
    raw: dict[str, Any] | None = None,
    stderr_tail: Sequence[str] | None = None,
) -> dict[str, Any]:
    environment: dict[str, Any] = {
        "root": str(ROOT),
        "server_command": list(command),
        "embedding_model": embedding_model,
        "data_dir_strategy": "temporary isolated DATA_DIR" if data_dir else "not used",
        "data_dir": data_dir,
        "started_at_utc": datetime.fromtimestamp(started_at, tz=timezone.utc).isoformat(),
        "duration_seconds": round(time.time() - started_at, 2),
        "mode": mode,
    }
    if raw:
        environment["raw"] = raw
    if stderr_tail:
        environment["stderr_tail"] = [str(line) for line in stderr_tail if str(line).strip()]
    return environment


def _blocker(
    *,
    phase: str,
    command_or_tool: str,
    message: str,
    stderr_tail: Sequence[str] | None = None,
    reason_code: str | None = None,
    diagnostics: dict[str, Any] | None = None,
) -> dict[str, Any]:
    blocker = {
        "phase": phase,
        "command_or_tool": command_or_tool,
        "message": message,
        "summary_partial_reason_code": reason_code,
        "stderr_tail": [str(line) for line in (stderr_tail or []) if str(line).strip()],
    }
    if diagnostics:
        blocker["diagnostics"] = diagnostics
    return blocker


def _reason_codes_from_rows(per_query: Sequence[dict[str, Any]]) -> list[str]:
    codes = {
        str(row.get("summary_partial_reason_code"))
        for row in per_query
        if isinstance(row.get("summary_partial_reason_code"), str) and row.get("summary_partial_reason_code")
    }
    return sorted(codes)


def _positive_query_aggregate_metrics(per_query: Sequence[dict[str, Any]]) -> dict[str, Any]:
    positive_rows = [
        row
        for row in per_query
        if isinstance(row.get("expected_ids"), list) and len(row.get("expected_ids", [])) > 0
    ]

    def _mean_metric(metric_key: str) -> float | None:
        values = [
            float(value)
            for row in positive_rows
            for value in [row.get(metric_key)]
            if isinstance(value, (int, float))
        ]
        if not values:
            return None
        return sum(values) / len(values)

    return {
        "positive_query_count": len(positive_rows),
        "positive_mean_mrr": _mean_metric("mrr"),
        "positive_mean_precision_at_5": _mean_metric("precision_at_5"),
        "positive_mean_precision_at_10": _mean_metric("precision_at_10"),
    }


def _write_reports(
    *,
    output_json: Path,
    output_md: Path,
    manifest: dict[str, Any],
    aggregate: dict[str, Any],
    per_query: Sequence[dict[str, Any]],
    warnings: Sequence[Any],
    blockers: Sequence[Any],
    environment: dict[str, Any],
    stderr_tail: Sequence[Any] | None = None,
) -> None:
    write_json_report(
        output_json,
        BENCHMARK_NAME,
        manifest,
        aggregate,
        per_query,
        warnings=warnings,
        blockers=blockers,
        stderr_tail=stderr_tail,
        environment=environment,
    )
    write_markdown_report(
        output_md,
        "Memory Retrieval Baseline",
        aggregate,
        per_query,
        manifest=manifest,
        environment=environment,
        stderr_tail=stderr_tail,
        warnings=warnings,
        blockers=blockers,
    )


def _validate_report_payload(path: Path) -> dict[str, Any]:
    payload = json.loads(path.read_text(encoding="utf-8"))
    required_keys = {
        "generated_at_utc",
        "benchmark_name",
        "manifest",
        "aggregate_metrics",
        "per_query",
        "warnings",
        "blockers",
        "stderr_tail",
        "environment",
    }
    missing = sorted(required_keys - set(payload))
    if missing:
        raise AssertionError(f"Report missing required keys: {missing}")
    if not isinstance(payload["per_query"], list):
        raise AssertionError("Report per_query must be a list")
    if not isinstance(payload["warnings"], list):
        raise AssertionError("Report warnings must be a list")
    if not isinstance(payload["blockers"], list):
        raise AssertionError("Report blockers must be a list")
    if not isinstance(payload["stderr_tail"], list):
        raise AssertionError("Report stderr_tail must be a list")
    return payload


def run_full_self_test() -> int:
    memories = load_memory_corpus()
    graph_fixture = load_graph_fixture()
    queries = load_golden_queries()
    fixture_summary = _validate_fixture_caps(memories=memories, graph_fixture=graph_fixture, queries=queries)
    query_summary = _validate_query_caps()
    negative_snippet = negative_query_metrics_snippet(queries)

    sample_rows: list[dict[str, Any]] = []
    for index, query in enumerate(queries, start=1):
        expected_ids = [str(value) for value in query.get("expected_ids", [])]
        if expected_ids:
            mock_result_ids = [expected_ids[0], "synthetic_non_expected_id"]
        else:
            mock_result_ids = ["synthetic_no_match_id"]
        sample_rows.append(
            {
                "query_id": query.get("id", f"query_{index}"),
                "query": query.get("query"),
                **compute_query_metrics(
                    mock_result_ids,
                    expected_ids,
                    query_type=str(query.get("query_type")),
                    latency_ms=10.0 + index,
                    negative=str(query.get("query_type")) == "negative_no_match" or not expected_ids,
                ),
            }
        )

    aggregate = aggregate_metrics(sample_rows)
    with tempfile.TemporaryDirectory(prefix="task-8-memory-self-test-") as tmp:
        tmp_path = Path(tmp)
        json_path = tmp_path / "self-test.json"
        md_path = tmp_path / "self-test.md"
        manifest = _fixture_manifest(memories=memories, graph_fixture=graph_fixture, queries=queries, command=["offline-self-test"])
        _write_reports(
            output_json=json_path,
            output_md=md_path,
            manifest=manifest,
            aggregate=aggregate,
            per_query=sample_rows,
            warnings=["offline self-test"],
            blockers=[],
            environment={"mode": "self-test", "data_dir_strategy": "not used"},
            stderr_tail=[],
        )
        parsed = _validate_report_payload(json_path)
        if parsed.get("aggregate_metrics", {}).get("query_count") != len(sample_rows):
            raise AssertionError("self-test JSON report query_count mismatch")
        if "Memory Retrieval Baseline" not in md_path.read_text(encoding="utf-8"):
            raise AssertionError("self-test markdown title missing")

    print(
        "self-test passed "
        f"(fixtures={fixture_summary['memory_count']} memories, queries={query_summary['query_count']}, "
        f"negative_queries={negative_snippet['query_count']})"
    )
    print(
        json.dumps(
            {
                "fixture_summary": fixture_summary,
                "query_summary": query_summary,
                "negative_query_summary": {
                    "query_count": negative_snippet["query_count"],
                    "aggregate_metrics": negative_snippet["aggregate_metrics"],
                },
                "report_generation": "validated",
            },
            sort_keys=True,
        )
    )
    return 0


def _validate_fixture_caps(
    *,
    memories: list[dict[str, Any]] | None = None,
    graph_fixture: dict[str, list[dict[str, Any]]] | None = None,
    queries: list[dict[str, Any]] | None = None,
) -> dict[str, Any]:
    memories = memories if memories is not None else load_memory_corpus()
    graph_fixture = graph_fixture if graph_fixture is not None else load_graph_fixture()
    queries = queries if queries is not None else load_golden_queries()

    entities = graph_fixture["entities"]
    relations = graph_fixture["relations"]

    memory_count = len(memories)
    if memory_count < MEMORY_CORPUS_MIN_COUNT or memory_count > MEMORY_CORPUS_MAX_COUNT:
        raise AssertionError(
            f"Memory fixture count out of V1 cap: {memory_count} (expected {MEMORY_CORPUS_MIN_COUNT}-{MEMORY_CORPUS_MAX_COUNT})"
        )
    if len(entities) > GRAPH_ENTITY_MAX_COUNT:
        raise AssertionError(f"Graph entities exceed V1 cap: {len(entities)} > {GRAPH_ENTITY_MAX_COUNT}")
    if len(relations) > GRAPH_RELATION_MAX_COUNT:
        raise AssertionError(f"Graph relations exceed V1 cap: {len(relations)} > {GRAPH_RELATION_MAX_COUNT}")
    if len(queries) != EXPECTED_GOLDEN_QUERY_COUNT:
        raise AssertionError(f"Expected {EXPECTED_GOLDEN_QUERY_COUNT} golden queries, found {len(queries)}")

    memory_ids = [str(memory["id"]) for memory in memories]
    if len(memory_ids) != len(set(memory_ids)):
        raise AssertionError("Memory fixture IDs must be unique")

    entity_id_set = {str(entity["id"]) for entity in entities}
    query_ids = [str(query["id"]) for query in queries]

    if len(query_ids) != len(set(query_ids)):
        raise AssertionError("Golden query IDs must be unique")

    for relation in relations:
        from_entity = str(relation["from_entity"])
        to_entity = str(relation["to_entity"])
        if from_entity not in entity_id_set or to_entity not in entity_id_set:
            raise AssertionError(f"Relation references unknown entity: {from_entity} -> {to_entity}")

    memory_id_set = set(memory_ids)
    expected_total = 0
    for query in queries:
        expected_ids = [str(memory_id) for memory_id in query.get("expected_ids", [])]
        expected_total += len(expected_ids)
        for memory_id in expected_ids:
            if memory_id not in memory_id_set:
                raise AssertionError(f"Golden query references unknown memory id: {memory_id}")

    return {
        "caps": {
            "memory_count_min": MEMORY_CORPUS_MIN_COUNT,
            "memory_count_max": MEMORY_CORPUS_MAX_COUNT,
            "graph_entity_max": GRAPH_ENTITY_MAX_COUNT,
            "graph_relation_max": GRAPH_RELATION_MAX_COUNT,
        },
        "memory_count": memory_count,
        "graph_entity_count": len(entities),
        "graph_relation_count": len(relations),
        "query_count": len(queries),
        "query_expected_id_total": expected_total,
        "query_unique_ids": len(set(query_ids)),
        "memory_unique_ids": len(memory_id_set),
    }


def _validate_query_caps() -> dict[str, Any]:
    queries = load_golden_queries()
    observed_query_types = {str(query.get("query_type")) for query in queries}
    missing_query_types = sorted(REQUIRED_QUERY_TYPES - observed_query_types)
    if missing_query_types:
        raise AssertionError(f"Golden queries missing required query types: {missing_query_types}")

    supported_tools = {"recall", "search_memory", "get_valid"}
    invalid_tools: list[str] = []
    for query in queries:
        tool = str(query.get("tool") or "")
        if tool not in supported_tools:
            invalid_tools.append(f"{query.get('id')}: {tool}")
    if invalid_tools:
        raise AssertionError(f"Golden queries use unsupported tools: {invalid_tools}")

    return {
        "query_count": len(queries),
        "required_query_types": sorted(REQUIRED_QUERY_TYPES),
        "observed_query_types": sorted(observed_query_types),
    }


def self_test_fixtures() -> dict[str, Any]:
    summary = _validate_fixture_caps()
    print(
        "fixtures self-test passed "
        f"(memory_count={summary['memory_count']}, graph_entity_count={summary['graph_entity_count']}, "
        f"graph_relation_count={summary['graph_relation_count']}, query_count={summary['query_count']})"
    )
    print(json.dumps(summary, sort_keys=True))
    return summary


def self_test_queries() -> dict[str, Any]:
    summary = _validate_query_caps()
    print(
        "queries self-test passed "
        f"(query_count={summary['query_count']}, required_query_types={summary['required_query_types']})"
    )
    print(json.dumps(summary, sort_keys=True))
    return summary


def run_queries(args: argparse.Namespace) -> int:
    queries = load_golden_queries()
    with McpClient.start(timeout=args.timeout, client_name="memory-retrieval-benchmark", client_version="0.1.0") as client:
        result = execute_queries(client, queries)
    if args.output:
        output_path = Path(args.output)
        output_path.parent.mkdir(parents=True, exist_ok=True)
        output_path.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps({"query_count": len(result["per_query"]), **result["diagnostics"]}, sort_keys=True))
    return 0


def run_benchmark(args: argparse.Namespace) -> int:
    EVIDENCE_DIR.mkdir(parents=True, exist_ok=True)
    run_started = time.time()
    resolved_command = resolve_mcp_command()
    warnings: list[Any] = []
    blockers: list[dict[str, Any]] = []
    per_query: list[dict[str, Any]] = []
    raw: dict[str, Any] = {}
    stderr_tail: list[str] = []

    try:
        memories = load_memory_corpus()
        graph_fixture = load_graph_fixture()
        queries = load_golden_queries()
        validation = _validate_fixture_caps(memories=memories, graph_fixture=graph_fixture, queries=queries)
        query_validation = _validate_query_caps()
        raw["fixture_validation"] = validation
        raw["query_validation"] = query_validation
    except Exception as exc:  # noqa: BLE001
        memories = []
        graph_fixture = {"entities": [], "relations": []}
        queries = []
        blockers.append(
            _blocker(
                phase="load_and_validate_fixtures",
                command_or_tool="fixture_loaders",
                message=str(exc),
                reason_code=None,
                diagnostics={"error_type": type(exc).__name__},
            )
        )

    data_dir_used: str | None = None
    if not blockers:
        with tempfile.TemporaryDirectory(prefix="task-8-memory-retrieval-data-") as data_dir:
            data_dir_used = data_dir
            env = build_env({"DATA_DIR": data_dir, "EMBEDDING_MODEL": args.embedding_model, "RUST_LOG": "warn"})
            try:
                with McpClient.start(
                    command=resolved_command,
                    root=ROOT,
                    env_overrides=env,
                    timeout=args.startup_timeout_s,
                    client_name="memory-retrieval-benchmark",
                    client_version="0.1.0",
                ) as client:
                    try:
                        startup_decoded = _decode_tool_response(client.call_tool("get_status", {}, timeout=args.timeout))
                        raw["startup_status"] = startup_decoded["payload"]
                    except Exception as exc:  # noqa: BLE001
                        warnings.append({"phase": "startup_status", "message": str(exc), "error_type": type(exc).__name__})

                    embedding_readiness = wait_for_embedding_ready(
                        client,
                        readiness_timeout_s=args.embedding_ready_timeout_s,
                        poll_interval_s=args.poll_interval_s,
                        timeout_per_call_s=min(args.timeout, 60.0),
                    )
                    raw["embedding_readiness"] = embedding_readiness

                    if embedding_readiness.get("status") != "ready":
                        blockers.append(
                            _blocker(
                                phase="embedding_readiness",
                                command_or_tool="get_status",
                                message=(
                                    f"embedding readiness did not complete within "
                                    f"{args.embedding_ready_timeout_s:.1f}s"
                                ),
                                stderr_tail=client.stderr_tail(80),
                                reason_code=(embedding_readiness.get("reason_codes") or [None])[0],
                                diagnostics=embedding_readiness,
                            )
                        )
                        seed_progress = {"status": "skipped", "reason": "embedding_not_ready"}
                    else:
                        seed_progress = seed_memory_fixtures(
                            client,
                            call_timeout_s=max(args.timeout, 120.0),
                            readiness_timeout_s=args.readiness_timeout_s,
                            readiness_poll_interval_s=args.poll_interval_s,
                            readiness_fallback_sleep_s=args.readiness_fallback_sleep_s,
                        )
                    raw["seed_progress"] = seed_progress
                    if seed_progress.get("status") != "completed":
                        blockers.extend(
                            _blocker(
                                phase=str(blocker.get("phase") or "seed_memory_fixtures"),
                                command_or_tool="seed_memory_fixtures",
                                message=str(blocker.get("message") or blocker.get("error") or "fixture seeding blocked"),
                                stderr_tail=client.stderr_tail(80),
                                reason_code=None,
                                diagnostics={key: value for key, value in blocker.items() if key not in {"phase", "message", "error"}},
                            )
                            for blocker in seed_progress.get("blockers", [])
                            if isinstance(blocker, dict)
                        )
                        if not blockers:
                            blockers.append(
                                _blocker(
                                    phase="seed_memory_fixtures",
                                    command_or_tool="seed_memory_fixtures",
                                    message="fixture seeding did not complete",
                                    stderr_tail=client.stderr_tail(80),
                                    diagnostics={"seed_status": seed_progress.get("status")},
                                )
                            )
                    else:
                        query_result = execute_queries(client, queries)
                        per_query = query_result["per_query"]
                        raw["query_diagnostics"] = query_result["diagnostics"]
                        if query_result["diagnostics"].get("call_error_count"):
                            blockers.append(
                                _blocker(
                                    phase="query_execution",
                                    command_or_tool="recall/search_memory/get_valid",
                                    message="one or more golden queries returned call_error",
                                    stderr_tail=client.stderr_tail(80),
                                    diagnostics=query_result["diagnostics"],
                                )
                            )
                    stderr_tail = client.stderr_tail(80)
            except Exception as exc:  # noqa: BLE001
                blockers.append(
                    _blocker(
                        phase="mcp_startup_or_runtime",
                        command_or_tool=" ".join(resolved_command),
                        message=str(exc),
                        stderr_tail=stderr_tail,
                        reason_code=None,
                        diagnostics={"error_type": type(exc).__name__},
                    )
                )

    aggregate = aggregate_metrics(per_query)
    aggregate.update(_positive_query_aggregate_metrics(per_query))
    aggregate["baseline_query_count"] = len(queries)
    aggregate["seed_completed"] = bool(raw.get("seed_progress", {}).get("status") == "completed")
    aggregate["observed_summary_partial_reason_codes"] = _reason_codes_from_rows(per_query)
    aggregate["blocker_count"] = len(blockers)

    if blockers and not per_query:
        warnings.append("benchmark blocked before query loop; structured blocker evidence written")

    manifest = _fixture_manifest(memories=memories, graph_fixture=graph_fixture, queries=queries, command=resolved_command)
    environment = _environment(
        command=resolved_command,
        data_dir=data_dir_used,
        embedding_model=args.embedding_model,
        started_at=run_started,
        mode="benchmark",
        raw=raw,
        stderr_tail=stderr_tail,
    )
    
    output_json = Path(args.output_json)
    output_md = Path(args.output_md)
    _write_reports(
        output_json=output_json,
        output_md=output_md,
        manifest=manifest,
        aggregate=aggregate,
        per_query=per_query,
        warnings=warnings,
        blockers=blockers,
        environment=environment,
        stderr_tail=stderr_tail,
    )

    print(
        json.dumps(
            {
                "output_json": str(output_json),
                "output_md": str(output_md),
                "query_count": len(queries),
                "ran_queries": len(per_query),
                "blocker_count": len(blockers),
                "observed_reason_codes": aggregate["observed_summary_partial_reason_codes"],
            },
            indent=2,
        )
    )
    return 0 if not blockers else 2


def parse_args(argv: Iterable[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Memory retrieval benchmark fixture + query execution utilities.")
    parser.add_argument("--self-test", action="store_true", help="Run built-in checks.")
    parser.add_argument("--phase", choices=("all", "fixtures", "queries"), default="all", help="Self-test phase selector.")
    parser.add_argument("--run", action="store_true", help="Execute all golden queries via MCP and print summary.")
    parser.add_argument("--timeout", type=float, default=120.0, help="MCP tool call timeout in seconds.")
    parser.add_argument("--startup-timeout-s", type=float, default=60.0, help="MCP initialize timeout in seconds.")
    parser.add_argument("--embedding-ready-timeout-s", type=float, default=600.0, help="Pre-seeding embedding readiness timeout in seconds.")
    parser.add_argument("--readiness-timeout-s", type=float, default=180.0, help="Post-seeding readiness timeout in seconds.")
    parser.add_argument("--readiness-fallback-sleep-s", type=float, default=3.0, help="Fallback settle sleep when readiness signal is unavailable.")
    parser.add_argument("--poll-interval-s", type=float, default=2.0, help="Readiness poll interval in seconds.")
    parser.add_argument("--embedding-model", default="e5_small", help="Embedding model for benchmark process env.")
    parser.add_argument("--output", type=Path, default=None, help="Optional JSON output path for --run results.")
    parser.add_argument("--output-json", type=Path, default=OUTPUT_JSON, help="Benchmark JSON evidence output path.")
    parser.add_argument("--output-md", type=Path, default=OUTPUT_MD, help="Benchmark Markdown evidence output path.")
    parser.add_argument(
        "--negative-metrics-snippet-out",
        type=Path,
        default=None,
        help="Write negative_no_match baseline metric snippet JSON for QA evidence.",
    )
    return parser.parse_args(list(argv) if argv is not None else None)


def main(argv: Iterable[str] | None = None) -> int:
    args = parse_args(argv)

    if args.self_test and args.phase == "all":
        return run_full_self_test()

    if args.self_test and args.phase == "fixtures":
        self_test_fixtures()
        return 0

    if args.self_test and args.phase == "queries":
        self_test_queries()
        return 0

    if args.negative_metrics_snippet_out is not None:
        queries = load_golden_queries()
        snippet = write_negative_query_metrics_snippet(args.negative_metrics_snippet_out, queries)
        print(
            "negative metrics snippet written "
            f"(path={args.negative_metrics_snippet_out}, query_count={snippet['query_count']})"
        )
        return 0

    if args.run:
        return run_queries(args)

    return run_benchmark(args)


if __name__ == "__main__":
    raise SystemExit(main())
