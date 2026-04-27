#!/usr/bin/env python3
"""Task 14 MCP regression/benchmark harness.

Bounded stdio JSON-RPC harness that verifies MCP lifecycle and representative
public tool calls without changing server behavior or docs.

Outputs:
  - .sisyphus/evidence/task-14-harness-run.json
  - .sisyphus/evidence/task-14-harness-summary.md
  - .sisyphus/evidence/task-14-tool-name-validation.md
"""

from __future__ import annotations

import argparse
import json
import os
import queue
import shlex
import subprocess
import sys
import tempfile
import threading
import time
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
EVIDENCE_DIR = ROOT / ".sisyphus" / "evidence"
TASK14_RUN_JSON = EVIDENCE_DIR / "task-14-harness-run.json"
TASK14_SUMMARY_MD = EVIDENCE_DIR / "task-14-harness-summary.md"
TASK14_TOOL_VALIDATION_MD = EVIDENCE_DIR / "task-14-tool-name-validation.md"

TASK1_TOOL_LIST = EVIDENCE_DIR / "task-1-tool-list.json"
TASK3_BASELINE = EVIDENCE_DIR / "task-3-tool-selection-baseline.json"

SCENARIO_SET_VERSION = "task3-ts-001-012"
FORBIDDEN_TOOL_NAMES = {"search", "search_code"}
SCRIPTED_PUBLIC_TOOL_NAMES = {"project_info", "recall_code"}


@dataclass
class StageResult:
    name: str
    ok: bool
    duration_ms: float
    detail: dict[str, Any]


class McpClient:
    def __init__(self, proc: subprocess.Popen[str]):
        self.proc = proc
        self._next_id = 1
        self._responses: queue.Queue[dict[str, Any]] = queue.Queue()
        self._stderr: list[str] = []
        self._stdout_thread = threading.Thread(target=self._read_stdout, daemon=True)
        self._stderr_thread = threading.Thread(target=self._read_stderr, daemon=True)
        self._stdout_thread.start()
        self._stderr_thread.start()

    def _read_stdout(self) -> None:
        assert self.proc.stdout is not None
        for line in self.proc.stdout:
            line = line.strip()
            if not line:
                continue
            try:
                self._responses.put(json.loads(line))
            except json.JSONDecodeError:
                self._responses.put({"non_json_stdout": line})

    def _read_stderr(self) -> None:
        assert self.proc.stderr is not None
        for line in self.proc.stderr:
            self._stderr.append(line.rstrip())

    def request(self, method: str, params: dict[str, Any] | None = None, timeout: float = 30.0) -> dict[str, Any]:
        request_id = self._next_id
        self._next_id += 1
        payload: dict[str, Any] = {"jsonrpc": "2.0", "id": request_id, "method": method}
        if params is not None:
            payload["params"] = params

        assert self.proc.stdin is not None
        self.proc.stdin.write(json.dumps(payload) + "\n")
        self.proc.stdin.flush()

        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            try:
                response = self._responses.get(timeout=0.25)
            except queue.Empty:
                if self.proc.poll() is not None:
                    raise RuntimeError(
                        f"MCP process exited early (code={self.proc.returncode}); stderr_tail={self.stderr_tail()}"
                    )
                continue

            if response.get("id") != request_id:
                continue

            if "error" in response:
                raise RuntimeError(f"JSON-RPC error in {method}: {response['error']}")
            return response

        raise TimeoutError(f"Timeout waiting for {method} ({timeout}s); stderr_tail={self.stderr_tail()}")

    def notify(self, method: str, params: dict[str, Any] | None = None) -> None:
        payload: dict[str, Any] = {"jsonrpc": "2.0", "method": method}
        if params is not None:
            payload["params"] = params
        assert self.proc.stdin is not None
        self.proc.stdin.write(json.dumps(payload) + "\n")
        self.proc.stdin.flush()

    def stderr_tail(self, n: int = 40) -> list[str]:
        return self._stderr[-n:]

    def close(self) -> None:
        if self.proc.poll() is None:
            self.proc.terminate()
            try:
                self.proc.wait(timeout=8)
            except subprocess.TimeoutExpired:
                self.proc.kill()


