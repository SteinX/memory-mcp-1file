from __future__ import annotations

import argparse
import json
import math
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Iterable, Mapping, Sequence


REASON_CODE_EXPLANATIONS: dict[str, str] = {
    "missing": "Requested contract data was absent; inspect surrounding result/blocker evidence before treating it as a failure.",
    "stale": "Contract data is present but not current; usually degraded unless the query has fresh successful retrieval evidence.",
    "partial": "The response is intentionally partial; informational when retrieval/readiness succeeded, degraded or blocking only with supporting evidence.",
    "degraded": "The server explicitly reported degraded behavior.",
    "invalid_locator": "The caller supplied a locator that cannot be resolved.",
    "generation_mismatch": "The response was produced from a different generation than the caller expected.",
    "unsupported": "The requested contract feature or lookup mode is unsupported by this surface.",
}


def classify_reason_code(reason_code: Any, *, evidence: Mapping[str, Any] | None = None) -> dict[str, str]:
    code = str(reason_code) if reason_code is not None else "unknown"
    evidence = evidence or {}
    blocker_count = int(evidence.get("blocker_count") or 0)
    readiness_timeout = bool(evidence.get("readiness_timeout"))
    failure_type = str(evidence.get("failure_type") or "")
    retrieval_blocked = bool(evidence.get("retrieval_blocked")) or failure_type in {"call_error", "parse_error", "embedding_not_ready"}

    if code == "partial":
        if blocker_count > 0 or readiness_timeout or retrieval_blocked:
            classification = "blocking" if retrieval_blocked or readiness_timeout else "degraded"
            explanation = "Partial contract metadata coincides with readiness/query blocker evidence."
        elif bool(evidence.get("readiness_degraded")):
            classification = "degraded"
            explanation = "Partial contract metadata coincides with degraded readiness evidence."
        else:
            classification = "informational"
            explanation = REASON_CODE_EXPLANATIONS["partial"]
    elif code in {"degraded", "stale", "generation_mismatch"}:
        classification = "degraded"
        explanation = REASON_CODE_EXPLANATIONS.get(code, "Contract metadata indicates degraded semantics.")
    elif code in {"invalid_locator", "unsupported"}:
        classification = "blocking"
        explanation = REASON_CODE_EXPLANATIONS.get(code, "Contract metadata indicates the request cannot be satisfied as issued.")
    elif code == "missing":
        classification = "blocking" if blocker_count > 0 or retrieval_blocked else "informational"
        explanation = REASON_CODE_EXPLANATIONS["missing"]
    else:
        classification = "informational"
        explanation = "Unknown reason code; preserved as informational unless surrounding blocker evidence says otherwise."

    return {
        "reason_code": code,
        "classification": classification,
        "impact": classification,
        "explanation": explanation,
    }


def classify_reason_codes(reason_codes: Sequence[Any] | Iterable[Any], *, evidence: Mapping[str, Any] | None = None) -> dict[str, dict[str, str]]:
    codes = sorted({str(code) for code in reason_codes if isinstance(code, str) and code})
    return {code: classify_reason_code(code, evidence=evidence) for code in codes}


def classify_readiness_fallback(settle: Mapping[str, Any] | None) -> dict[str, Any]:
    if not isinstance(settle, Mapping):
        return {
            "status": "unavailable",
            "impact": "informational",
            "elapsed_s": None,
            "explanation": "No settle_readiness result was recorded for this run.",
        }

    status = str(settle.get("status") or "unknown")
    reason = str(settle.get("reason") or "")
    elapsed_s = settle.get("elapsed_s")
    fallback_used = status.startswith("fallback") or status == "timeout"
    if status == "ready":
        impact = "informational"
        explanation = "Readiness was confirmed by an explicit server signal."
    elif status == "timeout":
        impact = "blocking"
        explanation = "Readiness polling timed out; retrieval evidence may be incomplete."
    elif fallback_used:
        impact = "degraded"
        explanation = "Readiness used fallback settling because no direct ready signal was available; results can still be valid but should be reviewed with elapsed time."
    else:
        impact = "informational"
        explanation = "Readiness settling completed without a blocking fallback classification."

    return {
        "status": status,
        "reason": reason,
        "impact": impact,
        "classification": impact,
        "elapsed_s": elapsed_s,
        "fallback_used": fallback_used,
        "fallback_sleep_s": settle.get("fallback_sleep_s"),
        "poll_attempts": settle.get("poll_attempts"),
        "readiness_signal": settle.get("readiness_signal"),
        "explanation": explanation,
    }


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


