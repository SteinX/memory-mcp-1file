from __future__ import annotations

import argparse
import json
import math
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Iterable, Mapping, Sequence


def _as_id_list(values: Sequence[Any] | Iterable[Any]) -> list[str]:
    return [str(value) for value in values]


def _expected_id_set(expected_ids: Sequence[Any] | Iterable[Any]) -> set[str]:
    return set(_as_id_list(expected_ids))


def expected_rank(result_ids: Sequence[Any] | Iterable[Any], expected_ids: Sequence[Any] | Iterable[Any]) -> int | None:
    expected = _expected_id_set(expected_ids)
    if not expected:
        return None
    for index, result_id in enumerate(_as_id_list(result_ids), start=1):
        if result_id in expected:
            return index
    return None


def precision_at_k(
    result_ids: Sequence[Any] | Iterable[Any],
    expected_ids: Sequence[Any] | Iterable[Any],
    k: int,
) -> float:
    if k <= 0:
        return 0.0
    expected = _expected_id_set(expected_ids)
    if not expected:
        return 0.0
    results = _as_id_list(result_ids)[:k]
    hits = sum(1 for result_id in results if result_id in expected)
    return hits / float(k)


def mrr(result_ids: Sequence[Any] | Iterable[Any], expected_ids: Sequence[Any] | Iterable[Any]) -> float:
    rank = expected_rank(result_ids, expected_ids)
    if rank is None:
        return 0.0
    return 1.0 / float(rank)


def latency_summary(latencies_ms: Sequence[Any] | Iterable[Any]) -> dict[str, float | int]:
    values = [float(value) for value in latencies_ms if value is not None]
    if not values:
        return {
            "count": 0,
            "mean_latency_ms": 0.0,
            "max_latency_ms": 0.0,
            "p95_latency_ms": 0.0,
        }
    values.sort()
    p95_index = max(0, min(len(values) - 1, math.ceil(len(values) * 0.95) - 1))
    return {
        "count": len(values),
        "mean_latency_ms": sum(values) / len(values),
        "max_latency_ms": values[-1],
        "p95_latency_ms": values[p95_index],
    }


def compute_query_metrics(
    result_ids: Sequence[Any] | Iterable[Any],
    expected_ids: Sequence[Any] | Iterable[Any],
    *,
    ks: Sequence[int] = (5, 10),
    query_type: str | None = None,
    latency_ms: float | int | None = None,
    negative: bool = False,
) -> dict[str, Any]:
    results = _as_id_list(result_ids)
    expected = _as_id_list(expected_ids)
    unique_expected = _expected_id_set(expected)
    row: dict[str, Any] = {
        "query_type": query_type,
        "negative": negative,
        "result_count": len(results),
        "expected_count": len(unique_expected),
        "expected_rank": expected_rank(results, expected),
        "mrr": mrr(results, expected),
        "latency_ms": float(latency_ms) if latency_ms is not None else None,
    }
    for k in ks:
        row[f"precision_at_{int(k)}"] = precision_at_k(results, expected, int(k))
    row["hit_count"] = 0 if row["expected_rank"] is None else 1
    return row


def aggregate_metrics(per_query: Sequence[Mapping[str, Any]]) -> dict[str, Any]:
    rows = list(per_query)
    latencies = [row.get("latency_ms") for row in rows if row.get("latency_ms") is not None]
    precision_5 = [float(row.get("precision_at_5", 0.0)) for row in rows]
    precision_10 = [float(row.get("precision_at_10", 0.0)) for row in rows]
    reciprocal_ranks = [float(row.get("mrr", 0.0)) for row in rows]
    ranks = [int(row["expected_rank"]) for row in rows if row.get("expected_rank") is not None]
    hits = sum(1 for row in rows if row.get("expected_rank") is not None)
    summary = latency_summary(latencies)
    aggregate = {
        "query_count": len(rows),
        "hit_rate": (hits / len(rows)) if rows else 0.0,
        "precision_at_5": (sum(precision_5) / len(precision_5)) if precision_5 else 0.0,
        "precision_at_10": (sum(precision_10) / len(precision_10)) if precision_10 else 0.0,
        "mrr": (sum(reciprocal_ranks) / len(reciprocal_ranks)) if reciprocal_ranks else 0.0,
        "mean_expected_rank": (sum(ranks) / len(ranks)) if ranks else None,
        "latency_summary": summary,
        "mean_latency_ms": summary["mean_latency_ms"],
        "max_latency_ms": summary["max_latency_ms"],
        "p95_latency_ms": summary["p95_latency_ms"],
    }
    return aggregate