def now_iso() -> str:
    return datetime.now(timezone.utc).isoformat()


def choose_command(explicit: str | None) -> list[str]:
    if explicit:
        return shlex.split(explicit)

    release_bin = ROOT / "target" / "release" / "memory-mcp"
    debug_bin = ROOT / "target" / "debug" / "memory-mcp"
    if release_bin.exists():
        return [str(release_bin), "--stdio"]
    if debug_bin.exists():
        return [str(debug_bin), "--stdio"]
    return ["cargo", "run", "--quiet", "--", "--stdio"]


def parse_tool_text_result(response: dict[str, Any]) -> dict[str, Any]:
    result = response.get("result", {})
    content = result.get("content", [])
    text: str | None = None
    if content and isinstance(content, list):
        first = content[0] if content else {}
        text = first.get("text") if isinstance(first, dict) else None

    parsed: dict[str, Any] | None = None
    parse_error: str | None = None
    if text is not None:
        try:
            maybe = json.loads(text)
            if isinstance(maybe, dict):
                parsed = maybe
            else:
                parsed = {"_non_object_payload": maybe}
        except Exception as exc:  # noqa: BLE001
            parse_error = str(exc)

    return {
        "raw_jsonrpc_response": response,
        "text": text,
        "parsed": parsed,
        "parse_error": parse_error,
    }


def stage_call(name: str, fn: Any) -> StageResult:
    start = time.perf_counter()
    try:
        detail = fn()
        return StageResult(name=name, ok=True, duration_ms=round((time.perf_counter() - start) * 1000, 2), detail=detail)
    except Exception as exc:  # noqa: BLE001
        return StageResult(
            name=name,
            ok=False,
            duration_ms=round((time.perf_counter() - start) * 1000, 2),
            detail={"error": str(exc), "stage": name},
        )


def load_task1_tool_names() -> list[str]:
    if not TASK1_TOOL_LIST.exists():
        return []
    doc = json.loads(TASK1_TOOL_LIST.read_text(encoding="utf-8"))
    tools = doc.get("result", {}).get("tools", [])
    names: list[str] = [tool["name"] for tool in tools if isinstance(tool, dict) and isinstance(tool.get("name"), str)]
    return sorted(set(names))


def load_task3_scenarios() -> list[dict[str, Any]]:
    if not TASK3_BASELINE.exists():
        return []
    doc = json.loads(TASK3_BASELINE.read_text(encoding="utf-8"))
    scenarios = doc.get("scenarios", [])
    return scenarios if isinstance(scenarios, list) else []


def extract_project_id(project_info_list_payload: dict[str, Any]) -> str | None:
    parsed = project_info_list_payload.get("parsed")
    if not isinstance(parsed, dict):
        return None

    projects = parsed.get("projects")
    if isinstance(projects, list) and projects:
        first = projects[0]
        if isinstance(first, dict):
            for key in ("project_id", "id", "projectId"):
                value = first.get(key)
                if isinstance(value, str) and value:
                    return value
    return None


