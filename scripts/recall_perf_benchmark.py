#!/usr/bin/env python3
"""Read-only MCP recall latency micro-benchmark.

This script targets a running Streamable HTTP MCP server. It does not mutate
server state; it only calls read-only retrieval/status tools and writes a local
JSON report when requested.
"""

from __future__ import annotations

import argparse
import json
import signal
import statistics
import time
import urllib.error
import urllib.request
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


class ScenarioTimeoutError(TimeoutError):
    pass


def _parse_response_body(body: bytes) -> dict[str, Any]:
    text = body.decode("utf-8", errors="replace").strip()
    if not text:
        return {}
    try:
        value = json.loads(text)
        return value if isinstance(value, dict) else {"payload": value}
    except json.JSONDecodeError:
        pass

    data_lines = []
    for line in text.splitlines():
        if line.startswith("data:"):
            payload = line.removeprefix("data:").strip()
            if payload:
                data_lines.append(payload)
    for payload in reversed(data_lines):
        try:
            value = json.loads(payload)
            return value if isinstance(value, dict) else {"payload": value}
        except json.JSONDecodeError:
            continue
    raise ValueError(f"response is neither JSON nor SSE JSON: {text[:500]}")


def _parse_sse_response(response: Any) -> dict[str, Any]:
    data_lines: list[str] = []
    while True:
        line = response.readline()
        if not line:
            break
        text = line.decode("utf-8", errors="replace").strip()
        if not text:
            if data_lines:
                payload = "\n".join(data_lines)
                value = json.loads(payload)
                return value if isinstance(value, dict) else {"payload": value}
            continue
        if text.startswith(":"):
            continue
        if text.startswith("data:"):
            payload = text.removeprefix("data:").strip()
            if payload:
                data_lines.append(payload)
    if data_lines:
        payload = "\n".join(data_lines)
        value = json.loads(payload)
        return value if isinstance(value, dict) else {"payload": value}
    return {}


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
    return parsed if isinstance(parsed, dict) else {"payload": parsed}


class HttpMcpClient:
    def __init__(self, endpoint: str, timeout: float) -> None:
        self.endpoint = endpoint.rstrip("/")
        self.timeout = timeout
        self.session_id: str | None = None
        self.next_id = 1

    def request(self, method: str, params: dict[str, Any] | None = None) -> dict[str, Any]:
        request_id = self.next_id
        self.next_id += 1
        payload: dict[str, Any] = {"jsonrpc": "2.0", "id": request_id, "method": method}
        if params is not None:
            payload["params"] = params
        return self._post(payload)

    def notify(self, method: str, params: dict[str, Any] | None = None) -> dict[str, Any]:
        payload: dict[str, Any] = {"jsonrpc": "2.0", "method": method}
        if params is not None:
            payload["params"] = params
        return self._post(payload)

    def _post(self, payload: dict[str, Any]) -> dict[str, Any]:
        headers = {
            "content-type": "application/json",
            "accept": "application/json, text/event-stream",
        }
        if self.session_id:
            headers["mcp-session-id"] = self.session_id
        request = urllib.request.Request(
            self.endpoint,
            data=json.dumps(payload).encode("utf-8"),
            headers=headers,
            method="POST",
        )
        try:
            with urllib.request.urlopen(request, timeout=self.timeout) as response:
                session = response.headers.get("mcp-session-id")
                if session:
                    self.session_id = session.strip()
                content_type = response.headers.get("content-type", "")
                if "text/event-stream" in content_type:
                    return _parse_sse_response(response)
                return _parse_response_body(response.read())
        except urllib.error.HTTPError as error:
            body = error.read().decode("utf-8", errors="replace")
            raise RuntimeError(f"HTTP {error.code} from MCP server: {body}") from error

    def initialize(self) -> None:
        self.request(
            "initialize",
            {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "recall-perf-benchmark", "version": "0.1.0"},
            },
        )
        self.notify("notifications/initialized")

    def call_tool(self, name: str, arguments: dict[str, Any]) -> dict[str, Any]:
        response = self.request("tools/call", {"name": name, "arguments": arguments})
        return _tool_payload(response)


@dataclass(frozen=True)
class Scenario:
    name: str
    tool: str
    arguments: dict[str, Any]


def _percentile(values: list[float], q: float) -> float:
    if not values:
        return 0.0
    ordered = sorted(values)
    index = int(round((len(ordered) - 1) * q))
    return ordered[index]


def _extract_summary(tool: str, payload: dict[str, Any]) -> dict[str, Any]:
    summary = payload.get("summary") if isinstance(payload.get("summary"), dict) else {}
    diagnostics = payload.get("diagnostics") if isinstance(payload.get("diagnostics"), dict) else {}
    out: dict[str, Any] = {
        "count": payload.get("count"),
        "is_partial": payload.get("is_partial"),
    }
    if tool == "recall_code":
        out.update(
            {
                "bm25_hits": summary.get("bm25_hits"),
                "vector_hits": summary.get("vector_hits"),
                "fallback_path": summary.get("fallback_path"),
                "serving_generation": summary.get("serving_generation"),
                "indexing_generation": summary.get("indexing_generation"),
            }
        )
    if tool == "recall":
        out.update(
            {
                "bm25_hits": diagnostics.get("bm25_hits"),
                "vector_hits": diagnostics.get("vector_hits"),
                "ppr_hits": diagnostics.get("ppr_hits"),
                "bm25_retrieved_candidates": diagnostics.get("bm25_retrieved_candidates"),
                "vector_retrieved_candidates": diagnostics.get("vector_retrieved_candidates"),
            }
        )
    return out


