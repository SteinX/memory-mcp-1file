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
from evals.lib.metrics import aggregate_metrics, compute_query_metrics, write_json_report, write_markdown_report


ROOT = PROJECT_ROOT
BASELINE_JSON = ROOT / ".sisyphus" / "evidence" / "task-2-recall-code-baseline.json"
EVIDENCE_DIR = ROOT / ".sisyphus" / "evidence" / "evals"
OUTPUT_JSON = EVIDENCE_DIR / "code-retrieval-baseline.json"
OUTPUT_MD = EVIDENCE_DIR / "code-retrieval-baseline.md"
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


def _load_baseline_query_set(path: Path = BASELINE_JSON) -> list[dict[str, Any]]:
    payload = json.loads(path.read_text(encoding="utf-8"))
    query_set = payload.get("query_set")
    if not isinstance(query_set, list):
        raise TypeError(f"Invalid baseline query set shape in {path}: expected list at query_set")
    if not query_set:
        raise ValueError(f"Baseline query set is empty in {path}")
    return query_set


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
    return {
        "phase": phase,
        "command_or_tool": command_or_tool,
        "message": message,
        "stderr_tail": [str(line) for line in stderr_tail if str(line).strip()],
        "summary_partial_reason_code": reason_code,
    }


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


def run_self_test() -> int:
    query_set = _load_baseline_query_set(BASELINE_JSON)
    print(
        f"self-test: loaded baseline query count={len(query_set)} "
        f"from {BASELINE_JSON.relative_to(ROOT)}"
    )

    sample_rows: list[dict[str, Any]] = []
    for idx, query in enumerate(query_set[:4], start=1):
        expected_paths = [str(p) for p in query.get("expected_paths", [])]
        if expected_paths:
            mock_results = [expected_paths[0], "src/graph/ppr.rs", "src/server/handler.rs"]
        else:
            mock_results = ["src/graph/ppr.rs", "src/server/handler.rs"]
        metrics_row = compute_query_metrics(
            mock_results,
            expected_paths,
            query_type=str(query.get("query_type")),
            latency_ms=100.0 + (idx * 5.0),
            negative=str(query.get("query_type")) == "negative_no_match",
        )
        sample_rows.append(
            {
                "query_id": query.get("query_id", f"q{idx}"),
                "query_type": query.get("query_type"),
                **metrics_row,
            }
        )

    aggregate = aggregate_metrics(sample_rows)
    with tempfile.TemporaryDirectory(prefix="task-9-code-self-test-") as tmp:
        tmp_path = Path(tmp)
        json_path = tmp_path / "self-test.json"
        md_path = tmp_path / "self-test.md"
        write_json_report(
            json_path,
            "code_retrieval_benchmark_self_test",
            {"source": str(BASELINE_JSON.relative_to(ROOT)), "query_count": len(query_set)},
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
            manifest={"source": str(BASELINE_JSON.relative_to(ROOT)), "query_count": len(query_set)},
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
        if "Code Retrieval Self-Test" not in md_path.read_text(encoding="utf-8"):
            raise AssertionError("self-test markdown title missing")

    print("self-test passed")
    return 0


def run_benchmark(args: argparse.Namespace) -> int:
    EVIDENCE_DIR.mkdir(parents=True, exist_ok=True)
    query_set = _load_baseline_query_set(BASELINE_JSON)
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
                            "query_error": query_error,
                            "symbol_probe": symbol_probe,
                            **metrics_row,
                        }
                    )
                    raw[f"query_{idx:02d}_{query_id}"] = payload

            raw["project_info_stats"] = _call_tool(client, "project_info", {"action": "stats", "project_id": project_id}, timeout=60)
            stderr_tail = client.stderr_tail(80)

    aggregate = aggregate_metrics(per_query)
    aggregate["baseline_query_count"] = len(query_set)
    aggregate["readiness_timeout"] = bool(any(b.get("phase") in {"embedding_readiness", "index_readiness"} for b in blockers))
    aggregate["observed_summary_partial_reason_codes"] = sorted(observed_reason_codes)

    environment = {
        "root": str(ROOT),
        "server_command": resolved_command,
        "baseline_source": str(BASELINE_JSON.relative_to(ROOT)),
        "required_source_paths": list(REQUIRED_SOURCE_PATHS),
        "data_dir_strategy": "temporary isolated DATA_DIR",
        "fixture_project_strategy": "temporary copied fixture project",
        "started_at_utc": datetime.fromtimestamp(run_started, tz=timezone.utc).isoformat(),
        "duration_seconds": round(time.time() - run_started, 2),
    }

    manifest = {
        "task": "9. Migrate code retrieval benchmark to evals",
        "query_set_source": str(BASELINE_JSON.relative_to(ROOT)),
        "query_count": len(query_set),
        "tools_used": ["index_project", "project_info(status)", "recall_code", "search_symbols", "symbol_graph"],
    }

    output_json = Path(args.output_json)
    output_md = Path(args.output_md)
    write_json_report(
        output_json,
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
        output_md,
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
                "output_json": str(output_json),
                "output_md": str(output_md),
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
    return parser.parse_args(list(argv) if argv is not None else None)


def main(argv: Iterable[str] | None = None) -> int:
    args = parse_args(argv)
    if args.self_test:
        return run_self_test()
    return run_benchmark(args)


if __name__ == "__main__":
    raise SystemExit(main())