def main() -> int:
    parser = argparse.ArgumentParser(description="Task 14 MCP regression harness")
    parser.add_argument("--command", help="Explicit server command, e.g. 'target/release/memory-mcp --stdio'")
    parser.add_argument("--timeout", type=float, default=30.0, help="Per-stage timeout in seconds")
    args = parser.parse_args()

    EVIDENCE_DIR.mkdir(parents=True, exist_ok=True)

    command = choose_command(args.command)
    env = os.environ.copy()
    env["DATA_DIR"] = tempfile.mkdtemp(prefix="task-14-harness-data-")
    env["LOG_LEVEL"] = env.get("LOG_LEVEL", "warn")
    env["EMBEDDING_MODEL"] = env.get("EMBEDDING_MODEL", "e5_small")

    run_started = time.time()
    manifest: dict[str, Any] = {
        "timestamp_utc": now_iso(),
        "startup_mode": "local_stdio_jsonrpc",
        "command": command,
        "cwd": str(ROOT),
        "data_dir_strategy": "isolated temporary DATA_DIR per run",
        "data_dir": env["DATA_DIR"],
        "model_env": env.get("EMBEDDING_MODEL"),
        "scenario_set_version": SCENARIO_SET_VERSION,
        "source_artifacts": {
            "task1_tool_list": str(TASK1_TOOL_LIST.relative_to(ROOT)) if TASK1_TOOL_LIST.exists() else None,
            "task3_baseline": str(TASK3_BASELINE.relative_to(ROOT)) if TASK3_BASELINE.exists() else None,
        },
    }

    proc = subprocess.Popen(  # noqa: S603
        command,
        cwd=ROOT,
        env=env,
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        bufsize=1,
    )
    client = McpClient(proc)

    stages: list[StageResult] = []
    run_payload: dict[str, Any] = {"manifest": manifest}
    exit_code = 0

    try:
        init = stage_call(
            "initialize",
            lambda: client.request(
                "initialize",
                {
                    "protocolVersion": "2024-11-05",
                    "capabilities": {},
                    "clientInfo": {"name": "task-14-mcp-regression-harness", "version": "0.1.0"},
                },
                timeout=args.timeout,
            ),
        )
        stages.append(init)

        initialized = stage_call("notifications/initialized", lambda: {"notified": client.notify("notifications/initialized") is None})
        stages.append(initialized)

        tools_list = stage_call("tools/list", lambda: client.request("tools/list", {}, timeout=args.timeout))
        stages.append(tools_list)

        project_info_list = stage_call(
            "tools/call project_info(list)",
            lambda: parse_tool_text_result(
                client.request(
                    "tools/call",
                    {"name": "project_info", "arguments": {"action": "list"}},
                    timeout=args.timeout,
                )
            ),
        )
        stages.append(project_info_list)

        project_id = extract_project_id(project_info_list.detail) if project_info_list.ok else None
        run_payload["selected_project_id"] = project_id

        if project_id:
            project_info_status = stage_call(
                "tools/call project_info(status)",
                lambda: parse_tool_text_result(
                    client.request(
                        "tools/call",
                        {"name": "project_info", "arguments": {"action": "status", "project_id": project_id}},
                        timeout=args.timeout,
                    )
                ),
            )
            stages.append(project_info_status)

        recall_args: dict[str, Any] = {
            "query": "find the RRF merge function",
            "mode": "hybrid",
            "limit": 5,
        }
        if project_id:
            recall_args["projectId"] = project_id

        recall_code = stage_call(
            "tools/call recall_code",
            lambda: parse_tool_text_result(
                client.request(
                    "tools/call",
                    {"name": "recall_code", "arguments": recall_args},
                    timeout=args.timeout,
                )
            ),
        )
        stages.append(recall_code)

        tools_list_names: list[str] = []
        tools_list_required_check: dict[str, Any] = {}
        if tools_list.ok:
            names = []
            for tool in tools_list.detail.get("result", {}).get("tools", []):
                if isinstance(tool, dict) and isinstance(tool.get("name"), str):
                    names.append(tool["name"])
            tools_list_names = sorted(set(names))

        baseline_names = load_task1_tool_names()
        baseline_set = set(baseline_names)
        runtime_set = set(tools_list_names)

        tools_list_required_check = {
            "baseline_tool_count": len(baseline_names),
            "runtime_tool_count": len(tools_list_names),
            "missing_from_runtime": sorted(baseline_set - runtime_set),
            "added_in_runtime": sorted(runtime_set - baseline_set),
            "required_scripted_tools_present": sorted(name for name in SCRIPTED_PUBLIC_TOOL_NAMES if name in runtime_set),
            "required_scripted_tools_missing": sorted(SCRIPTED_PUBLIC_TOOL_NAMES - runtime_set),
        }

        scenarios = load_task3_scenarios()
        scenario_checks: list[dict[str, Any]] = []
        for scenario in scenarios:
            scenario_id = scenario.get("id")
            expected_no_tool = bool(scenario.get("expected_no_mcp_tool"))
            expected_tools = scenario.get("expected_first_tools") or []
            if expected_no_tool:
                status = "compatible_no_tool_scenario"
                matched = []
            else:
                expected_tools = [t for t in expected_tools if isinstance(t, str)]
                matched = sorted([t for t in expected_tools if t in runtime_set])
                status = "compatible" if matched else "missing_expected_tools"
            scenario_checks.append(
                {
                    "id": scenario_id,
                    "expected_no_mcp_tool": expected_no_tool,
                    "expected_first_tools": expected_tools,
                    "matched_runtime_tools": matched,
                    "status": status,
                }
            )

        validation = {
            "forbidden_public_tool_names": sorted(FORBIDDEN_TOOL_NAMES),
            "scripted_public_tool_names": sorted(SCRIPTED_PUBLIC_TOOL_NAMES),
            "forbidden_names_used_by_harness": sorted(SCRIPTED_PUBLIC_TOOL_NAMES & FORBIDDEN_TOOL_NAMES),
            "forbidden_names_present_in_runtime_tools": sorted(runtime_set & FORBIDDEN_TOOL_NAMES),
            "tools_list_required_check": tools_list_required_check,
            "scenario_compatibility": {
                "scenario_set_version": SCENARIO_SET_VERSION,
                "total": len(scenario_checks),
                "compatible_count": sum(1 for row in scenario_checks if row["status"] != "missing_expected_tools"),
                "missing_expected_count": sum(1 for row in scenario_checks if row["status"] == "missing_expected_tools"),
                "checks": scenario_checks,
            },
        }

        critical_stages = {"initialize", "tools/list", "tools/call project_info(list)", "tools/call recall_code"}
        failed_critical = [s.name for s in stages if (s.name in critical_stages and not s.ok)]

        run_payload.update(
            {
                "stages": [
                    {
                        "name": s.name,
                        "ok": s.ok,
                        "duration_ms": s.duration_ms,
                        "detail": s.detail,
                    }
                    for s in stages
                ],
                "runtime_tools": tools_list_names,
                "validation": validation,
                "critical_failures": failed_critical,
                "server_stderr_tail": client.stderr_tail(),
                "duration_seconds": round(time.time() - run_started, 2),
                "harness_status": "ok" if not failed_critical else "failed",
            }
        )

        TASK14_RUN_JSON.write_text(json.dumps(run_payload, indent=2, sort_keys=True), encoding="utf-8")
        TASK14_SUMMARY_MD.write_text(build_summary_md(run_payload), encoding="utf-8")
        TASK14_TOOL_VALIDATION_MD.write_text(build_tool_validation_md(run_payload), encoding="utf-8")

        if failed_critical:
            exit_code = 2
    except Exception as exc:  # noqa: BLE001
        failure_payload = {
            "manifest": manifest,
            "harness_status": "failed",
            "failure_stage": "global",
            "error": str(exc),
            "stages": [
                {
                    "name": s.name,
                    "ok": s.ok,
                    "duration_ms": s.duration_ms,
                    "detail": s.detail,
                }
                for s in stages
            ],
            "server_stderr_tail": client.stderr_tail(),
            "partial_output_paths": {
                "run_json": str(TASK14_RUN_JSON.relative_to(ROOT)),
                "summary_md": str(TASK14_SUMMARY_MD.relative_to(ROOT)),
                "tool_validation_md": str(TASK14_TOOL_VALIDATION_MD.relative_to(ROOT)),
            },
        }
        TASK14_RUN_JSON.write_text(json.dumps(failure_payload, indent=2, sort_keys=True), encoding="utf-8")
        TASK14_SUMMARY_MD.write_text(
            "# Task 14 harness summary\n\n"
            f"- Status: failed\n"
            f"- Failure stage: global\n"
            f"- Error: `{exc}`\n"
            f"- Partial outputs: `{TASK14_RUN_JSON.relative_to(ROOT)}`\n",
            encoding="utf-8",
        )
        TASK14_TOOL_VALIDATION_MD.write_text(
            "# Task 14 tool name validation\n\n"
            "Harness failed before full validation. See run JSON for diagnostics.\n",
            encoding="utf-8",
        )
        exit_code = 3
    finally:
        client.close()

    return exit_code


