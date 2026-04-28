from __future__ import annotations

import argparse
import ast
import json
import re
import subprocess
import sys
import tempfile
import time
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Iterable, Mapping, Protocol, Sequence

PROJECT_ROOT = Path(__file__).resolve().parents[1]
if str(PROJECT_ROOT) not in sys.path:
    sys.path.insert(0, str(PROJECT_ROOT))

try:
    from evals.lib.mcp_client import McpClient, build_env, resolve_mcp_command
    from evals.lib.metrics import (
        aggregate_metrics,
        classify_readiness_fallback,
        classify_reason_code,
        classify_reason_codes,
        compute_query_metrics,
        write_json_report,
        write_markdown_report,
    )
except ModuleNotFoundError:  # pragma: no cover - supports `python3 evals/...` direct script invocation
    from lib.mcp_client import McpClient, build_env, resolve_mcp_command
    from lib.metrics import (
        aggregate_metrics,
        classify_readiness_fallback,
        classify_reason_code,
        classify_reason_codes,
        compute_query_metrics,
        write_json_report,
        write_markdown_report,
    )


HERE = Path(__file__).resolve().parent
ROOT = HERE.parent
FIXTURE_CORPUS_PATH = HERE / "fixtures" / "memory_corpus.json"
FIXTURE_GRAPH_PATH = HERE / "fixtures" / "memory_graph.json"
GOLDEN_QUERIES_PATH = HERE / "golden" / "memory_retrieval_queries.json"
MINI_FIXTURE_CORPUS_PATH = HERE / "fixtures" / "memory_corpus_mini_long_memory.json"
MINI_GOLDEN_QUERIES_PATH = HERE / "golden" / "memory_retrieval_queries_mini_long_memory.json"
MEDIUM_FIXTURE_CORPUS_PATH = HERE / "fixtures" / "memory_corpus_medium_long_memory.json"
MEDIUM_GOLDEN_QUERIES_PATH = HERE / "golden" / "memory_retrieval_queries_medium_long_memory.json"
STRESS_FIXTURE_MANIFEST_PATH = HERE / "fixtures" / "memory_corpus_stress_manifest.json"
STRESS_GOLDEN_MANIFEST_PATH = HERE / "golden" / "memory_retrieval_queries_stress_manifest.json"
EVIDENCE_DIR = ROOT / ".sisyphus" / "evidence" / "evals"
V2_EVIDENCE_DIR = ROOT / ".sisyphus" / "evidence" / "benchmark-v2"
OUTPUT_JSON = EVIDENCE_DIR / "memory-retrieval-baseline.json"
OUTPUT_MD = EVIDENCE_DIR / "memory-retrieval-baseline.md"
BENCHMARK_NAME = "memory_retrieval_baseline"
V2_SCHEMA_VERSION = "2.0"
V2_DEFAULT_FIXTURE_TIER = "small"
V2_DEFAULT_BASELINE_VERSION = "v2-initial"
V2_THRESHOLD_POLICY = "local-v2-threshold-policy"
V2_DETERMINISM_POLICY = "stable_fixture_order+stable_tie_break+stable_report_order+tolerance_1e-9_1e-6"
V2_VALID_FIXTURE_TIERS = ("small", "medium", "stress")

RUNTIME_TARGET_BY_TIER: dict[str, dict[str, Any]] = {
    "small": {
        "target_minutes": "5-10",
        "required_by_default": True,
        "optional_policy": "small tier default",
    },
    "medium": {
        "target_minutes": "15-30",
        "required_by_default": False,
        "optional_policy": "explicit medium-tier run",
    },
    "stress": {
        "target_minutes": "45-90+",
        "required_by_default": False,
        "optional_policy": "manual-only stress tier",
    },
}

EXPECTED_MEMORY_COUNT = 15
EXPECTED_GRAPH_ENTITY_COUNT = 5
EXPECTED_GRAPH_RELATION_COUNT = 8
EXPECTED_GOLDEN_QUERY_COUNT = 10
MEMORY_CORPUS_MIN_COUNT = 15
MEMORY_CORPUS_MAX_COUNT = 30
GRAPH_ENTITY_MAX_COUNT = 5
GRAPH_RELATION_MAX_COUNT = 8
FIXTURE_SCHEMA_VERSION = 1
MINI_MEMORY_CORPUS_MAX_COUNT = 30
MINI_GOLDEN_QUERY_MAX_COUNT = 20
MEDIUM_MEMORY_CORPUS_MAX_COUNT = 120
MEDIUM_GOLDEN_QUERY_MAX_COUNT = 80

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


def _git_commit_hash() -> str | None:
    try:
        result = subprocess.run(
            ["git", "rev-parse", "HEAD"],
            cwd=str(ROOT),
            check=False,
            capture_output=True,
            text=True,
        )
    except Exception:  # noqa: BLE001
        return None
    if result.returncode != 0:
        return None
    value = result.stdout.strip()
    return value or None


def _canonical_baseline_targets() -> tuple[Path, Path]:
    return OUTPUT_JSON.resolve(), OUTPUT_MD.resolve()


def _is_canonical_target_pair(output_json: Path, output_md: Path) -> bool:
    canonical_json, canonical_md = _canonical_baseline_targets()
    return output_json.resolve() == canonical_json and output_md.resolve() == canonical_md


def _non_refresh_report_paths(benchmark_name: str, fixture_tier: str) -> tuple[Path, Path]:
    timestamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    base = V2_EVIDENCE_DIR / "runs" / f"{benchmark_name}-{fixture_tier}-{timestamp}"
    return base.with_suffix(".json"), base.with_suffix(".md")


def _normalize_fixture_tier(value: str) -> str:
    tier = str(value or "").strip().lower()
    if tier in V2_VALID_FIXTURE_TIERS:
        return tier
    valid = ", ".join(V2_VALID_FIXTURE_TIERS)
    raise ValueError(f"Invalid fixture tier: {value!r}. Valid choices: {valid}")


def _runtime_target_for_tier(fixture_tier: str) -> dict[str, Any]:
    return dict(RUNTIME_TARGET_BY_TIER.get(fixture_tier, RUNTIME_TARGET_BY_TIER[V2_DEFAULT_FIXTURE_TIER]))


def _fixture_paths_for_tier(fixture_tier: str) -> dict[str, str]:
    tier = _normalize_fixture_tier(fixture_tier)
    if tier == "small":
        return {
            "memory_corpus": str(MINI_FIXTURE_CORPUS_PATH.relative_to(ROOT)),
            "memory_graph": str(FIXTURE_GRAPH_PATH.relative_to(ROOT)),
            "golden_queries": str(MINI_GOLDEN_QUERIES_PATH.relative_to(ROOT)),
        }
    if tier == "medium":
        return {
            "memory_corpus": str(MEDIUM_FIXTURE_CORPUS_PATH.relative_to(ROOT)),
            "memory_graph": str(FIXTURE_GRAPH_PATH.relative_to(ROOT)),
            "golden_queries": str(MEDIUM_GOLDEN_QUERIES_PATH.relative_to(ROOT)),
        }
    return {
        "memory_corpus_manifest": str(STRESS_FIXTURE_MANIFEST_PATH.relative_to(ROOT)),
        "memory_graph": str(FIXTURE_GRAPH_PATH.relative_to(ROOT)),
        "golden_queries_manifest": str(STRESS_GOLDEN_MANIFEST_PATH.relative_to(ROOT)),
    }


def _load_tier_inputs(
    fixture_tier: str,
) -> tuple[list[dict[str, Any]], dict[str, list[dict[str, Any]]], list[dict[str, Any]], dict[str, Any], dict[str, Any]]:
    tier = _normalize_fixture_tier(fixture_tier)
    graph_fixture = load_graph_fixture()
    if tier == "small":
        memories = load_memory_corpus(MINI_FIXTURE_CORPUS_PATH)
        queries = load_golden_queries(MINI_GOLDEN_QUERIES_PATH)
        validation = _validate_mini_long_memory_caps(memories=memories, queries=queries)
    elif tier == "medium":
        memories = load_memory_corpus(MEDIUM_FIXTURE_CORPUS_PATH)
        queries = load_golden_queries(MEDIUM_GOLDEN_QUERIES_PATH)
        validation = _validate_medium_long_memory_caps(memories=memories, queries=queries)
    else:
        memories = []
        queries = []
        validation = _validate_stress_long_memory_manifests()

    observed_query_types = sorted({str(query.get("query_type")) for query in queries})
    query_validation = {
        "fixture_tier": tier,
        "query_count": len(queries),
        "observed_query_types": observed_query_types,
    }
    return memories, graph_fixture, queries, validation, query_validation