def _run_scenario(
    client: HttpMcpClient,
    scenario: Scenario,
    warmup: int,
    iterations: int,
) -> dict[str, Any]:
    samples: list[float] = []
    last_payload: dict[str, Any] = {}
    for i in range(warmup + iterations):
        started = time.perf_counter()
        payload = client.call_tool(scenario.tool, scenario.arguments)
        elapsed_ms = (time.perf_counter() - started) * 1000.0
        last_payload = payload
        if i >= warmup:
            samples.append(elapsed_ms)
    return {
        "name": scenario.name,
        "tool": scenario.tool,
        "arguments": scenario.arguments,
        "iterations": iterations,
        "warmup": warmup,
        "latency_ms": {
            "min": min(samples) if samples else 0.0,
            "max": max(samples) if samples else 0.0,
            "mean": statistics.fmean(samples) if samples else 0.0,
            "median": statistics.median(samples) if samples else 0.0,
            "p95": _percentile(samples, 0.95),
            "samples": samples,
        },
        "last_summary": _extract_summary(scenario.tool, last_payload),
    }


def _scenarios(project_id: str) -> list[Scenario]:
    return [
        Scenario(
            "code_recall_exact_symbol",
            "recall_code",
            {
                "project_id": project_id,
                "query": "RCChatSearchTabView",
                "limit": 10,
            },
        ),
        Scenario(
            "code_recall_broad_flow",
            "recall_code",
            {
                "project_id": project_id,
                "query": "RCElevatorComponent scrollSeqTo",
                "limit": 10,
            },
        ),
        Scenario(
            "memory_recall_unfiltered",
            "recall",
            {
                "query": "code search generated source indexing generation",
                "limit": 10,
            },
        ),
    ]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--endpoint", default="http://127.0.0.1:23819/mcp")
    parser.add_argument("--project-id", default="reddoc_true_dev")
    parser.add_argument("--iterations", type=int, default=7)
    parser.add_argument("--warmup", type=int, default=2)
    parser.add_argument("--timeout", type=float, default=180.0)
    parser.add_argument(
        "--scenario-timeout",
        type=float,
        help="Wall-clock timeout in seconds for each scenario.",
    )
    parser.add_argument(
        "--scenario",
        action="append",
        help="Run only the named scenario. Can be passed more than once.",
    )
    parser.add_argument(
        "--skip-project-stats",
        action="store_true",
        help="Do not call project_info stats before benchmark scenarios.",
    )
    parser.add_argument("--output-json", type=Path)
    args = parser.parse_args()

    client = HttpMcpClient(args.endpoint, args.timeout)
    client.initialize()

    context: dict[str, Any] = {"project_id": args.project_id}
    if args.skip_project_stats:
        context["project_stats_skipped"] = True
    else:
        try:
            stats = client.call_tool(
                "project_info",
                {"action": "stats", "project_id": args.project_id},
            )
            context["project_stats"] = {
                "status": stats.get("status"),
                "files": stats.get("files"),
                "chunks": stats.get("chunks"),
                "symbols": stats.get("symbols"),
                "serving": stats.get("serving"),
                "indexing_generation": stats.get("indexing_generation"),
            }
        except Exception as error:  # noqa: BLE001
            context["project_stats_error"] = str(error)

    scenarios = _scenarios(args.project_id)
    if args.scenario:
        wanted = set(args.scenario)
        scenarios = [scenario for scenario in scenarios if scenario.name in wanted]
        missing = wanted.difference({scenario.name for scenario in scenarios})
        if missing:
            raise SystemExit(f"unknown scenario(s): {', '.join(sorted(missing))}")

    report = {
        "generated_at": datetime.now(timezone.utc).isoformat(),
        "endpoint": args.endpoint,
        "context": context,
        "results": [],
    }

    if args.output_json:
        args.output_json.parent.mkdir(parents=True, exist_ok=True)
        args.output_json.write_text(json.dumps(report, indent=2, ensure_ascii=False) + "\n")

    for scenario in scenarios:
        started = time.perf_counter()
        old_handler = None
        if args.scenario_timeout is not None:
            old_handler = signal.getsignal(signal.SIGALRM)

            def _handle_timeout(signum: int, frame: Any) -> None:  # noqa: ARG001
                raise ScenarioTimeoutError(
                    f"scenario exceeded {args.scenario_timeout:.3f}s wall-clock timeout"
                )

            signal.signal(signal.SIGALRM, _handle_timeout)
            signal.setitimer(signal.ITIMER_REAL, args.scenario_timeout)
        try:
            result = _run_scenario(client, scenario, args.warmup, args.iterations)
        except Exception as error:  # noqa: BLE001
            result = {
                "name": scenario.name,
                "tool": scenario.tool,
                "arguments": scenario.arguments,
                "iterations": args.iterations,
                "warmup": args.warmup,
                "elapsed_ms": (time.perf_counter() - started) * 1000.0,
                "error": {
                    "type": type(error).__name__,
                    "message": str(error),
                },
            }
        finally:
            if args.scenario_timeout is not None:
                signal.setitimer(signal.ITIMER_REAL, 0)
                signal.signal(signal.SIGALRM, old_handler)
        report["results"].append(result)
        if args.output_json:
            args.output_json.write_text(json.dumps(report, indent=2, ensure_ascii=False) + "\n")

    print(json.dumps(report, indent=2, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