def build_summary_md(run_payload: dict[str, Any]) -> str:
    manifest = run_payload.get("manifest", {})
    stages = run_payload.get("stages", [])
    validation = run_payload.get("validation", {})
    tools_check = validation.get("tools_list_required_check", {})
    scenario = validation.get("scenario_compatibility", {})
    lines = [
        "# Task 14 harness summary",
        "",
        f"- Timestamp (UTC): `{manifest.get('timestamp_utc')}`",
        f"- Status: `{run_payload.get('harness_status')}`",
        f"- Command: `{manifest.get('command')}`",
        f"- Startup mode: `{manifest.get('startup_mode')}`",
        f"- Data dir strategy: {manifest.get('data_dir_strategy')}",
        f"- Scenario set: `{manifest.get('scenario_set_version')}`",
        f"- Duration: `{run_payload.get('duration_seconds')}` seconds",
        "",
        "## Stage results",
        "",
        "| Stage | OK | Duration ms |",
        "|---|---:|---:|",
    ]
    for s in stages:
        lines.append(f"| `{s.get('name')}` | `{s.get('ok')}` | {s.get('duration_ms')} |")

    lines.extend(
        [
            "",
            "## Tool compatibility snapshot",
            "",
            f"- Baseline tool count (Task 1): `{tools_check.get('baseline_tool_count')}`",
            f"- Runtime tool count: `{tools_check.get('runtime_tool_count')}`",
            f"- Missing baseline tools in runtime: `{tools_check.get('missing_from_runtime')}`",
            f"- Added runtime tools vs baseline: `{tools_check.get('added_in_runtime')}`",
            f"- Scripted required tools missing: `{tools_check.get('required_scripted_tools_missing')}`",
            "",
            "## Scenario compatibility (Task 3 IDs)",
            "",
            f"- Total scenarios checked: `{scenario.get('total')}`",
            f"- Compatible scenarios: `{scenario.get('compatible_count')}`",
            f"- Missing-expected scenarios: `{scenario.get('missing_expected_count')}`",
            "",
            "## Evidence files",
            "",
            f"- `{TASK14_RUN_JSON.relative_to(ROOT)}`",
            f"- `{TASK14_SUMMARY_MD.relative_to(ROOT)}`",
            f"- `{TASK14_TOOL_VALIDATION_MD.relative_to(ROOT)}`",
            "",
            "## Diagnostics",
            "",
            f"- Critical failures: `{run_payload.get('critical_failures')}`",
            "- See run JSON for full JSON-RPC payloads and stage-level errors/timeouts.",
        ]
    )
    return "\n".join(lines) + "\n"