def _self_test_fixture_tier_resolution() -> dict[str, Any]:
    resolved: dict[str, Any] = {}
    for tier in V2_VALID_FIXTURE_TIERS:
        fixture_paths = _fixture_paths_for_tier(tier)
        missing = [
            path
            for path in fixture_paths.values()
            if not (ROOT / path).exists()
        ]
        if missing:
            raise AssertionError(f"fixture tier {tier} has missing path mappings: {missing}")
        resolved[tier] = {
            "fixture_paths": fixture_paths,
            "runtime_target": _runtime_target_for_tier(tier),
        }
    return resolved


def _cli_command(argv: Sequence[str]) -> str:
    if not argv:
        return "python3 evals/memory_retrieval_benchmark.py"
    if argv[0].endswith("memory_retrieval_benchmark.py"):
        return "python3 " + " ".join(argv)
    return " ".join(argv)


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


def _memory_alias_values(memory: Mapping[str, Any]) -> list[str]:
    """Return explicit ID-like aliases that must not collide with fixture IDs or each other."""
    aliases: list[str] = []
    metadata = memory.get("metadata")
    if not isinstance(metadata, Mapping):
        return aliases

    explicit_alias_keys = {
        "alias",
        "aliases",
        "canonical_id",
        "fixture_id",
        "legacy_id",
        "server_id",
    }

    def collect(value: Any) -> None:
        if isinstance(value, str) and value.strip():
            aliases.append(value.strip())
        elif isinstance(value, list):
            for item in value:
                collect(item)
        elif isinstance(value, Mapping):
            nested_string = value.get("String") or value.get("string")
            if isinstance(nested_string, str) and nested_string.strip():
                aliases.append(nested_string.strip())

    for key, value in metadata.items():
        normalized_key = str(key).strip().lower()
        if normalized_key in explicit_alias_keys or normalized_key.endswith("_alias") or normalized_key.endswith("_id_alias"):
            collect(value)
    return aliases


def _validate_memory_fixture_identity_integrity(memories: Sequence[Mapping[str, Any]], *, label: str) -> dict[str, Any]:
    memory_ids = [str(memory.get("id")) for memory in memories]
    _assert_unique_values(memory_ids, label=f"{label} fixture IDs")

    alias_owner: dict[str, str] = {}
    collisions: list[str] = []
    memory_id_set = set(memory_ids)
    for memory in memories:
        memory_id = str(memory.get("id"))
        for alias in _memory_alias_values(memory):
            if alias == memory_id:
                continue
            if alias in memory_id_set:
                collisions.append(f"{memory_id} alias {alias} collides with fixture memory id")
            previous_owner = alias_owner.get(alias)
            if previous_owner is not None and previous_owner != memory_id:
                collisions.append(f"alias {alias} is shared by {previous_owner} and {memory_id}")
            alias_owner[alias] = memory_id

    if collisions:
        raise AssertionError(f"{label} fixture alias collisions detected: {collisions}")

    return {
        "memory_unique_ids": len(memory_id_set),
        "explicit_alias_count": len(alias_owner),
    }


def _validate_memory_label_rationale(
    query: Mapping[str, Any],
    *,
    fixture_tier: str,
    allowed_categories: set[str] | None = None,
) -> str:
    query_id = str(query.get("id") or "<missing-id>")
    label_rationale = query.get("label_rationale")
    if not isinstance(label_rationale, Mapping):
        raise AssertionError(f"Memory query missing label_rationale object: {query_id}")

    required_fields = (
        "scenario_category",
        "tier",
        "why_label_is_correct",
        "expected_behavior",
    )
    missing = [field for field in required_fields if not str(label_rationale.get(field) or "").strip()]
    if missing:
        raise AssertionError(f"Memory query {query_id} missing label_rationale fields: {missing}")

    scenario_category = str(label_rationale["scenario_category"]).strip()
    tier = str(label_rationale["tier"]).strip()
    if tier != fixture_tier:
        raise AssertionError(f"Memory query {query_id} must declare label_rationale.tier={fixture_tier}, got {tier!r}")
    if allowed_categories is not None and scenario_category not in allowed_categories:
        raise AssertionError(
            f"Memory query {query_id} uses unsupported label_rationale.scenario_category={scenario_category!r}; "
            f"expected one of {sorted(allowed_categories)}"
        )

    expected_ids = [str(memory_id) for memory_id in query.get("expected_ids", [])]
    rationale_expected_ids = label_rationale.get("expected_ids", [])
    if not isinstance(rationale_expected_ids, list):
        raise AssertionError(f"Memory query {query_id} label_rationale.expected_ids must be a list")
    rationale_expected = [str(memory_id) for memory_id in rationale_expected_ids]
    if rationale_expected != expected_ids:
        raise AssertionError(
            f"Memory query {query_id} label_rationale.expected_ids drift: "
            f"expected {expected_ids}, got {rationale_expected}"
        )

    no_match_raw = label_rationale.get("no_match_expected")
    if not isinstance(no_match_raw, bool):
        raise AssertionError(f"Memory query {query_id} label_rationale.no_match_expected must be boolean")
    query_type = str(query.get("query_type") or "")
    if expected_ids and no_match_raw:
        raise AssertionError(f"Memory query {query_id} marks no_match_expected=true but has expected_ids")
    if not expected_ids and query_type != "negative_no_match" and not no_match_raw:
        raise AssertionError(
            f"Memory query {query_id} with empty expected_ids must be negative_no_match or label_rationale.no_match_expected=true"
        )
    if query_type == "negative_no_match" and (expected_ids or not no_match_raw):
        raise AssertionError(
            f"Memory query {query_id} negative_no_match must have empty expected_ids and no_match_expected=true"
        )

    return scenario_category


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
    fixture_to_server_id_map: dict[str, str] = {}
    call_log: list[dict[str, Any]] = []
    for memory in memories:
        fixture_id = str(memory.get("id")) if memory.get("id") is not None else None
        decoded = _decode_tool_response(client.call_tool("store_memory", dict(memory), timeout=timeout_s))
        payload = decoded["payload"]
        memory_id = str(payload.get("id") or memory.get("id"))
        if fixture_id:
            fixture_to_server_id_map[fixture_id] = memory_id
        memory_ids.append(memory_id)
        call_log.append(
            {
                "fixture_id": fixture_id,
                "memory_id": memory_id,
                "memory_type": memory.get("memory_type"),
                "namespace": memory.get("namespace"),
                "user_id": memory.get("user_id"),
                "agent_id": memory.get("agent_id"),
                "run_id": memory.get("run_id"),
                "parse_errors": decoded["parse_errors"],
            }
        )
    return {
        "memory_ids": memory_ids,
        "memory_count": len(memory_ids),
        "fixture_to_server_id_map": fixture_to_server_id_map,
        "calls": call_log,
    }


def _remap_expected_fixture_ids(
    expected_fixture_ids: Sequence[str],
    fixture_to_server_id_map: Mapping[str, str] | None,
) -> tuple[list[str], list[str]]:
    if not fixture_to_server_id_map:
        return list(expected_fixture_ids), []

    remapped: list[str] = []
    missing: list[str] = []
    seen: set[str] = set()
    for fixture_id in expected_fixture_ids:
        server_id = fixture_to_server_id_map.get(fixture_id)
        if not server_id:
            missing.append(fixture_id)
            continue
        if server_id in seen:
            continue
        seen.add(server_id)
        remapped.append(server_id)
    return remapped, missing


def _server_to_fixture_id_map(fixture_to_server_id_map: Mapping[str, str] | None) -> dict[str, str]:
    if not fixture_to_server_id_map:
        return {}
    server_to_fixture: dict[str, str] = {}
    for fixture_id, server_id in fixture_to_server_id_map.items():
        server_to_fixture.setdefault(str(server_id), str(fixture_id))
    return server_to_fixture