def recall_at_k(
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
    hits = len({result_id for result_id in results if result_id in expected})
    return hits / float(len(expected))


def _dcg_at_k(result_ids: Sequence[Any] | Iterable[Any], expected_ids: Sequence[Any] | Iterable[Any], k: int) -> float:
    if k <= 0:
        return 0.0
    expected = _expected_id_set(expected_ids)
    if not expected:
        return 0.0

    score = 0.0
    seen_relevant: set[str] = set()
    for index, result_id in enumerate(_as_id_list(result_ids)[:k], start=1):
        if result_id in expected and result_id not in seen_relevant:
            seen_relevant.add(result_id)
            score += 1.0 / math.log2(index + 1)
    return score


def _ideal_dcg_at_k(relevant_count: int, k: int) -> float:
    if k <= 0 or relevant_count <= 0:
        return 0.0
    score = 0.0
    for index in range(1, min(k, relevant_count) + 1):
        score += 1.0 / math.log2(index + 1)
    return score


def ndcg_at_k(
    result_ids: Sequence[Any] | Iterable[Any],
    expected_ids: Sequence[Any] | Iterable[Any],
    k: int,
) -> float:
    if k <= 0:
        return 0.0
    expected = _expected_id_set(expected_ids)
    if not expected:
        return 0.0

    dcg = _dcg_at_k(result_ids, expected, k)
    ideal = _ideal_dcg_at_k(len(expected), k)
    if ideal <= 0.0:
        return 0.0
    return dcg / ideal


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
        row[f"recall_at_{int(k)}"] = recall_at_k(results, expected, int(k))
        row[f"ndcg_at_{int(k)}"] = ndcg_at_k(results, expected, int(k))
    row["hit_count"] = 0 if row["expected_rank"] is None else 1
    return row


def aggregate_metrics(per_query: Sequence[Mapping[str, Any]]) -> dict[str, Any]:
    rows = list(per_query)
    latencies = [row.get("latency_ms") for row in rows if row.get("latency_ms") is not None]
    precision_5 = [float(row.get("precision_at_5", 0.0)) for row in rows]
    precision_10 = [float(row.get("precision_at_10", 0.0)) for row in rows]
    recall_5 = [float(row.get("recall_at_5", 0.0)) for row in rows]
    recall_10 = [float(row.get("recall_at_10", 0.0)) for row in rows]
    ndcg_5 = [float(row.get("ndcg_at_5", 0.0)) for row in rows]
    ndcg_10 = [float(row.get("ndcg_at_10", 0.0)) for row in rows]
    reciprocal_ranks = [float(row.get("mrr", 0.0)) for row in rows]
    ranks = [int(row["expected_rank"]) for row in rows if row.get("expected_rank") is not None]
    hits = sum(1 for row in rows if row.get("expected_rank") is not None)
    summary = latency_summary(latencies)
    aggregate = {
        "query_count": len(rows),
        "hit_rate": (hits / len(rows)) if rows else 0.0,
        "precision_at_5": (sum(precision_5) / len(precision_5)) if precision_5 else 0.0,
        "precision_at_10": (sum(precision_10) / len(precision_10)) if precision_10 else 0.0,
        "recall_at_5": (sum(recall_5) / len(recall_5)) if recall_5 else 0.0,
        "recall_at_10": (sum(recall_10) / len(recall_10)) if recall_10 else 0.0,
        "ndcg_at_5": (sum(ndcg_5) / len(ndcg_5)) if ndcg_5 else 0.0,
        "ndcg_at_10": (sum(ndcg_10) / len(ndcg_10)) if ndcg_10 else 0.0,
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
    manifest_mapping = manifest if isinstance(manifest, Mapping) else {}
    threshold_policy_raw = manifest_mapping.get("threshold_policy")
    threshold_policy = None
    if isinstance(threshold_policy_raw, str):
        threshold_policy = threshold_policy_raw
    elif isinstance(threshold_policy_raw, Mapping):
        threshold_policy = str(threshold_policy_raw.get("name") or "") or None

    threshold_eval = aggregate_metrics_value.get("threshold_evaluation") if isinstance(aggregate_metrics_value, Mapping) else None
    threshold_status = "deferred"
    threshold_status_reason = "threshold evaluation not provided by benchmark harness"
    if isinstance(threshold_eval, Mapping):
        status_value = threshold_eval.get("status")
        if isinstance(status_value, str) and status_value:
            threshold_status = status_value
        reason_value = threshold_eval.get("reason")
        if isinstance(reason_value, str) and reason_value:
            threshold_status_reason = reason_value

    failure_buckets: dict[str, int] = {}
    for row in per_query:
        if not isinstance(row, Mapping):
            continue
        failure_type = row.get("failure_type")
        if not isinstance(failure_type, str) or not failure_type:
            continue
        failure_buckets[failure_type] = int(failure_buckets.get(failure_type, 0)) + 1

    reason_codes = aggregate_metrics_value.get("observed_summary_partial_reason_codes") if isinstance(aggregate_metrics_value, Mapping) else []
    if not isinstance(reason_codes, list):
        reason_codes = []
    reason_code_classification = aggregate_metrics_value.get("reason_code_classification") if isinstance(aggregate_metrics_value, Mapping) else {}
    if not isinstance(reason_code_classification, Mapping):
        reason_code_classification = {}
    readiness_fallback = aggregate_metrics_value.get("readiness_fallback") if isinstance(aggregate_metrics_value, Mapping) else None
    if not isinstance(readiness_fallback, Mapping):
        readiness_fallback = None

    baseline_diff_summary = aggregate_metrics_value.get("baseline_diff_summary") if isinstance(aggregate_metrics_value, Mapping) else None
    if not isinstance(baseline_diff_summary, Mapping):
        baseline_diff_summary = {
            "status": "deferred",
            "reason": "baseline diff summary is produced by explicit baseline-diff workflow",
        }

    metric_summary_keys = (
        "query_count",
        "hit_rate",
        "mrr",
        "precision_at_5",
        "precision_at_10",
        "recall_at_5",
        "recall_at_10",
        "ndcg_at_5",
        "ndcg_at_10",
        "mean_latency_ms",
        "max_latency_ms",
        "p95_latency_ms",
        "blocker_count",
        "positive_query_count",
        "positive_hit_rate",
        "positive_mean_mrr",
        "positive_mean_recall_at_5",
        "positive_mean_ndcg_at_5",
        "positive_mean_precision_at_5",
        "runtime_minutes",
    )
    metric_summary = {
        key: aggregate_metrics_value.get(key)
        for key in metric_summary_keys
        if key in aggregate_metrics_value
    }

    deterministic_local_metadata = {
        "threshold_policy_enforcement": "local-only",
        "determinism_policy": manifest_mapping.get("determinism_policy"),
        "runtime_target": manifest_mapping.get("runtime_target"),
    }

    payload = {
        "generated_at_utc": datetime.now(timezone.utc).isoformat().replace("+00:00", "Z"),
        "benchmark_name": benchmark_name,
        "manifest": manifest,
        "schema_version": manifest_mapping.get("schema_version"),
        "fixture_tier": manifest_mapping.get("fixture_tier"),
        "baseline_version": manifest_mapping.get("baseline_version"),
        "threshold_policy": threshold_policy,
        "threshold_status": threshold_status,
        "readiness_summary": {
            "taxonomy": {
                "reason_codes": sorted({str(code) for code in reason_codes if isinstance(code, str) and code}),
                "reason_code_classification": dict(reason_code_classification),
            },
            "readiness_fallback": dict(readiness_fallback) if readiness_fallback else None,
            "threshold_status_reason": threshold_status_reason,
        },
        "failure_buckets": dict(sorted(failure_buckets.items())),
        "baseline_diff_summary": dict(baseline_diff_summary),
        "metric_summary": metric_summary,
        "deterministic_local_metadata": deterministic_local_metadata,
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
    manifest_mapping = manifest if isinstance(manifest, Mapping) else {}
    threshold_policy_raw = manifest_mapping.get("threshold_policy")
    threshold_policy = None
    if isinstance(threshold_policy_raw, str):
        threshold_policy = threshold_policy_raw
    elif isinstance(threshold_policy_raw, Mapping):
        threshold_policy = str(threshold_policy_raw.get("name") or "") or None

    threshold_eval = aggregate_metrics_value.get("threshold_evaluation") if isinstance(aggregate_metrics_value, Mapping) else None
    threshold_status = "deferred"
    threshold_status_reason = "threshold evaluation not provided by benchmark harness"
    if isinstance(threshold_eval, Mapping):
        status_value = threshold_eval.get("status")
        if isinstance(status_value, str) and status_value:
            threshold_status = status_value
        reason_value = threshold_eval.get("reason")
        if isinstance(reason_value, str) and reason_value:
            threshold_status_reason = reason_value

    failure_buckets: dict[str, int] = {}
    for row in per_query:
        if not isinstance(row, Mapping):
            continue
        failure_type = row.get("failure_type")
        if not isinstance(failure_type, str) or not failure_type:
            continue
        failure_buckets[failure_type] = int(failure_buckets.get(failure_type, 0)) + 1

    baseline_diff_summary = aggregate_metrics_value.get("baseline_diff_summary") if isinstance(aggregate_metrics_value, Mapping) else None
    if not isinstance(baseline_diff_summary, Mapping):
        baseline_diff_summary = {
            "status": "deferred",
            "reason": "baseline diff summary is produced by explicit baseline-diff workflow",
        }

    lines.extend(["", "## Benchmark V2 summary", "", "| Field | Value |", "|---|---|"])
    lines.append(f"| schema_version | `{_format_value(manifest_mapping.get('schema_version'))}` |")
    lines.append(f"| fixture_tier | `{_format_value(manifest_mapping.get('fixture_tier'))}` |")
    lines.append(f"| baseline_version | `{_format_value(manifest_mapping.get('baseline_version'))}` |")
    lines.append(f"| threshold_policy | `{_format_value(threshold_policy)}` |")
    lines.append(f"| threshold_status | `{_format_value(threshold_status)}` |")
    lines.append(f"| threshold_status_reason | `{_format_value(threshold_status_reason)}` |")

    lines.extend(["", "### Readiness summary", ""])
    reason_codes = aggregate_metrics_value.get("observed_summary_partial_reason_codes")
    lines.append(f"- reason_codes: `{_format_value(reason_codes)}`")
    reason_code_classification = aggregate_metrics_value.get("reason_code_classification")
    lines.append(f"- reason_code_classification: `{_format_value(reason_code_classification)}`")
    readiness_fallback = aggregate_metrics_value.get("readiness_fallback")
    lines.append(f"- readiness_fallback: `{_format_value(readiness_fallback)}`")

    lines.extend(["", "### Failure buckets", "", "| Failure type | Count |", "|---|---:|"])
    if failure_buckets:
        for failure_type in sorted(failure_buckets):
            lines.append(f"| {failure_type} | {_format_value(failure_buckets[failure_type])} |")
    else:
        lines.append("| none | 0 |")

    lines.extend(["", "### Baseline diff summary", ""])
    lines.append(f"- status: `{_format_value(baseline_diff_summary.get('status'))}`")
    lines.append(f"- reason: `{_format_value(baseline_diff_summary.get('reason'))}`")
    if baseline_diff_summary.get("comparison") is not None:
        lines.append(f"- comparison: `{_format_value(baseline_diff_summary.get('comparison'))}`")

    lines.extend(["", "### Metric summary", "", "| Metric | Value |", "|---|---:|"])
    for key in (
        "query_count",
        "hit_rate",
        "mrr",
        "precision_at_5",
        "precision_at_10",
        "recall_at_5",
        "recall_at_10",
        "ndcg_at_5",
        "ndcg_at_10",
        "mean_latency_ms",
        "max_latency_ms",
        "p95_latency_ms",
        "blocker_count",
        "positive_query_count",
        "positive_hit_rate",
        "positive_mean_mrr",
        "positive_mean_recall_at_5",
        "positive_mean_ndcg_at_5",
        "positive_mean_precision_at_5",
        "runtime_minutes",
    ):
        if key in aggregate_metrics_value:
            lines.append(f"| {key} | {_format_value(aggregate_metrics_value.get(key))} |")

    lines.extend(["", "### Deterministic / local-only metadata", ""])
    lines.append("- threshold_policy_enforcement: `local-only`")
    lines.append(f"- determinism_policy: `{_format_value(manifest_mapping.get('determinism_policy'))}`")
    lines.append(f"- runtime_target: `{_format_value(manifest_mapping.get('runtime_target'))}`")

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

    reason_code_classification = aggregate_metrics_value.get("reason_code_classification")
    if isinstance(reason_code_classification, dict) and reason_code_classification:
        lines.extend(["", "## Reason code classification", "", "| Reason code | Impact | Explanation |", "|---|---|---|"])
        for code in sorted(reason_code_classification):
            row = reason_code_classification.get(code)
            if not isinstance(row, Mapping):
                continue
            lines.append(
                "| "
                + f"{_format_value(code)} | "
                + f"{_format_value(row.get('impact') or row.get('classification'))} | "
                + f"{_format_value(row.get('explanation'))} |"
            )

    readiness_fallback = aggregate_metrics_value.get("readiness_fallback")
    if isinstance(readiness_fallback, Mapping) and readiness_fallback:
        lines.extend(["", "## Readiness fallback", ""])
        lines.append(f"- Status: `{_format_value(readiness_fallback.get('status'))}`")
        lines.append(f"- Impact: `{_format_value(readiness_fallback.get('impact') or readiness_fallback.get('classification'))}`")
        lines.append(f"- Elapsed (s): `{_format_value(readiness_fallback.get('elapsed_s'))}`")
        lines.append(f"- Fallback used: `{_format_value(readiness_fallback.get('fallback_used'))}`")
        if readiness_fallback.get("explanation") is not None:
            lines.append(f"- Explanation: {_format_value(readiness_fallback.get('explanation'))}")

    if blockers:
        lines.extend(["", "## Blockers", ""])
        for blocker in blockers:
            lines.append(f"- {_format_value(blocker)}")

    lines.extend(
        [
            "",
            "## Per-query metrics",
            "",
            "| Query | Rank | MRR | R@5 | R@10 | NDCG@5 | NDCG@10 | P@5 | P@10 | Latency ms | Failure | Top-1 |",
            "|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|---|",
        ]
    )
    for index, row in enumerate(per_query, start=1):
        top_1 = "—"
        raw_top_k = row.get("raw_top_k")
        if isinstance(raw_top_k, list) and raw_top_k:
            first = raw_top_k[0]
            if isinstance(first, dict):
                first_id = first.get("result_id")
                first_fixture = first.get("fixture_id")
                top_1 = _format_value(first_id)
                if first_fixture:
                    top_1 = f"{top_1} ({_format_value(first_fixture)})"
        lines.append(
            "| "
            + f"{_format_value(row.get('query_id', index))} | "
            + f"{_format_value(row.get('expected_rank'))} | "
            + f"{_format_value(row.get('mrr'))} | "
            + f"{_format_value(row.get('recall_at_5'))} | "
            + f"{_format_value(row.get('recall_at_10'))} | "
            + f"{_format_value(row.get('ndcg_at_5'))} | "
            + f"{_format_value(row.get('ndcg_at_10'))} | "
            + f"{_format_value(row.get('precision_at_5'))} | "
            + f"{_format_value(row.get('precision_at_10'))} | "
            + f"{_format_value(row.get('latency_ms'))} | "
            + f"{_format_value(row.get('failure_type'))} | "
            + f"{top_1} |"
        )

    output_path = Path(path)
    output_path.parent.mkdir(parents=True, exist_ok=True)
    output_path.write_text("\n".join(lines) + "\n", encoding="utf-8")


REPO_ROOT = Path(__file__).resolve().parents[2]
EVAL_EVIDENCE_DIR = REPO_ROOT / ".sisyphus" / "evidence" / "evals"
V2_EVIDENCE_DIR = REPO_ROOT / ".sisyphus" / "evidence" / "benchmark-v2"
V2_BASELINE_DIFF_DIR = V2_EVIDENCE_DIR / "baseline-diff"
LEGACY_CODE_BASELINE_JSON = REPO_ROOT / ".sisyphus" / "evidence" / "task-2-recall-code-baseline.json"


def _load_json_report(path: str | Path | None) -> dict[str, Any] | None:
    if path is None:
        return None
    p = Path(path)
    if not p.exists():
        return None
    return json.loads(p.read_text(encoding="utf-8"))


def _aggregate_value(payload: Mapping[str, Any] | None, key: str) -> Any:
    if not isinstance(payload, Mapping):
        return None
    aggregate = payload.get("aggregate_metrics")
    if not isinstance(aggregate, Mapping):
        aggregate = {}
    value = aggregate.get(key)
    if value is None and key == "blocker_count":
        blockers = payload.get("blockers")
        if isinstance(blockers, list):
            return len(blockers)
    return value


def _numeric_delta(before: Any, after: Any) -> float | None:
    if isinstance(before, (int, float)) and isinstance(after, (int, float)):
        return float(after) - float(before)
    return None


_HIGHER_IS_BETTER_METRICS = {
    "hit_rate",
    "mrr",
    "precision_at_5",
    "precision_at_10",
    "recall_at_5",
    "recall_at_10",
    "ndcg_at_5",
    "ndcg_at_10",
}

_LOWER_IS_BETTER_METRICS = {
    "mean_latency_ms",
    "max_latency_ms",
    "p95_latency_ms",
    "blocker_count",
}


def _metric_delta(payload_before: Mapping[str, Any] | None, payload_after: Mapping[str, Any] | None, key: str) -> dict[str, Any]:
    before = _aggregate_value(payload_before, key)
    after = _aggregate_value(payload_after, key)
    return {
        "before": before,
        "after": after,
        "delta": _numeric_delta(before, after),
    }


def _classify_metric_change(metric_key: str, triplet: Mapping[str, Any]) -> str:
    before = triplet.get("before")
    after = triplet.get("after")
    delta = triplet.get("delta")
    if before is None and after is None:
        return "missing"
    if before is None:
        return "missing_before"
    if after is None:
        return "missing_after"
    if not isinstance(delta, (int, float)):
        return "changed"
    if abs(float(delta)) <= 1e-12:
        return "unchanged"
    if metric_key in _HIGHER_IS_BETTER_METRICS:
        return "improved" if float(delta) > 0 else "regressed"
    if metric_key in _LOWER_IS_BETTER_METRICS:
        return "improved" if float(delta) < 0 else "regressed"
    return "changed"


def _baseline_diff_summary(metrics: Mapping[str, Any]) -> dict[str, Any]:
    tracked_metrics = (
        "hit_rate",
        "mrr",
        "precision_at_5",
        "precision_at_10",
        "recall_at_5",
        "recall_at_10",
        "ndcg_at_5",
        "ndcg_at_10",
        "mean_latency_ms",
        "max_latency_ms",
        "p95_latency_ms",
        "blocker_count",
    )
    status_by_metric: dict[str, str] = {}
    improved: list[str] = []
    regressed: list[str] = []
    changed: list[str] = []
    unchanged: list[str] = []
    missing_before: list[str] = []
    missing_after: list[str] = []
    for metric_key in tracked_metrics:
        triplet = metrics.get(metric_key)
        if not isinstance(triplet, Mapping):
            continue
        status = _classify_metric_change(metric_key, triplet)
        status_by_metric[metric_key] = status
        if status == "improved":
            improved.append(metric_key)
            changed.append(metric_key)
        elif status == "regressed":
            regressed.append(metric_key)
            changed.append(metric_key)
        elif status == "changed":
            changed.append(metric_key)
        elif status == "unchanged":
            unchanged.append(metric_key)
        elif status == "missing_before":
            missing_before.append(metric_key)
        elif status == "missing_after":
            missing_after.append(metric_key)

    return {
        "changed": sorted(changed),
        "missing": {
            "before": sorted(missing_before),
            "after": sorted(missing_after),
        },
        "improved": sorted(improved),
        "regressed": sorted(regressed),
        "unchanged": sorted(unchanged),
        "metric_status": status_by_metric,
    }


def _reason_codes(payload: Mapping[str, Any] | None) -> list[str] | None:
    if not isinstance(payload, Mapping):
        return None
    value = _aggregate_value(payload, "observed_summary_partial_reason_codes")
    if not isinstance(value, list):
        return []
    return sorted({str(code) for code in value if isinstance(code, str) and code})


def _reason_code_delta(before_codes: list[str] | None, after_codes: list[str] | None) -> dict[str, Any] | None:
    if before_codes is None or after_codes is None:
        return None
    before_set = set(before_codes)
    after_set = set(after_codes)
    return {
        "added": sorted(after_set - before_set),
        "removed": sorted(before_set - after_set),
        "count_delta": len(after_set) - len(before_set),
    }


def _blocker_signatures(payload: Mapping[str, Any] | None) -> list[str] | None:
    if not isinstance(payload, Mapping):
        return None
    blockers = payload.get("blockers")
    if not isinstance(blockers, list):
        return []
    signatures: list[str] = []
    for blocker in blockers:
        if isinstance(blocker, Mapping):
            phase = blocker.get("phase")
            message = blocker.get("message") or blocker.get("error")
            if phase or message:
                signatures.append(f"{_format_value(phase)}: {_format_value(message)}")
                continue
        signatures.append(_format_value(blocker))
    return signatures


def _blocker_delta(before_blockers: list[str] | None, after_blockers: list[str] | None) -> dict[str, Any] | None:
    if before_blockers is None or after_blockers is None:
        return None
    before_set = set(before_blockers)
    after_set = set(after_blockers)
    return {
        "added": sorted(after_set - before_set),
        "removed": sorted(before_set - after_set),
        "count_delta": len(after_set) - len(before_set),
    }


def _build_baseline_diff(
    *,
    benchmark_name: str,
    before_path: str | Path | None,
    after_path: str | Path | None,
) -> dict[str, Any]:
    before_payload = _load_json_report(before_path)
    after_payload = _load_json_report(after_path)
    before_available = before_payload is not None
    after_available = after_payload is not None

    reason_codes_before = _reason_codes(before_payload)
    reason_codes_after = _reason_codes(after_payload)
    blocker_signatures_before = _blocker_signatures(before_payload)
    blocker_signatures_after = _blocker_signatures(after_payload)

    metrics = {
        "hit_rate": _metric_delta(before_payload, after_payload, "hit_rate"),
        "mrr": _metric_delta(before_payload, after_payload, "mrr"),
        "precision_at_5": _metric_delta(before_payload, after_payload, "precision_at_5"),
        "precision_at_10": _metric_delta(before_payload, after_payload, "precision_at_10"),
        "mean_latency_ms": _metric_delta(before_payload, after_payload, "mean_latency_ms"),
        "max_latency_ms": _metric_delta(before_payload, after_payload, "max_latency_ms"),
        "p95_latency_ms": _metric_delta(before_payload, after_payload, "p95_latency_ms"),
        "blocker_count": _metric_delta(before_payload, after_payload, "blocker_count"),
        "reason_codes": {
            "before": reason_codes_before,
            "after": reason_codes_after,
            "delta": _reason_code_delta(reason_codes_before, reason_codes_after),
        },
        "blockers": {
            "before": blocker_signatures_before,
            "after": blocker_signatures_after,
            "delta": _blocker_delta(blocker_signatures_before, blocker_signatures_after),
        },
    }

    comparison = _baseline_diff_summary(metrics)

    return {
        "generated_at_utc": datetime.now(timezone.utc).isoformat().replace("+00:00", "Z"),
        "benchmark_name": benchmark_name,
        "baseline_pair": {
            "before": str(before_path) if before_path is not None else None,
            "after": str(after_path) if after_path is not None else None,
            "before_available": before_available,
            "after_available": after_available,
        },
        "metrics": metrics,
        "comparison": comparison,
        "notes": [
            "Diff artifacts are evidence/reporting only.",
            "These deltas are informational and are not used as CI regression gates.",
        ],
    }


def write_baseline_diff_json(path: str | Path, diff_payload: Mapping[str, Any]) -> None:
    output_path = Path(path)
    output_path.parent.mkdir(parents=True, exist_ok=True)
    output_path.write_text(json.dumps(dict(diff_payload), indent=2, sort_keys=True) + "\n", encoding="utf-8")


def write_baseline_diff_markdown(path: str | Path, title: str, diff_payload: Mapping[str, Any]) -> None:
    baseline_pair = diff_payload.get("baseline_pair") if isinstance(diff_payload, Mapping) else {}
    metrics = diff_payload.get("metrics") if isinstance(diff_payload, Mapping) else {}
    comparison = diff_payload.get("comparison") if isinstance(diff_payload, Mapping) else {}
    lines: list[str] = [f"# {title}"]

    lines.extend(
        [
            "",
            "## Baseline pair",
            "",
            f"- Before: `{_format_value((baseline_pair or {}).get('before'))}`",
            f"- After: `{_format_value((baseline_pair or {}).get('after'))}`",
            f"- Before available: `{_format_value((baseline_pair or {}).get('before_available'))}`",
            f"- After available: `{_format_value((baseline_pair or {}).get('after_available'))}`",
            "",
            "## Metric deltas",
            "",
            "| Metric | Before | After | Delta |",
            "|---|---:|---:|---:|",
        ]
    )

    for key in (
        "hit_rate",
        "mrr",
        "precision_at_5",
        "precision_at_10",
        "recall_at_5",
        "recall_at_10",
        "ndcg_at_5",
        "ndcg_at_10",
        "mean_latency_ms",
        "max_latency_ms",
        "p95_latency_ms",
        "blocker_count",
    ):
        triplet = metrics.get(key) if isinstance(metrics, Mapping) else {}
        lines.append(
            "| "
            + f"{key} | "
            + f"{_format_value((triplet or {}).get('before'))} | "
            + f"{_format_value((triplet or {}).get('after'))} | "
            + f"{_format_value((triplet or {}).get('delta'))} |"
        )

    reason_codes = metrics.get("reason_codes") if isinstance(metrics, Mapping) else {}
    blockers = metrics.get("blockers") if isinstance(metrics, Mapping) else {}

    lines.extend(["", "## Reason codes", ""])
    lines.append(f"- before: `{_format_value((reason_codes or {}).get('before'))}`")
    lines.append(f"- after: `{_format_value((reason_codes or {}).get('after'))}`")
    lines.append(f"- delta: `{_format_value((reason_codes or {}).get('delta'))}`")

    lines.extend(["", "## Blockers", ""])
    lines.append(f"- before: `{_format_value((blockers or {}).get('before'))}`")
    lines.append(f"- after: `{_format_value((blockers or {}).get('after'))}`")
    lines.append(f"- delta: `{_format_value((blockers or {}).get('delta'))}`")

    lines.extend(["", "## Baseline change summary", ""])
    lines.append(f"- changed: `{_format_value((comparison or {}).get('changed'))}`")
    missing = (comparison or {}).get("missing") if isinstance(comparison, Mapping) else {}
    lines.append(f"- missing_before: `{_format_value((missing or {}).get('before'))}`")
    lines.append(f"- missing_after: `{_format_value((missing or {}).get('after'))}`")
    lines.append(f"- improved: `{_format_value((comparison or {}).get('improved'))}`")
    lines.append(f"- regressed: `{_format_value((comparison or {}).get('regressed'))}`")
    lines.append(f"- unchanged: `{_format_value((comparison or {}).get('unchanged'))}`")

    lines.extend(
        [
            "",
            "## Policy",
            "",
            "- Baseline diff is reporting/evidence only.",
            "- No CI gate or automated regression blocker is enforced by this diff output.",
        ]
    )

    output_path = Path(path)
    output_path.parent.mkdir(parents=True, exist_ok=True)
    output_path.write_text("\n".join(lines) + "\n", encoding="utf-8")


def _latest_memory_pre_remap_baseline() -> Path | None:
    candidates = sorted(EVAL_EVIDENCE_DIR.glob("memory-retrieval-baseline-pre-remap-*.json"))
    return candidates[-1] if candidates else None


def generate_baseline_diff_artifacts(
    *,
    memory_before: str | Path | None,
    memory_after: str | Path | None,
    memory_diff_json: str | Path,
    memory_diff_md: str | Path,
    code_before: str | Path | None,
    code_after: str | Path | None,
    code_diff_json: str | Path,
    code_diff_md: str | Path,
) -> dict[str, Any]:
    memory_diff = _build_baseline_diff(
        benchmark_name="memory_retrieval_baseline",
        before_path=memory_before,
        after_path=memory_after,
    )
    code_diff = _build_baseline_diff(
        benchmark_name="code_retrieval_baseline",
        before_path=code_before,
        after_path=code_after,
    )

    write_baseline_diff_json(memory_diff_json, memory_diff)
    write_baseline_diff_markdown(memory_diff_md, "Memory Retrieval Baseline Diff", memory_diff)
    write_baseline_diff_json(code_diff_json, code_diff)
    write_baseline_diff_markdown(code_diff_md, "Code Retrieval Baseline Diff", code_diff)

    return {
        "memory": {
            "json": str(memory_diff_json),
            "markdown": str(memory_diff_md),
            "baseline_pair": memory_diff.get("baseline_pair"),
        },
        "code": {
            "json": str(code_diff_json),
            "markdown": str(code_diff_md),
            "baseline_pair": code_diff.get("baseline_pair"),
        },
    }


def _self_test() -> None:
    assert expected_rank(["a", "b", "c"], ["x", "c"]) == 3
    assert expected_rank(["a", "b", "c"], ["z"]) is None
    assert precision_at_k(["a", "b", "c"], ["b"], 5) == 0.2
    assert precision_at_k(["a", "b", "c"], ["z"], 5) == 0.0
    assert recall_at_k(["a", "b", "c"], ["b", "c"], 1) == 0.0
    assert recall_at_k(["a", "b", "c"], ["b", "c"], 2) == 0.5
    assert recall_at_k(["a", "b", "c"], ["b", "c"], 3) == 1.0
    assert recall_at_k(["a", "b", "c"], [], 5) == 0.0
    assert round(ndcg_at_k(["a", "b", "c"], ["b", "c"], 2), 10) == round((1.0 / math.log2(3)) / (1.0 + (1.0 / math.log2(3))), 10)
    assert round(ndcg_at_k(["b", "c", "a"], ["b", "c"], 2), 10) == 1.0
    assert round(ndcg_at_k(["c", "b", "a"], ["b", "c"], 2), 10) == 1.0
    assert ndcg_at_k(["a", "b", "c"], [], 5) == 0.0
    assert mrr(["a", "b", "c"], ["b"]) == 0.5
    assert mrr(["a", "b", "c"], ["z"]) == 0.0

    negative_row = compute_query_metrics(["a", "b"], [], negative=True, query_type="negative_no_match", latency_ms=7.5)
    assert negative_row["expected_rank"] is None
    assert negative_row["mrr"] == 0.0
    assert negative_row["precision_at_5"] == 0.0
    assert negative_row["recall_at_5"] == 0.0
    assert negative_row["ndcg_at_5"] == 0.0
    assert negative_row["negative"] is True

    per_query = [
        {**compute_query_metrics(["x", "y"], ["y"], latency_ms=10.0), "failure_type": "none"},
        {
            **compute_query_metrics(["a", "b"], [], negative=True, query_type="negative_no_match", latency_ms=20.0),
            "failure_type": "true_miss",
        },
        {
            **compute_query_metrics([], ["z"], latency_ms=30.0),
            "failure_type": "empty_results",
        },
        {
            **compute_query_metrics([], [], negative=True, query_type="negative_no_match", latency_ms=40.0),
            "failure_type": "expected_no_match",
        },
    ]
    aggregate = aggregate_metrics(per_query)
    assert aggregate["query_count"] == 4
    assert aggregate["precision_at_5"] == 0.05
    assert aggregate["recall_at_5"] == 0.25
    assert round(aggregate["ndcg_at_5"], 12) == round((0.6309297535714575 + 0.0 + 0.0 + 0.0) / 4.0, 12)
    assert aggregate["mrr"] == 0.125
    assert aggregate["mean_latency_ms"] == 25.0
    assert aggregate["max_latency_ms"] == 40.0
    assert aggregate["p95_latency_ms"] == 40.0
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
    assert "threshold_status" in report
    assert "readiness_summary" in report
    assert "failure_buckets" in report
    assert report["failure_buckets"] == {
        "empty_results": 1,
        "expected_no_match": 1,
        "none": 1,
        "true_miss": 1,
    }
    assert "baseline_diff_summary" in report
    assert "metric_summary" in report
    assert "deterministic_local_metadata" in report

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
    assert "## Benchmark V2 summary" in markdown
    assert "### Failure buckets" in markdown
    assert "| expected_no_match | 1 |" in markdown
    assert "| true_miss | 1 |" in markdown

    diff_payload = _build_baseline_diff(
        benchmark_name="self_test",
        before_path=None,
        after_path=payload_path,
    )
    assert diff_payload["baseline_pair"]["before_available"] is False
    assert diff_payload["baseline_pair"]["after_available"] is True
    assert "hit_rate" in diff_payload["metrics"]
    assert "comparison" in diff_payload
    assert {"changed", "missing", "improved", "regressed", "unchanged"}.issubset(set(diff_payload["comparison"]))

    diff_json = payload_path.with_name("metrics-self-test-diff.json")
    diff_md = payload_path.with_name("metrics-self-test-diff.md")
    write_baseline_diff_json(diff_json, diff_payload)
    write_baseline_diff_markdown(diff_md, "Metrics Self-Test Diff", diff_payload)
    loaded_diff = json.loads(diff_json.read_text(encoding="utf-8"))
    assert loaded_diff["benchmark_name"] == "self_test"
    assert "Metric deltas" in diff_md.read_text(encoding="utf-8")


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Pure-Python metrics helpers for memory retrieval benchmarks.")
    parser.add_argument("--self-test", action="store_true", help="Run built-in correctness checks.")
    parser.add_argument("--baseline-diff", action="store_true", help="Generate baseline diff JSON/Markdown artifacts for memory and code benchmarks.")
    parser.add_argument("--memory-before", type=Path, default=None, help="Path to memory benchmark baseline BEFORE JSON.")
    parser.add_argument("--memory-after", type=Path, default=EVAL_EVIDENCE_DIR / "memory-retrieval-baseline.json", help="Path to memory benchmark baseline AFTER JSON.")
    parser.add_argument("--memory-diff-json", type=Path, default=V2_BASELINE_DIFF_DIR / "memory-retrieval-baseline-diff.json", help="Output path for memory baseline diff JSON.")
    parser.add_argument("--memory-diff-md", type=Path, default=V2_BASELINE_DIFF_DIR / "memory-retrieval-baseline-diff.md", help="Output path for memory baseline diff Markdown.")
    parser.add_argument("--code-before", type=Path, default=LEGACY_CODE_BASELINE_JSON, help="Path to code benchmark baseline BEFORE JSON.")
    parser.add_argument("--code-after", type=Path, default=EVAL_EVIDENCE_DIR / "code-retrieval-baseline.json", help="Path to code benchmark baseline AFTER JSON.")
    parser.add_argument("--code-diff-json", type=Path, default=V2_BASELINE_DIFF_DIR / "code-retrieval-baseline-diff.json", help="Output path for code baseline diff JSON.")
    parser.add_argument("--code-diff-md", type=Path, default=V2_BASELINE_DIFF_DIR / "code-retrieval-baseline-diff.md", help="Output path for code baseline diff Markdown.")
    args = parser.parse_args(list(argv) if argv is not None else None)
    if args.self_test:
        _self_test()
        print("self-test passed")
        return 0
    if args.baseline_diff:
        memory_before = args.memory_before if args.memory_before is not None else _latest_memory_pre_remap_baseline()
        outputs = generate_baseline_diff_artifacts(
            memory_before=memory_before,
            memory_after=args.memory_after,
            memory_diff_json=args.memory_diff_json,
            memory_diff_md=args.memory_diff_md,
            code_before=args.code_before,
            code_after=args.code_after,
            code_diff_json=args.code_diff_json,
            code_diff_md=args.code_diff_md,
        )
        print(json.dumps(outputs, indent=2, sort_keys=True))
        return 0
    parser.print_help()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