def build_tool_validation_md(run_payload: dict[str, Any]) -> str:
    validation = run_payload.get("validation", {})
    tools_check = validation.get("tools_list_required_check", {})
    scenario = validation.get("scenario_compatibility", {})

    lines = [
        "# Task 14 tool name validation",
        "",
        "Validation target: ensure harness uses current public MCP tool names and avoids stale names (`search`, `search_code`).",
        "",
        "## Scripted tool names",
        "",
        f"- Scripted public tools: `{validation.get('scripted_public_tool_names')}`",
        f"- Forbidden names: `{validation.get('forbidden_public_tool_names')}`",
        f"- Forbidden names used by harness: `{validation.get('forbidden_names_used_by_harness')}`",
        "",
        "## Runtime tools/list comparison against Task 1 baseline",
        "",
        f"- Baseline tool count: `{tools_check.get('baseline_tool_count')}`",
        f"- Runtime tool count: `{tools_check.get('runtime_tool_count')}`",
        f"- Missing from runtime: `{tools_check.get('missing_from_runtime')}`",
        f"- Added in runtime: `{tools_check.get('added_in_runtime')}`",
        f"- Required scripted tools present: `{tools_check.get('required_scripted_tools_present')}`",
        f"- Required scripted tools missing: `{tools_check.get('required_scripted_tools_missing')}`",
        f"- Forbidden names present in runtime tools/list: `{validation.get('forbidden_names_present_in_runtime_tools')}`",
        "",
        "## Scenario/compatibility feasibility checks (Task 3 IDs)",
        "",
        f"- Scenario set version: `{scenario.get('scenario_set_version')}`",
        f"- Compatible count: `{scenario.get('compatible_count')}` / `{scenario.get('total')}`",
        "",
        "| Scenario ID | Expected first tools | Matched runtime tools | Status |",
        "|---|---|---|---|",
    ]

    for check in scenario.get("checks", []):
        lines.append(
            "| {sid} | `{expected}` | `{matched}` | `{status}` |".format(
                sid=check.get("id"),
                expected=check.get("expected_first_tools"),
                matched=check.get("matched_runtime_tools"),
                status=check.get("status"),
            )
        )

    lines.extend(
        [
            "",
            "## Verdict",
            "",
            "- PASS when: no forbidden names are scripted, and required scripted tools are present in runtime tools/list.",
            "- This harness intentionally calls `recall_code` (public) and never calls `search_code` (internal-only).",
        ]
    )
    return "\n".join(lines) + "\n"