def seed_memory_fixtures(
    client: McpClientLike,
    *,
    fixture_tier: str = V2_DEFAULT_FIXTURE_TIER,
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
        normalized_tier = _normalize_fixture_tier(fixture_tier)
        memories = load_memory_corpus(corpus_path)
        graph_fixture = load_graph_fixture(graph_path)
        queries = load_golden_queries(golden_path)
        if normalized_tier == "small":
            validation = _validate_mini_long_memory_caps(memories=memories, queries=queries)
        elif normalized_tier == "medium":
            validation = _validate_medium_long_memory_caps(memories=memories, queries=queries)
        else:
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


def _extract_record_id_from_mapping(value: Mapping[str, Any]) -> str | None:
    if not isinstance(value, Mapping):
        return None

    if isinstance(value.get("String"), str) and value.get("String"):
        return str(value.get("String"))

    for key in ("id", "key", "record", "memory", "item", "node", "value"):
        nested = value.get(key)
        normalized = _normalize_memory_result_id(nested)
        if normalized:
            return normalized

    return None


def _normalize_memory_result_id(value: Any) -> str | None:
    if value is None:
        return None

    if isinstance(value, Mapping):
        return _extract_record_id_from_mapping(value)

    if isinstance(value, str):
        raw = value.strip()
        if not raw:
            return None

        if raw.startswith("{") and "String" in raw:
            parsed: Any | None = None
            try:
                parsed = json.loads(raw)
            except json.JSONDecodeError:
                try:
                    parsed = ast.literal_eval(raw)
                except (SyntaxError, ValueError):
                    parsed = None

            if isinstance(parsed, Mapping):
                normalized = _extract_record_id_from_mapping(parsed)
                if normalized:
                    return normalized

            regex_match = re.search(r'''['\"]String['\"]\s*:\s*['\"]([^'\"]+)['\"]''', raw)
            if regex_match:
                return regex_match.group(1)

        return raw

    return None


def _extract_item_id(item: dict[str, Any]) -> str | None:
    direct_keys = ("id", "memory_id", "entity_id")
    for key in direct_keys:
        value = item.get(key)
        normalized = _normalize_memory_result_id(value)
        if normalized:
            return normalized

    nested_keys = ("memory", "record", "item", "node")
    for key in nested_keys:
        nested = item.get(key)
        if isinstance(nested, dict):
            for nested_key in direct_keys:
                value = nested.get(nested_key)
                normalized = _normalize_memory_result_id(value)
                if normalized:
                    return normalized
    return None


def _extract_item_score(item: dict[str, Any]) -> float | None:
    direct_keys = ("score", "similarity", "distance", "rank_score", "relevance", "confidence")
    for key in direct_keys:
        value = item.get(key)
        if isinstance(value, (int, float)):
            return float(value)

    nested_keys = ("memory", "record", "item", "node")
    for key in nested_keys:
        nested = item.get(key)
        if not isinstance(nested, dict):
            continue
        for nested_key in direct_keys:
            value = nested.get(nested_key)
            if isinstance(value, (int, float)):
                return float(value)
    return None


def _truncate_text_preview(value: str, *, limit: int = 120) -> str:
    compact = " ".join(value.split())
    if len(compact) <= limit:
        return compact
    return compact[: max(0, limit - 1)].rstrip() + "…"


def _extract_item_preview(item: dict[str, Any]) -> str | None:
    text_keys = ("content", "text", "summary", "title", "description")

    def _extract_from_mapping(mapping: dict[str, Any]) -> str | None:
        for key in text_keys:
            value = mapping.get(key)
            if isinstance(value, str) and value.strip():
                return value
            if isinstance(value, dict):
                nested_content = value.get("content")
                if isinstance(nested_content, str) and nested_content.strip():
                    return nested_content
        return None

    direct = _extract_from_mapping(item)
    if direct:
        return _truncate_text_preview(direct)

    for key in ("memory", "record", "item", "node"):
        nested = item.get(key)
        if not isinstance(nested, dict):
            continue
        nested_value = _extract_from_mapping(nested)
        if nested_value:
            return _truncate_text_preview(nested_value)
    return None


def _extract_raw_top_k(
    payload: dict[str, Any],
    *,
    server_to_fixture: Mapping[str, str],
    limit: int = 10,
) -> list[dict[str, Any]]:
    diagnostics: list[dict[str, Any]] = []
    seen_result_ids: set[str] = set()

    for item in _normalize_result_items(payload):
        result_id = _extract_item_id(item)
        if not result_id or result_id in seen_result_ids:
            continue
        seen_result_ids.add(result_id)

        row: dict[str, Any] = {
            "rank": len(diagnostics) + 1,
            "result_id": result_id,
            "fixture_id": server_to_fixture.get(result_id),
        }
        score = _extract_item_score(item)
        if score is not None:
            row["score"] = score
        preview = _extract_item_preview(item)
        if preview:
            row["preview"] = preview

        diagnostics.append(row)
        if len(diagnostics) >= max(1, limit):
            break

    return diagnostics


def _has_embedding_not_ready_signal(
    *,
    summary_partial_reason_code: str | None,
    warnings: Sequence[Any],
    contract: Mapping[str, Any] | None,
) -> bool:
    signal_bag: list[str] = []
    if isinstance(summary_partial_reason_code, str):
        signal_bag.append(summary_partial_reason_code)

    for warning in warnings:
        if isinstance(warning, str):
            signal_bag.append(warning)
        elif isinstance(warning, dict):
            signal_bag.extend(str(value) for value in warning.values() if value is not None)

    if isinstance(contract, Mapping):
        signal_bag.extend(str(value) for value in contract.values() if value is not None)

    signal_text = " ".join(signal_bag).lower()
    if "embedding" not in signal_text:
        return False
    return any(token in signal_text for token in ("not_ready", "not ready", "loading", "initializ", "timeout"))


def _top_result_score(raw_top_k: Sequence[Mapping[str, Any]]) -> float | None:
    for row in raw_top_k:
        if not isinstance(row, Mapping):
            continue
        score = row.get("score")
        if isinstance(score, (int, float)):
            return float(score)
    return None


def _classify_failure_type(
    *,
    query_type: str,
    expected_fixture_ids: Sequence[str],
    expected_server_ids: Sequence[str],
    missing_expected_fixture_ids: Sequence[str],
    found_expected_server_ids: Sequence[str],
    result_count: int,
    raw_top_k: Sequence[Mapping[str, Any]],
    call_error: str | None,
    parse_errors: Sequence[str],
    summary_partial_reason_code: str | None,
    warnings: Sequence[Any],
    contract: Mapping[str, Any] | None,
) -> str:
    if call_error:
        return "call_error"

    if parse_errors and result_count == 0:
        return "parse_error"

    if _has_embedding_not_ready_signal(
        summary_partial_reason_code=summary_partial_reason_code,
        warnings=warnings,
        contract=contract,
    ):
        return "embedding_not_ready"

    if query_type == "negative_no_match":
        if result_count == 0:
            return "expected_no_match"
        return "wrong_rank"

    if found_expected_server_ids:
        return "none"

    if result_count == 0:
        return "empty_results"

    if missing_expected_fixture_ids or (expected_fixture_ids and not expected_server_ids):
        return "id_mismatch"

    top_score = _top_result_score(raw_top_k)
    if top_score is not None and top_score <= 0.05:
        return "low_confidence"

    return "true_miss"


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


def execute_query(
    client: McpClientLike,
    query: dict[str, Any],
    *,
    fixture_to_server_id_map: Mapping[str, str] | None = None,
) -> dict[str, Any]:
    query_id = str(query.get("id"))
    query_type = str(query.get("query_type") or "")
    expected_fixture_ids = [str(value) for value in query.get("expected_ids", [])]
    expected_server_ids, missing_expected_fixture_ids = _remap_expected_fixture_ids(
        expected_fixture_ids,
        fixture_to_server_id_map,
    )
    server_to_fixture = _server_to_fixture_id_map(fixture_to_server_id_map)
    negative = query_type == "negative_no_match" or not expected_fixture_ids

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
    raw_top_k: list[dict[str, Any]] = []

    try:
        raw_response = client.call_tool(tool_name, arguments)
        decoded = _decode_tool_response(raw_response)
        payload = decoded["payload"]
        result = decoded["result"]
        parse_errors = decoded["parse_errors"]
        result_ids = _normalize_result_ids(payload)
        raw_top_k = _extract_raw_top_k(payload, server_to_fixture=server_to_fixture, limit=10)
        summary_partial_reason_code = _extract_summary_partial_reason_code(payload, result)
        warnings = _extract_warnings(payload, result)
        contract = _extract_contract(payload, result)
    except Exception as exc:  # noqa: BLE001
        call_error = str(exc)

    latency_ms = (time.perf_counter() - started) * 1000.0
    expected_server_set = set(expected_server_ids)
    found_expected_server_ids = [result_id for result_id in result_ids if result_id in expected_server_set]
    found_expected_fixture_ids = [
        server_to_fixture[result_id]
        for result_id in found_expected_server_ids
        if result_id in server_to_fixture
    ]
    failure_type = _classify_failure_type(
        query_type=query_type,
        expected_fixture_ids=expected_fixture_ids,
        expected_server_ids=expected_server_ids,
        missing_expected_fixture_ids=missing_expected_fixture_ids,
        found_expected_server_ids=found_expected_server_ids,
        result_count=len(result_ids),
        raw_top_k=raw_top_k,
        call_error=call_error,
        parse_errors=parse_errors,
        summary_partial_reason_code=summary_partial_reason_code,
        warnings=warnings,
        contract=contract,
    )
    metric_row = compute_query_metrics(
        result_ids,
        expected_server_ids,
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
        "expected_ids": expected_fixture_ids,
        "expected_fixture_ids": expected_fixture_ids,
        "expected_server_ids": expected_server_ids,
        "missing_expected_fixture_ids": missing_expected_fixture_ids,
        "found_expected_ids": found_expected_server_ids,
        "found_expected_server_ids": found_expected_server_ids,
        "found_expected_fixture_ids": found_expected_fixture_ids,
        "result_ids": result_ids,
        "raw_top_k": raw_top_k,
        "result_count": len(result_ids),
        "summary_partial_reason_code": summary_partial_reason_code,
        "reason_code_classification": classify_reason_code(
            summary_partial_reason_code,
            evidence={"failure_type": failure_type, "retrieval_blocked": failure_type in {"call_error", "parse_error", "embedding_not_ready"}},
        ) if summary_partial_reason_code else None,
        "failure_type": failure_type,
        "warnings": warnings,
        "contract": contract,
        "parse_errors": parse_errors,
    }
    if call_error is not None:
        row["call_error"] = call_error
    return row


def execute_queries(
    client: McpClientLike,
    queries: Sequence[dict[str, Any]],
    *,
    fixture_to_server_id_map: Mapping[str, str] | None = None,
) -> dict[str, Any]:
    per_query = [
        execute_query(client, query, fixture_to_server_id_map=fixture_to_server_id_map)
        for query in queries
    ]
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
    fixture_tier: str = V2_DEFAULT_FIXTURE_TIER,
) -> dict[str, Any]:
    fixture_tier = _normalize_fixture_tier(fixture_tier)
    if memories is None or queries is None:
        tier_memories, _, tier_queries, _, _ = _load_tier_inputs(fixture_tier)
        memories = memories if memories is not None else tier_memories
        queries = queries if queries is not None else tier_queries
    memories = memories if memories is not None else []
    graph_fixture = graph_fixture if graph_fixture is not None else load_graph_fixture()
    queries = queries if queries is not None else []
    query_types = sorted({str(query.get("query_type")) for query in queries})
    fixture_paths = _fixture_paths_for_tier(fixture_tier)
    return {
        "fixture_paths": fixture_paths,
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
        "fixture_tier": fixture_tier,
        "runtime_target": _runtime_target_for_tier(fixture_tier),
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

    positive_hits = sum(1 for row in positive_rows if row.get("expected_rank") is not None)
    positive_hit_rate = (positive_hits / len(positive_rows)) if positive_rows else None

    return {
        "positive_query_count": len(positive_rows),
        "positive_hit_rate": positive_hit_rate,
        "positive_mean_mrr": _mean_metric("mrr"),
        "positive_mean_recall_at_5": _mean_metric("recall_at_5"),
        "positive_mean_ndcg_at_5": _mean_metric("ndcg_at_5"),
        "positive_mean_precision_at_5": _mean_metric("precision_at_5"),
        "positive_mean_precision_at_10": _mean_metric("precision_at_10"),
    }


def _evaluate_threshold_status(
    *,
    fixture_tier: str,
    aggregate: Mapping[str, Any],
    policy_name: str,
) -> dict[str, Any]:
    policy_matrix: dict[str, list[dict[str, Any]]] = {
        "small": [
            {"metric": "blocker_count", "comparator": "==", "threshold": 0, "severity": "blocker"},
            {"metric": "positive_hit_rate", "comparator": ">=", "threshold": 0.80, "severity": "blocker"},
            {"metric": "positive_mean_mrr", "comparator": ">=", "threshold": 0.70, "severity": "blocker"},
            {"metric": "positive_mean_recall_at_5", "comparator": ">=", "threshold": 0.80, "severity": "blocker"},
            {"metric": "positive_mean_ndcg_at_5", "comparator": ">=", "threshold": 0.75, "severity": "warn"},
            {"metric": "positive_mean_precision_at_5", "comparator": ">=", "threshold": 0.20, "severity": "warn"},
            {"metric": "mean_latency_ms", "comparator": "<=", "threshold": 5000, "severity": "warn"},
            {"metric": "runtime_minutes", "comparator": "<=", "threshold": 10, "severity": "warn"},
        ],
        "medium": [
            {"metric": "blocker_count", "comparator": "==", "threshold": 0, "severity": "blocker"},
            {"metric": "positive_hit_rate", "comparator": ">=", "threshold": 0.82, "severity": "blocker"},
            {"metric": "positive_mean_mrr", "comparator": ">=", "threshold": 0.72, "severity": "blocker"},
            {"metric": "positive_mean_recall_at_5", "comparator": ">=", "threshold": 0.82, "severity": "blocker"},
            {"metric": "positive_mean_ndcg_at_5", "comparator": ">=", "threshold": 0.77, "severity": "warn"},
            {"metric": "positive_mean_precision_at_5", "comparator": ">=", "threshold": 0.22, "severity": "warn"},
            {"metric": "mean_latency_ms", "comparator": "<=", "threshold": 7500, "severity": "warn"},
            {"metric": "runtime_minutes", "comparator": "<=", "threshold": 30, "severity": "warn"},
        ],
        "stress": [
            {"metric": "blocker_count", "comparator": "==", "threshold": 0, "severity": "blocker"},
            {"metric": "positive_hit_rate", "comparator": ">=", "threshold": 0.80, "severity": "blocker"},
            {"metric": "positive_mean_mrr", "comparator": ">=", "threshold": 0.70, "severity": "blocker"},
            {"metric": "positive_mean_recall_at_5", "comparator": ">=", "threshold": 0.80, "severity": "blocker"},
            {"metric": "positive_mean_ndcg_at_5", "comparator": ">=", "threshold": 0.75, "severity": "warn"},
            {"metric": "positive_mean_precision_at_5", "comparator": ">=", "threshold": 0.20, "severity": "warn"},
            {"metric": "mean_latency_ms", "comparator": "<=", "threshold": 15000, "severity": "warn"},
            {"metric": "runtime_minutes", "comparator": "<=", "threshold": 90, "severity": "warn"},
        ],
    }

    rules = policy_matrix.get(fixture_tier)
    if not rules:
        return {
            "policy_name": policy_name,
            "enforcement": "local-only",
            "status": "deferred",
            "reason": f"unsupported fixture tier for threshold evaluation: {fixture_tier}",
            "fixture_tier": fixture_tier,
            "evaluated_metrics": 0,
            "failures": [],
        }

    query_count = aggregate.get("query_count")
    if not isinstance(query_count, int) or query_count <= 0:
        return {
            "policy_name": policy_name,
            "enforcement": "local-only",
            "status": "deferred",
            "reason": "threshold evaluation deferred because no queries were executed",
            "fixture_tier": fixture_tier,
            "evaluated_metrics": 0,
            "failures": [],
        }

    failures: list[dict[str, Any]] = []
    evaluated = 0
    for rule in rules:
        metric_key = str(rule["metric"])
        comparator = str(rule["comparator"])
        threshold = float(rule["threshold"])
        severity = str(rule["severity"])
        actual_raw = aggregate.get(metric_key)
        if not isinstance(actual_raw, (int, float)):
            return {
                "policy_name": policy_name,
                "enforcement": "local-only",
                "status": "deferred",
                "reason": f"threshold evaluation deferred because metric is unavailable: {metric_key}",
                "fixture_tier": fixture_tier,
                "evaluated_metrics": evaluated,
                "failures": failures,
            }
        actual = float(actual_raw)
        evaluated += 1

        passed = False
        if comparator == "==":
            passed = abs(actual - threshold) <= 1e-9
        elif comparator == ">=":
            passed = actual + 1e-9 >= threshold
        elif comparator == "<=":
            passed = actual <= threshold + 1e-9
        if not passed:
            failures.append(
                {
                    "metric": metric_key,
                    "severity": severity,
                    "comparator": comparator,
                    "threshold": threshold,
                    "actual": actual,
                }
            )

    blocker_failures = [failure for failure in failures if failure.get("severity") == "blocker"]
    warn_failures = [failure for failure in failures if failure.get("severity") == "warn"]
    if blocker_failures:
        status = "blocker"
        reason = f"{len(blocker_failures)} blocker threshold(s) failed"
    elif warn_failures:
        status = "warn"
        reason = f"{len(warn_failures)} warning threshold(s) failed"
    else:
        status = "pass"
        reason = "all required threshold checks passed"

    return {
        "policy_name": policy_name,
        "enforcement": "local-only",
        "status": status,
        "reason": reason,
        "fixture_tier": fixture_tier,
        "evaluated_metrics": evaluated,
        "failures": failures,
        "failure_counts": {
            "blocker": len(blocker_failures),
            "warn": len(warn_failures),
        },
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
    memories = load_memory_corpus(MINI_FIXTURE_CORPUS_PATH)
    graph_fixture = load_graph_fixture()
    queries = load_golden_queries(MINI_GOLDEN_QUERIES_PATH)
    mini_memories = load_memory_corpus(MINI_FIXTURE_CORPUS_PATH)
    mini_queries = load_golden_queries(MINI_GOLDEN_QUERIES_PATH)
    medium_memories = load_memory_corpus(MEDIUM_FIXTURE_CORPUS_PATH)
    medium_queries = load_golden_queries(MEDIUM_GOLDEN_QUERIES_PATH)
    fixture_summary = _validate_mini_long_memory_caps(memories=memories, queries=queries)
    query_summary = {
        "query_count": len(queries),
        "required_query_types": sorted(REQUIRED_QUERY_TYPES),
        "observed_query_types": sorted({str(query.get("query_type")) for query in queries}),
    }
    mini_fixture_summary = _validate_mini_long_memory_caps(memories=mini_memories, queries=mini_queries)
    medium_fixture_summary = _validate_medium_long_memory_caps(memories=medium_memories, queries=medium_queries)
    label_qa_rejections = _self_test_label_qa_rejections(
        medium_memories=medium_memories,
        medium_queries=medium_queries,
    )
    stress_manifest_summary = _validate_stress_long_memory_manifests()
    tier_resolution_summary = _self_test_fixture_tier_resolution()
    negative_snippet = negative_query_metrics_snippet(queries)

    sample_rows: list[dict[str, Any]] = []
    for index, query in enumerate(queries, start=1):
        expected_ids = [str(value) for value in query.get("expected_ids", [])]
        query_type = str(query.get("query_type"))
        if expected_ids:
            mock_result_ids = [expected_ids[0], "synthetic_non_expected_id"]
            failure_type = "none"
        else:
            mock_result_ids = []
            failure_type = "expected_no_match"
        sample_rows.append(
            {
                "query_id": query.get("id", f"query_{index}"),
                "query": query.get("query"),
                **compute_query_metrics(
                    mock_result_ids,
                    expected_ids,
                    query_type=query_type,
                    latency_ms=10.0 + index,
                    negative=query_type == "negative_no_match" or not expected_ids,
                ),
                "failure_type": failure_type,
            }
        )

    aggregate = aggregate_metrics(sample_rows)
    aggregate.update(_positive_query_aggregate_metrics(sample_rows))
    aggregate["runtime_minutes"] = 0.0
    aggregate["baseline_query_count"] = len(queries)
    aggregate["seed_completed"] = True
    aggregate["observed_summary_partial_reason_codes"] = []
    aggregate["blocker_count"] = 0
    aggregate["readiness_fallback"] = classify_readiness_fallback(None)
    aggregate["reason_code_classification"] = classify_reason_codes([], evidence={"blocker_count": 0})
    aggregate["threshold_evaluation"] = _evaluate_threshold_status(
        fixture_tier=V2_DEFAULT_FIXTURE_TIER,
        aggregate=aggregate,
        policy_name=V2_THRESHOLD_POLICY,
    )
    with tempfile.TemporaryDirectory(prefix="task-8-memory-self-test-") as tmp:
        tmp_path = Path(tmp)
        json_path = tmp_path / "self-test.json"
        md_path = tmp_path / "self-test.md"
        manifest = _fixture_manifest(
            memories=memories,
            graph_fixture=graph_fixture,
            queries=queries,
            command=["offline-self-test"],
            fixture_tier=V2_DEFAULT_FIXTURE_TIER,
        )
        manifest["schema_version"] = V2_SCHEMA_VERSION
        manifest["fixture_tier"] = V2_DEFAULT_FIXTURE_TIER
        manifest["baseline_version"] = V2_DEFAULT_BASELINE_VERSION
        manifest["threshold_policy"] = {"name": V2_THRESHOLD_POLICY, "enforcement": "local-only"}
        manifest["runtime_target"] = _runtime_target_for_tier(V2_DEFAULT_FIXTURE_TIER)
        manifest["determinism_policy"] = {"name": V2_DETERMINISM_POLICY}
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
        if parsed.get("schema_version") is None:
            raise AssertionError("self-test JSON report missing schema_version")
        if parsed.get("fixture_tier") is None:
            raise AssertionError("self-test JSON report missing fixture_tier")
        if parsed.get("threshold_status") is None:
            raise AssertionError("self-test JSON report missing threshold_status")
        if not isinstance(parsed.get("failure_buckets"), dict):
            raise AssertionError("self-test JSON report missing failure_buckets")
        if parsed["failure_buckets"].get("expected_no_match", 0) < 1:
            raise AssertionError("self-test JSON report missing expected_no_match failure bucket")
        if parsed["failure_buckets"].get("none", 0) < 1:
            raise AssertionError("self-test JSON report missing none failure bucket")
        if not isinstance(parsed.get("readiness_summary"), dict):
            raise AssertionError("self-test JSON report missing readiness_summary")
        markdown = md_path.read_text(encoding="utf-8")
        if "Memory Retrieval Baseline" not in markdown:
            raise AssertionError("self-test markdown title missing")
        if "## Benchmark V2 summary" not in markdown:
            raise AssertionError("self-test markdown V2 summary missing")
        if "| expected_no_match |" not in markdown:
            raise AssertionError("self-test markdown expected_no_match bucket missing")

    assert _classify_failure_type(
        query_type="search_vector",
        expected_fixture_ids=["m1"],
        expected_server_ids=["srv1"],
        missing_expected_fixture_ids=[],
        found_expected_server_ids=[],
        result_count=1,
        raw_top_k=[{"score": 0.01}],
        call_error=None,
        parse_errors=[],
        summary_partial_reason_code=None,
        warnings=[],
        contract=None,
    ) == "low_confidence"
    assert _classify_failure_type(
        query_type="search_vector",
        expected_fixture_ids=["m1"],
        expected_server_ids=["srv1"],
        missing_expected_fixture_ids=[],
        found_expected_server_ids=[],
        result_count=1,
        raw_top_k=[{"score": 0.5}],
        call_error=None,
        parse_errors=[],
        summary_partial_reason_code=None,
        warnings=[],
        contract=None,
    ) == "true_miss"

    print(
        "self-test passed "
        f"(fixtures={fixture_summary['memory_count']} memories, queries={query_summary['query_count']}, "
        f"negative_queries={negative_snippet['query_count']}, "
        f"mini_fixtures={mini_fixture_summary['memory_count']} memories, "
        f"mini_queries={mini_fixture_summary['query_count']}, "
        f"medium_fixtures={medium_fixture_summary['memory_count']} memories, "
        f"medium_queries={medium_fixture_summary['query_count']}, "
        f"stress_manifests={stress_manifest_summary['status']})"
    )
    print(
        json.dumps(
            {
                "fixture_summary": fixture_summary,
                "query_summary": query_summary,
                "mini_fixture_summary": mini_fixture_summary,
                "medium_fixture_summary": medium_fixture_summary,
                "label_qa_rejections": label_qa_rejections,
                "stress_manifest_summary": stress_manifest_summary,
                "tier_resolution_summary": tier_resolution_summary,
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

    identity_summary = _validate_memory_fixture_identity_integrity(memories, label="Baseline memory")
    memory_ids = [str(memory["id"]) for memory in memories]

    entity_id_set = {str(entity["id"]) for entity in entities}
    query_ids = [str(query["id"]) for query in queries]

    _assert_unique_values(query_ids, label="Baseline golden query IDs")

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
        "memory_unique_ids": identity_summary["memory_unique_ids"],
        "explicit_alias_count": identity_summary["explicit_alias_count"],
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


def _validate_mini_long_memory_caps(
    *,
    memories: list[dict[str, Any]] | None = None,
    queries: list[dict[str, Any]] | None = None,
) -> dict[str, Any]:
    memories = memories if memories is not None else load_memory_corpus(MINI_FIXTURE_CORPUS_PATH)
    queries = queries if queries is not None else load_golden_queries(MINI_GOLDEN_QUERIES_PATH)

    memory_count = len(memories)
    query_count = len(queries)
    if memory_count > MINI_MEMORY_CORPUS_MAX_COUNT:
        raise AssertionError(
            f"Mini long-memory fixture exceeds cap: {memory_count} > {MINI_MEMORY_CORPUS_MAX_COUNT}"
        )
    if query_count > MINI_GOLDEN_QUERY_MAX_COUNT:
        raise AssertionError(
            f"Mini long-memory golden queries exceed cap: {query_count} > {MINI_GOLDEN_QUERY_MAX_COUNT}"
        )

    identity_summary = _validate_memory_fixture_identity_integrity(memories, label="Mini long-memory")
    memory_index = {str(memory.get("id")): memory for memory in memories}

    query_ids = [str(query.get("id")) for query in queries]
    _assert_unique_values(query_ids, label="Mini long-memory query IDs")

    supported_tools = {"recall", "search_memory", "get_valid"}
    for query in queries:
        tool = str(query.get("tool") or "")
        if tool not in supported_tools:
            raise AssertionError(f"Mini long-memory query uses unsupported tool: {query.get('id')}: {tool}")
        expected_ids = [str(memory_id) for memory_id in query.get("expected_ids", [])]
        for memory_id in expected_ids:
            if memory_id not in memory_index:
                raise AssertionError(f"Mini long-memory query references unknown memory id: {memory_id}")

    temporal_covered = any(
        isinstance(query.get("filters"), dict) and query["filters"].get("valid_at")
        for query in queries
    )
    namespace_covered = any(
        isinstance(query.get("filters"), dict) and query["filters"].get("namespace")
        for query in queries
    )
    negative_covered = any(
        str(query.get("query_type")) == "negative_no_match"
        and isinstance(query.get("expected_ids"), list)
        and len(query.get("expected_ids", [])) == 0
        for query in queries
    )

    person_project_topic_covered = False
    for query in queries:
        expected_ids = [str(memory_id) for memory_id in query.get("expected_ids", [])]
        if not expected_ids:
            continue
        has_person = False
        has_project = False
        has_topic = False
        for expected_id in expected_ids:
            memory = memory_index.get(expected_id, {})
            metadata = memory.get("metadata")
            if not isinstance(metadata, dict):
                continue
            if metadata.get("person"):
                has_person = True
            if metadata.get("project"):
                has_project = True
            if metadata.get("topic"):
                has_topic = True
        if has_person and has_project and has_topic:
            person_project_topic_covered = True
            break

    missing_categories: list[str] = []
    if not temporal_covered:
        missing_categories.append("temporal")
    if not person_project_topic_covered:
        missing_categories.append("person_project_topic")
    if not namespace_covered:
        missing_categories.append("namespace")
    if not negative_covered:
        missing_categories.append("negative")
    if missing_categories:
        raise AssertionError(
            f"Mini long-memory queries missing required coverage categories: {missing_categories}"
        )

    return {
        "fixture_paths": {
            "memory_corpus": str(MINI_FIXTURE_CORPUS_PATH.relative_to(ROOT)),
            "golden_queries": str(MINI_GOLDEN_QUERIES_PATH.relative_to(ROOT)),
        },
        "caps": {
            "memory_count_max": MINI_MEMORY_CORPUS_MAX_COUNT,
            "query_count_max": MINI_GOLDEN_QUERY_MAX_COUNT,
        },
        "memory_count": memory_count,
        "query_count": query_count,
        "memory_unique_ids": identity_summary["memory_unique_ids"],
        "explicit_alias_count": identity_summary["explicit_alias_count"],
        "coverage": {
            "temporal": temporal_covered,
            "person_project_topic": person_project_topic_covered,
            "namespace": namespace_covered,
            "negative": negative_covered,
        },
    }


def _validate_medium_long_memory_caps(
    *,
    memories: list[dict[str, Any]] | None = None,
    queries: list[dict[str, Any]] | None = None,
) -> dict[str, Any]:
    memories = memories if memories is not None else load_memory_corpus(MEDIUM_FIXTURE_CORPUS_PATH)
    queries = queries if queries is not None else load_golden_queries(MEDIUM_GOLDEN_QUERIES_PATH)

    memory_count = len(memories)
    query_count = len(queries)
    if memory_count > MEDIUM_MEMORY_CORPUS_MAX_COUNT:
        raise AssertionError(
            f"Medium long-memory fixture exceeds cap: {memory_count} > {MEDIUM_MEMORY_CORPUS_MAX_COUNT}"
        )
    if query_count > MEDIUM_GOLDEN_QUERY_MAX_COUNT:
        raise AssertionError(
            f"Medium long-memory golden queries exceed cap: {query_count} > {MEDIUM_GOLDEN_QUERY_MAX_COUNT}"
        )

    identity_summary = _validate_memory_fixture_identity_integrity(memories, label="Medium long-memory")
    memory_index = {str(memory.get("id")): memory for memory in memories}

    query_ids = [str(query.get("id")) for query in queries]
    _assert_unique_values(query_ids, label="Medium long-memory query IDs")

    supported_tools = {"recall", "search_memory", "get_valid"}
    required_categories = {
        "long_memory_recall",
        "namespace_boundary",
        "temporal_boundary",
        "negative_no_match",
        "partial_readiness",
        "id_mismatch_alias",
        "record_shaped_ids",
    }
    covered_categories: set[str] = set()

    for query in queries:
        tool = str(query.get("tool") or "")
        if tool not in supported_tools:
            raise AssertionError(f"Medium long-memory query uses unsupported tool: {query.get('id')}: {tool}")

        expected_ids = [str(memory_id) for memory_id in query.get("expected_ids", [])]
        for memory_id in expected_ids:
            if memory_id not in memory_index:
                raise AssertionError(f"Medium long-memory query references unknown memory id: {memory_id}")

        scenario_category = _validate_memory_label_rationale(
            query,
            fixture_tier="medium",
            allowed_categories=required_categories,
        )
        covered_categories.add(scenario_category)

    missing_categories = sorted(required_categories - covered_categories)
    if missing_categories:
        raise AssertionError(
            f"Medium long-memory queries missing required scenario categories: {missing_categories}"
        )

    return {
        "fixture_paths": {
            "memory_corpus": str(MEDIUM_FIXTURE_CORPUS_PATH.relative_to(ROOT)),
            "golden_queries": str(MEDIUM_GOLDEN_QUERIES_PATH.relative_to(ROOT)),
        },
        "caps": {
            "memory_count_max": MEDIUM_MEMORY_CORPUS_MAX_COUNT,
            "query_count_max": MEDIUM_GOLDEN_QUERY_MAX_COUNT,
        },
        "memory_count": memory_count,
        "query_count": query_count,
        "memory_unique_ids": identity_summary["memory_unique_ids"],
        "explicit_alias_count": identity_summary["explicit_alias_count"],
        "coverage": {category: category in covered_categories for category in sorted(required_categories)},
    }


def _validate_stress_long_memory_manifests() -> dict[str, Any]:
    fixture_manifest = _load_json(STRESS_FIXTURE_MANIFEST_PATH)
    golden_manifest = _load_json(STRESS_GOLDEN_MANIFEST_PATH)

    if fixture_manifest.get("version") != FIXTURE_SCHEMA_VERSION:
        raise AssertionError(
            f"Invalid stress fixture manifest version in {STRESS_FIXTURE_MANIFEST_PATH}: expected {FIXTURE_SCHEMA_VERSION}"
        )
    if golden_manifest.get("version") != FIXTURE_SCHEMA_VERSION:
        raise AssertionError(
            f"Invalid stress golden manifest version in {STRESS_GOLDEN_MANIFEST_PATH}: expected {FIXTURE_SCHEMA_VERSION}"
        )

    if str(fixture_manifest.get("fixture_tier")) != "stress":
        raise AssertionError("Stress fixture manifest must declare fixture_tier=stress")
    if str(golden_manifest.get("fixture_tier")) != "stress":
        raise AssertionError("Stress golden manifest must declare fixture_tier=stress")

    if not isinstance(fixture_manifest.get("generation_plan"), dict):
        raise AssertionError("Stress fixture manifest missing generation_plan object")
    if not isinstance(golden_manifest.get("generation_plan"), dict):
        raise AssertionError("Stress golden manifest missing generation_plan object")

    return {
        "fixture_manifest": str(STRESS_FIXTURE_MANIFEST_PATH.relative_to(ROOT)),
        "golden_manifest": str(STRESS_GOLDEN_MANIFEST_PATH.relative_to(ROOT)),
        "status": "validated",
    }


def _assert_raises_with_message(fn: Any, expected_substring: str, *, label: str) -> str:
    try:
        fn()
    except AssertionError as exc:
        message = str(exc)
        if expected_substring not in message:
            raise AssertionError(
                f"{label} raised AssertionError without expected diagnostic {expected_substring!r}: {message}"
            ) from exc
        return message
    raise AssertionError(f"{label} did not reject invalid input")


def _self_test_label_qa_rejections(
    *,
    medium_memories: list[dict[str, Any]],
    medium_queries: list[dict[str, Any]],
) -> dict[str, str]:
    duplicate_memories = [dict(memory) for memory in medium_memories]
    duplicate = dict(duplicate_memories[0])
    duplicate["content"] = f"{duplicate['content']} duplicate copy for validation self-test"
    duplicate_memories.append(duplicate)

    alias_collision_memories = [dict(memory) for memory in medium_memories]
    alias_collision_memory = dict(alias_collision_memories[0])
    alias_collision_metadata = dict(alias_collision_memory.get("metadata", {}))
    alias_collision_metadata["aliases"] = [str(alias_collision_memories[1].get("id"))]
    alias_collision_memory["metadata"] = alias_collision_metadata
    alias_collision_memories[0] = alias_collision_memory

    missing_rationale_queries = [dict(query) for query in medium_queries]
    missing_rationale_queries[0] = dict(missing_rationale_queries[0])
    missing_rationale_queries[0].pop("label_rationale", None)

    drifted_rationale_queries = [dict(query) for query in medium_queries]
    drifted_query = dict(drifted_rationale_queries[0])
    drifted_rationale = dict(drifted_query["label_rationale"])
    drifted_rationale["expected_ids"] = []
    drifted_query["label_rationale"] = drifted_rationale
    drifted_rationale_queries[0] = drifted_query

    negative_consistency_queries = [dict(query) for query in medium_queries]
    negative_query = dict(negative_consistency_queries[3])
    negative_rationale = dict(negative_query["label_rationale"])
    negative_rationale["no_match_expected"] = False
    negative_query["label_rationale"] = negative_rationale
    negative_consistency_queries[3] = negative_query

    return {
        "duplicate_memory_ids": _assert_raises_with_message(
            lambda: _validate_medium_long_memory_caps(memories=duplicate_memories, queries=medium_queries),
            "duplicate values",
            label="medium duplicate memory ID validation",
        ),
        "alias_collision": _assert_raises_with_message(
            lambda: _validate_medium_long_memory_caps(memories=alias_collision_memories, queries=medium_queries),
            "alias collisions detected",
            label="medium alias collision validation",
        ),
        "missing_label_rationale": _assert_raises_with_message(
            lambda: _validate_medium_long_memory_caps(memories=medium_memories, queries=missing_rationale_queries),
            "missing label_rationale object",
            label="medium missing label_rationale validation",
        ),
        "expected_id_drift": _assert_raises_with_message(
            lambda: _validate_medium_long_memory_caps(memories=medium_memories, queries=drifted_rationale_queries),
            "label_rationale.expected_ids drift",
            label="medium label expected_ids drift validation",
        ),
        "negative_consistency": _assert_raises_with_message(
            lambda: _validate_medium_long_memory_caps(memories=medium_memories, queries=negative_consistency_queries),
            "negative_no_match must have empty expected_ids and no_match_expected=true",
            label="medium negative no-match consistency validation",
        ),
    }


def self_test_fixtures() -> dict[str, Any]:
    summary = _validate_fixture_caps()
    mini_summary = _validate_mini_long_memory_caps()
    medium_summary = _validate_medium_long_memory_caps()
    stress_summary = _validate_stress_long_memory_manifests()
    label_qa_rejections = _self_test_label_qa_rejections(
        medium_memories=load_memory_corpus(MEDIUM_FIXTURE_CORPUS_PATH),
        medium_queries=load_golden_queries(MEDIUM_GOLDEN_QUERIES_PATH),
    )
    print(
        "fixtures self-test passed "
        f"(memory_count={summary['memory_count']}, graph_entity_count={summary['graph_entity_count']}, "
        f"graph_relation_count={summary['graph_relation_count']}, query_count={summary['query_count']})"
    )
    print(
        json.dumps(
            {
                "baseline": summary,
                "mini_long_memory": mini_summary,
                "medium_long_memory": medium_summary,
                "stress_long_memory": stress_summary,
                "label_qa_rejections": label_qa_rejections,
            },
            sort_keys=True,
        )
    )
    return {
        "baseline": summary,
        "mini_long_memory": mini_summary,
        "medium_long_memory": medium_summary,
        "stress_long_memory": stress_summary,
        "label_qa_rejections": label_qa_rejections,
    }


def self_test_queries() -> dict[str, Any]:
    summary = _validate_query_caps()
    mini_summary = _validate_mini_long_memory_caps()
    medium_summary = _validate_medium_long_memory_caps()
    print(
        "queries self-test passed "
        f"(query_count={summary['query_count']}, required_query_types={summary['required_query_types']})"
    )
    print(
        json.dumps(
            {
                "baseline": summary,
                "mini_long_memory": mini_summary,
                "medium_long_memory": medium_summary,
            },
            sort_keys=True,
        )
    )
    return {
        "baseline": summary,
        "mini_long_memory": mini_summary,
        "medium_long_memory": medium_summary,
    }


def run_queries(args: argparse.Namespace) -> int:
    fixture_tier = _normalize_fixture_tier(args.fixture_tier)
    _, _, queries, _, _ = _load_tier_inputs(fixture_tier)
    if fixture_tier == "stress":
        print(json.dumps({"query_count": 0, "reason": "stress tier is manifest-only and requires generated queries"}, sort_keys=True))
        return 0
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
    V2_EVIDENCE_DIR.mkdir(parents=True, exist_ok=True)
    run_started = time.time()
    fixture_tier = _normalize_fixture_tier(args.fixture_tier)
    resolved_command = resolve_mcp_command()
    warnings: list[Any] = []
    blockers: list[dict[str, Any]] = []
    per_query: list[dict[str, Any]] = []
    raw: dict[str, Any] = {}
    stderr_tail: list[str] = []

    try:
        memories, graph_fixture, queries, validation, query_validation = _load_tier_inputs(fixture_tier)
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
                    elif fixture_tier == "stress":
                        seed_progress = {
                            "status": "deferred",
                            "reason": "stress_manifest_only",
                            "fixture_tier": fixture_tier,
                        }
                        warnings.append("stress tier is manifest-only; runtime seeding/query execution deferred")
                    else:
                        seed_progress = seed_memory_fixtures(
                            client,
                            call_timeout_s=max(args.timeout, 120.0),
                            fixture_tier=fixture_tier,
                            corpus_path=MINI_FIXTURE_CORPUS_PATH if fixture_tier == "small" else MEDIUM_FIXTURE_CORPUS_PATH,
                            readiness_timeout_s=args.readiness_timeout_s,
                            readiness_poll_interval_s=args.poll_interval_s,
                            readiness_fallback_sleep_s=args.readiness_fallback_sleep_s,
                            golden_path=MINI_GOLDEN_QUERIES_PATH if fixture_tier == "small" else MEDIUM_GOLDEN_QUERIES_PATH,
                        )
                    raw["seed_progress"] = seed_progress
                    if seed_progress.get("status") not in {"completed", "deferred"}:
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
                    elif seed_progress.get("status") == "completed":
                        fixture_to_server_id_map: dict[str, str] | None = None
                        memory_progress = seed_progress.get("memory") if isinstance(seed_progress, dict) else None
                        if isinstance(memory_progress, dict):
                            raw_fixture_map = memory_progress.get("fixture_to_server_id_map")
                            if isinstance(raw_fixture_map, dict):
                                fixture_to_server_id_map = {
                                    str(fixture_id): str(server_id)
                                    for fixture_id, server_id in raw_fixture_map.items()
                                }
                        query_result = execute_queries(
                            client,
                            queries,
                            fixture_to_server_id_map=fixture_to_server_id_map,
                        )
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
                    else:
                        per_query = []
                        raw["query_diagnostics"] = {
                            "degraded_or_partial_count": 0,
                            "parse_issue_count": 0,
                            "call_error_count": 0,
                            "reason": "stress_manifest_only",
                        }
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
    aggregate["runtime_minutes"] = round((time.time() - run_started) / 60.0, 6)
    aggregate["baseline_query_count"] = len(queries)
    aggregate["seed_completed"] = bool(raw.get("seed_progress", {}).get("status") == "completed")
    aggregate["observed_summary_partial_reason_codes"] = _reason_codes_from_rows(per_query)
    aggregate["blocker_count"] = len(blockers)
    seed_settle = raw.get("seed_progress", {}).get("settle") if isinstance(raw.get("seed_progress"), dict) else None
    aggregate["readiness_fallback"] = classify_readiness_fallback(seed_settle)
    aggregate["reason_code_classification"] = classify_reason_codes(
        aggregate["observed_summary_partial_reason_codes"],
        evidence={
            "blocker_count": len(blockers),
            "readiness_timeout": aggregate["readiness_fallback"].get("impact") == "blocking",
            "readiness_degraded": aggregate["readiness_fallback"].get("impact") == "degraded",
        },
    )
    aggregate["threshold_evaluation"] = _evaluate_threshold_status(
        fixture_tier=fixture_tier,
        aggregate=aggregate,
        policy_name=V2_THRESHOLD_POLICY,
    )

    if blockers and not per_query:
        warnings.append("benchmark blocked before query loop; structured blocker evidence written")

    output_json = Path(args.output_json)
    output_md = Path(args.output_md)
    using_canonical_targets = _is_canonical_target_pair(output_json, output_md)
    effective_output_json = output_json
    effective_output_md = output_md
    refresh_allowed = bool(args.refresh_baseline)
    if using_canonical_targets and not refresh_allowed:
        effective_output_json, effective_output_md = _non_refresh_report_paths(BENCHMARK_NAME, fixture_tier)
        warnings.append(
            "baseline refresh not requested; canonical baseline targets are protected and output was redirected"
        )

    refresh_metadata = {
        "schema_version": V2_SCHEMA_VERSION,
        "fixture_tier": fixture_tier,
        "baseline_version": args.baseline_version,
        "model": args.embedding_model,
        "command": _cli_command(list(getattr(args, "_argv", []))),
        "refresh_reason": args.refresh_reason,
        "refresh_mode": refresh_allowed,
        "refresh_requested": bool(args.refresh_baseline),
        "canonical_target_requested": using_canonical_targets,
        "canonical_target_written": using_canonical_targets and refresh_allowed,
        "output_json": str(effective_output_json),
        "output_md": str(effective_output_md),
        "refresh_timestamp_utc": datetime.now(timezone.utc).isoformat().replace("+00:00", "Z") if refresh_allowed else None,
        "git_commit": _git_commit_hash(),
    }

    if refresh_allowed and not args.refresh_reason:
        blockers.append(
            _blocker(
                phase="baseline_refresh_policy",
                command_or_tool="--refresh-baseline",
                message="explicit baseline refresh requires --refresh-reason",
                stderr_tail=stderr_tail,
                diagnostics={"required_field": "refresh_reason"},
            )
        )
        refresh_allowed = False
        refresh_metadata["refresh_mode"] = False
        refresh_metadata["canonical_target_written"] = False
        refresh_metadata["refresh_timestamp_utc"] = None
        if using_canonical_targets:
            effective_output_json, effective_output_md = _non_refresh_report_paths(BENCHMARK_NAME, fixture_tier)
            refresh_metadata["output_json"] = str(effective_output_json)
            refresh_metadata["output_md"] = str(effective_output_md)

    manifest = _fixture_manifest(
        memories=memories,
        graph_fixture=graph_fixture,
        queries=queries,
        command=resolved_command,
        fixture_tier=fixture_tier,
    )
    manifest["schema_version"] = V2_SCHEMA_VERSION
    manifest["fixture_tier"] = fixture_tier
    manifest["baseline_version"] = args.baseline_version
    manifest["threshold_policy"] = {"name": V2_THRESHOLD_POLICY, "enforcement": "local-only"}
    manifest["runtime_target"] = _runtime_target_for_tier(fixture_tier)
    manifest["determinism_policy"] = {"name": V2_DETERMINISM_POLICY}
    manifest["baseline_refresh"] = refresh_metadata
    environment = _environment(
        command=resolved_command,
        data_dir=data_dir_used,
        embedding_model=args.embedding_model,
        started_at=run_started,
        mode="benchmark",
        raw=raw,
        stderr_tail=stderr_tail,
    )
    _write_reports(
        output_json=effective_output_json,
        output_md=effective_output_md,
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
                "output_json": str(effective_output_json),
                "output_md": str(effective_output_md),
                "query_count": len(queries),
                "ran_queries": len(per_query),
                "blocker_count": len(blockers),
                "observed_reason_codes": aggregate["observed_summary_partial_reason_codes"],
                "baseline_refresh": refresh_metadata,
            },
            indent=2,
        )
    )
    return 0 if not blockers else 2


def parse_args(argv: Iterable[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Memory retrieval benchmark fixture + query execution utilities.")
    tier_help = "Fixture tier selection: small default; medium/stress explicit only."
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
    parser.add_argument("--fixture-tier", default=V2_DEFAULT_FIXTURE_TIER, help=tier_help)
    parser.add_argument("--tier", dest="fixture_tier", help="Alias for --fixture-tier.")
    parser.add_argument("--baseline-version", default=V2_DEFAULT_BASELINE_VERSION, help="Baseline version metadata for V2 artifacts.")
    parser.add_argument("--refresh-baseline", action="store_true", help="Allow canonical baseline output targets to be refreshed intentionally.")
    parser.add_argument("--refresh-reason", default=None, help="Required reason text when --refresh-baseline is used.")
    parser.add_argument(
        "--negative-metrics-snippet-out",
        type=Path,
        default=None,
        help="Write negative_no_match baseline metric snippet JSON for QA evidence.",
    )
    args = parser.parse_args(list(argv) if argv is not None else None)
    try:
        args.fixture_tier = _normalize_fixture_tier(args.fixture_tier)
    except ValueError as exc:
        parser.error(str(exc))
    return args


def main(argv: Iterable[str] | None = None) -> int:
    args = parse_args(argv)
    args._argv = list(argv) if argv is not None else ["evals/memory_retrieval_benchmark.py", *sys.argv[1:]]

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