def write_json_report(
    path: str | Path,
    benchmark_name: str,
    manifest: Any,
    aggregate_metrics_value: Mapping[str, Any],
    per_query: Sequence[Mapping[str, Any]],
    *,
    warnings: Sequence[Any] | None = None,
    blockers: Sequence[Any] | None = None,
    stderr_tail: Sequence[Any] | None = None,
    environment: Mapping[str, Any] | None = None,
) -> None:
    payload = {
        "generated_at_utc": datetime.now(timezone.utc).isoformat().replace("+00:00", "Z"),
        "benchmark_name": benchmark_name,
        "manifest": manifest,
        "aggregate_metrics": dict(aggregate_metrics_value),
        "per_query": list(per_query),
        "warnings": list(warnings or []),
        "blockers": list(blockers or []),
        "stderr_tail": [str(line) for line in (stderr_tail or []) if str(line).strip()],
    }
    if environment is not None:
        payload["environment"] = dict(environment)

    output_path = Path(path)
    output_path.parent.mkdir(parents=True, exist_ok=True)
    output_path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def _format_value(value: Any) -> str:
    if value is None:
        return "—"
    if isinstance(value, float):
        return f"{value:.4f}".rstrip("0").rstrip(".")
    if isinstance(value, (dict, list, tuple)):
        return json.dumps(value, sort_keys=True)
    return str(value)


def write_markdown_report(
    path: str | Path,
    title: str,
    aggregate_metrics_value: Mapping[str, Any],
    per_query: Sequence[Mapping[str, Any]],
    *,
    manifest: Mapping[str, Any] | None = None,
    environment: Mapping[str, Any] | None = None,
    stderr_tail: Sequence[Any] | None = None,
    warnings: Sequence[Any] | None = None,
    blockers: Sequence[Any] | None = None,
) -> None:
    lines: list[str] = [f"# {title}"]
    command_used = None
    if manifest and isinstance(manifest.get("command_selected"), list) and manifest.get("command_selected"):
        command_used = " ".join(str(part) for part in manifest["command_selected"])
    elif environment and isinstance(environment.get("server_command"), list) and environment.get("server_command"):
        command_used = " ".join(str(part) for part in environment["server_command"])

    if command_used or environment or stderr_tail:
        lines.extend(["", "## Run context"])
        if command_used:
            lines.extend(["", f"- Command used: `{command_used}`"])
        if environment and environment.get("embedding_model") is not None:
            lines.append(f"- Embedding model: `{_format_value(environment.get('embedding_model'))}`")
        if environment and environment.get("data_dir") is not None:
            lines.append(f"- Data dir: `{_format_value(environment.get('data_dir'))}`")
        if environment and environment.get("started_at_utc") is not None:
            lines.append(f"- Started at: `{_format_value(environment.get('started_at_utc'))}`")
        if environment and environment.get("duration_seconds") is not None:
            lines.append(f"- Duration (s): `{_format_value(environment.get('duration_seconds'))}`")
        if stderr_tail:
            lines.extend(["", "### stderr tail"])
            for line in stderr_tail:
                lines.append(f"- {_format_value(line)}")

    lines.extend(["", "## Aggregate metrics", "", "| Metric | Value |", "|---|---:|"])
    for key in sorted(aggregate_metrics_value):
        lines.append(f"| {key} | {_format_value(aggregate_metrics_value[key])} |")

    if warnings:
        lines.extend(["", "## Warnings", ""])
        for warning in warnings:
            lines.append(f"- {_format_value(warning)}")

    if blockers:
        lines.extend(["", "## Blockers", ""])
        for blocker in blockers:
            lines.append(f"- {_format_value(blocker)}")

    lines.extend(["", "## Per-query metrics", "", "| Query | Rank | MRR | P@5 | P@10 | Latency ms |", "|---|---:|---:|---:|---:|---:|"])
    for index, row in enumerate(per_query, start=1):
        lines.append(
            "| "
            + f"{_format_value(row.get('query_id', index))} | "
            + f"{_format_value(row.get('expected_rank'))} | "
            + f"{_format_value(row.get('mrr'))} | "
            + f"{_format_value(row.get('precision_at_5'))} | "
            + f"{_format_value(row.get('precision_at_10'))} | "
            + f"{_format_value(row.get('latency_ms'))} |"
        )

    output_path = Path(path)
    output_path.parent.mkdir(parents=True, exist_ok=True)
    output_path.write_text("\n".join(lines) + "\n", encoding="utf-8")