def _build_minimal_self_test_payload() -> dict[str, Any]:
    return {
        "harness_status": "ok",
        "runtime_tools": [
            "recall_code",
            "search_symbols",
            "symbol_graph",
            "project_info",
            "recall",
            "search_memory",
            "how_to_use",
        ],
        "validation": {
            "forbidden_public_tool_names": sorted(FORBIDDEN_TOOL_NAMES),
            "scripted_public_tool_names": sorted(SCRIPTED_PUBLIC_TOOL_NAMES),
            "forbidden_names_used_by_harness": [],
            "forbidden_names_present_in_runtime_tools": [],
            "tools_list_required_check": {
                "baseline_tool_count": 21,
                "runtime_tool_count": 21,
                "missing_from_runtime": [],
                "added_in_runtime": [],
                "required_scripted_tools_present": sorted(SCRIPTED_PUBLIC_TOOL_NAMES),
                "required_scripted_tools_missing": [],
            },
            "scenario_compatibility": {
                "scenario_set_version": SCENARIO_SET_VERSION,
                "total": 0,
                "compatible_count": 0,
                "missing_expected_count": 0,
                "checks": [],
            },
        },
        "critical_failures": [],
        "manifest": {
            "timestamp_utc": now_iso(),
            "command": ["memory-mcp", "--stdio"],
            "startup_mode": "local_stdio_jsonrpc",
            "data_dir_strategy": "isolated temporary DATA_DIR per run",
            "scenario_set_version": SCENARIO_SET_VERSION,
        },
        "duration_seconds": 0.01,
        "stages": [
            {"name": "initialize", "ok": True, "duration_ms": 1.0, "detail": {}},
            {"name": "tools/list", "ok": True, "duration_ms": 1.0, "detail": {}},
            {
                "name": "tools/call project_info(list)",
                "ok": True,
                "duration_ms": 1.0,
                "detail": {},
            },
            {"name": "tools/call recall_code", "ok": True, "duration_ms": 1.0, "detail": {}},
        ],
    }


def run_self_tests() -> None:
    payload = _build_minimal_self_test_payload()

    assert "harness_status" in payload
    assert "runtime_tools" in payload
    assert "validation" in payload
    assert payload["validation"]["tools_list_required_check"]["required_scripted_tools_missing"] == []

    validation = payload["validation"]
    assert "forbidden_public_tool_names" in validation
    assert "forbidden_names_present_in_runtime_tools" in validation
    assert "tools_list_required_check" in validation

    runtime_set = set(payload["runtime_tools"])
    forbidden_set = set(validation["forbidden_public_tool_names"])
    assert "search" not in runtime_set
    assert "search_code" not in runtime_set
    assert runtime_set.isdisjoint(forbidden_set)

    tools_list_required = validation["tools_list_required_check"]
    assert tools_list_required["required_scripted_tools_missing"] == []
    assert payload["validation"]["scenario_compatibility"]["scenario_set_version"] == SCENARIO_SET_VERSION

    summary_md = build_summary_md(payload)
    tool_validation_md = build_tool_validation_md(payload)

    assert "Status" in summary_md
    assert "Critical failures" in summary_md
    assert "Scenario compatibility" in summary_md
    assert "Scripted tool names" in tool_validation_md
    assert "Forbidden names" in tool_validation_md


if __name__ == "__main__":
    if len(sys.argv) > 1 and sys.argv[1] == "--self-test":
        run_self_tests()
        sys.exit(0)
    sys.exit(main())
