from __future__ import annotations

import contextlib
import json
import os
import queue
import shlex
import subprocess
import threading
import time
from pathlib import Path
from typing import Any, Mapping, Sequence


ROOT = Path(__file__).resolve().parents[2]


def _coerce_command(command: str | Sequence[str]) -> list[str]:
    if isinstance(command, str):
        return shlex.split(command)
    return [str(part) for part in command]


def resolve_mcp_command(command: str | Sequence[str] | None = None, *, root: Path | None = None) -> list[str]:
    """Resolve the server command without starting the MCP server.

    Preference order:
    1. target/fast/memory-mcp
    2. target/release/memory-mcp
    3. cargo run --quiet -- --stdio
    """

    if command is not None:
        return _coerce_command(command)

    root = root or ROOT
    fast_bin = root / "target" / "fast" / "memory-mcp"
    release_bin = root / "target" / "release" / "memory-mcp"
    if fast_bin.exists():
        return [str(fast_bin), "--stdio"]
    if release_bin.exists():
        return [str(release_bin), "--stdio"]
    return ["cargo", "run", "--quiet", "--", "--stdio"]


def build_env(overrides: Mapping[str, str] | None = None) -> dict[str, str]:
    """Build a child process environment without mutating global state."""

    env = os.environ.copy()
    default_data_dir = env.get("DATA_DIR") or str(ROOT / "memory-mcp-data")
    defaults = {
        "DATA_DIR": default_data_dir,
        "EMBEDDING_MODEL": env.get("EMBEDDING_MODEL", "e5_small"),
        "RUST_LOG": env.get("RUST_LOG", "warn"),
        "RUST_BACKTRACE": env.get("RUST_BACKTRACE", "1"),
    }
    env.update(defaults)
    if overrides:
        env.update({key: str(value) for key, value in overrides.items()})
    return env


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

    @classmethod
    def start(
        cls,
        command: str | Sequence[str] | None = None,
        *,
        root: Path | None = None,
        env_overrides: Mapping[str, str] | None = None,
        timeout: float = 30.0,
        initialize: bool = True,
        client_name: str = "evals-mcp-client",
        client_version: str = "0.1.0",
    ) -> "McpClient":
        resolved = resolve_mcp_command(command, root=root)
        env = build_env(env_overrides)
        proc = subprocess.Popen(  # noqa: S603
            resolved,
            cwd=root or ROOT,
            env=env,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            bufsize=1,
        )
        client = cls(proc)
        try:
            if initialize:
                client.initialize(timeout=timeout, client_name=client_name, client_version=client_version)
                client.notify_initialized()
            return client
        except Exception:
            client.close()
            raise

    def __enter__(self) -> "McpClient":
        return self

    def __exit__(self, exc_type, exc, tb) -> None:
        self.close()

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
                    self.close()
                    raise RuntimeError(
                        f"MCP process exited early (code={self.proc.returncode}); stderr_tail={self.stderr_tail()}"
                    )
                continue

            if response.get("id") != request_id:
                continue

            if "error" in response:
                raise RuntimeError(f"JSON-RPC error in {method}: {response['error']}")
            return response

        self.close()
        raise TimeoutError(f"Timeout waiting for {method} ({timeout}s); stderr_tail={self.stderr_tail()}")

    def notify(self, method: str, params: dict[str, Any] | None = None) -> None:
        payload: dict[str, Any] = {"jsonrpc": "2.0", "method": method}
        if params is not None:
            payload["params"] = params
        assert self.proc.stdin is not None
        self.proc.stdin.write(json.dumps(payload) + "\n")
        self.proc.stdin.flush()

    def initialize(
        self,
        *,
        timeout: float = 30.0,
        client_name: str = "evals-mcp-client",
        client_version: str = "0.1.0",
    ) -> dict[str, Any]:
        return self.request(
            "initialize",
            {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": client_name, "version": client_version},
            },
            timeout=timeout,
        )

    def notify_initialized(self) -> None:
        self.notify("notifications/initialized")

    def call_tool(self, name: str, arguments: dict[str, Any] | None = None, timeout: float = 30.0) -> dict[str, Any]:
        return self.request("tools/call", {"name": name, "arguments": arguments or {}}, timeout=timeout)

    def stderr_tail(self, n: int = 40) -> list[str]:
        return self._stderr[-n:]

    def close(self) -> None:
        proc = self.proc
        if proc.poll() is None:
            with contextlib.suppress(BrokenPipeError, OSError):
                if proc.stdin is not None:
                    proc.stdin.close()
            proc.terminate()
            try:
                proc.wait(timeout=5)
            except subprocess.TimeoutExpired:
                proc.kill()
                with contextlib.suppress(subprocess.TimeoutExpired):
                    proc.wait(timeout=5)

        with contextlib.suppress(Exception):
            if proc.stdout is not None:
                proc.stdout.close()
        with contextlib.suppress(Exception):
            if proc.stderr is not None:
                proc.stderr.close()