def _self_test() -> None:
    assert expected_rank(["a", "b", "c"], ["x", "c"]) == 3
    assert expected_rank(["a", "b", "c"], ["z"]) is None
    assert precision_at_k(["a", "b", "c"], ["b"], 5) == 0.2
    assert precision_at_k(["a", "b", "c"], ["z"], 5) == 0.0
    assert mrr(["a", "b", "c"], ["b"]) == 0.5
    assert mrr(["a", "b", "c"], ["z"]) == 0.0

    negative_row = compute_query_metrics(["a", "b"], [], negative=True, query_type="negative_no_match", latency_ms=7.5)
    assert negative_row["expected_rank"] is None
    assert negative_row["mrr"] == 0.0
    assert negative_row["precision_at_5"] == 0.0
    assert negative_row["negative"] is True

    per_query = [
        compute_query_metrics(["x", "y"], ["y"], latency_ms=10.0),
        compute_query_metrics(["a", "b"], [], negative=True, query_type="negative_no_match", latency_ms=20.0),
    ]
    aggregate = aggregate_metrics(per_query)
    assert aggregate["query_count"] == 2
    assert aggregate["precision_at_5"] == 0.1
    assert aggregate["mrr"] == 0.25
    assert aggregate["mean_latency_ms"] == 15.0
    assert aggregate["max_latency_ms"] == 20.0
    assert aggregate["p95_latency_ms"] == 20.0
    assert aggregate["mean_expected_rank"] == 2.0

    sample_manifest = {"command_selected": ["memory-mcp", "--stdio"]}
    sample_environment = {"server_command": ["memory-mcp", "--stdio"], "embedding_model": "e5_small", "data_dir": "/tmp/data"}
    with Path("/tmp").joinpath("metrics-self-test.json").open("w", encoding="utf-8") as handle:
        payload_path = Path(handle.name)
    write_json_report(
        payload_path,
        "metrics_self_test",
        sample_manifest,
        aggregate,
        per_query,
        warnings=[],
        blockers=[],
        stderr_tail=[],
        environment=sample_environment,
    )
    report = json.loads(payload_path.read_text(encoding="utf-8"))
    assert report["warnings"] == []
    assert report["blockers"] == []
    assert report["stderr_tail"] == []
    assert report["manifest"] == sample_manifest
    assert report["environment"] == sample_environment

    markdown_path = payload_path.with_suffix(".md")
    write_markdown_report(
        markdown_path,
        "Metrics Self-Test",
        aggregate,
        per_query,
        manifest=sample_manifest,
        environment=sample_environment,
        stderr_tail=[],
        warnings=[],
        blockers=[],
    )
    markdown = markdown_path.read_text(encoding="utf-8")
    assert "## Run context" in markdown
    assert "Command used:" in markdown


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Pure-Python metrics helpers for memory retrieval benchmarks.")
    parser.add_argument("--self-test", action="store_true", help="Run built-in correctness checks.")
    args = parser.parse_args(list(argv) if argv is not None else None)
    if args.self_test:
        _self_test()
        print("self-test passed")
        return 0
    parser.print_help()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
