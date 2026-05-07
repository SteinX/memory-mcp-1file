#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
IMAGE="${MCP_IMAGE:-memory-mcp:dev}"
VOLUME="${MCP_SMOKE_VOLUME:-memory-mcp-smoke-task7-$(date +%s)}"
KEEP_VOLUME="${MCP_SMOKE_KEEP_VOLUME:-0}"
FIXTURE="export-import-smoke@example.test memory migration fixture"
SOURCE_PROJECT="task7-smoke-project"
EVIDENCE_DIR="${ROOT_DIR}/.sisyphus/evidence"
HAPPY_LOG="${EVIDENCE_DIR}/task-7-mcp-smoke-happy.txt"
ERROR_LOG="${EVIDENCE_DIR}/task-7-mcp-smoke-error.txt"

mkdir -p "${EVIDENCE_DIR}"

cleanup() {
  if [[ "${KEEP_VOLUME}" != "1" ]]; then
    docker volume rm "${VOLUME}" >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT

if ! command -v docker >/dev/null 2>&1; then
  echo "docker is required for MCP smoke testing" >&2
  exit 1
fi

run_mcp_sequence() {
  local payload="$1"
  docker run --rm -i \
    --name "memory-mcp-smoke-$$" \
    --memory 4g \
    -v "${ROOT_DIR}/src:/project" \
    -v "${VOLUME}:/data" \
    -e RUST_LOG=warn \
    -e RUST_BACKTRACE=1 \
    "${IMAGE}" \
    /usr/local/bin/memory-mcp <<<"${payload}"
}

assert_happy_path() {
  python3 - "$HAPPY_LOG" "$FIXTURE" "$SOURCE_PROJECT" "$1" <<'PY'
import json
import sys

path, fixture, source_project, expected_old_id = sys.argv[1:5]

with open(path, "r", encoding="utf-8") as f:
    lines = [line.strip() for line in f if line.strip()]

responses = []
for line in lines:
    if not line.startswith("{"):
        continue
    try:
        responses.append(json.loads(line))
    except json.JSONDecodeError:
        pass

by_id = {item.get("id"): item for item in responses if isinstance(item, dict) and "id" in item}

def tool_body(resp_id):
    response = by_id.get(resp_id)
    if not response:
        raise AssertionError(f"missing JSON-RPC response id={resp_id}")
    result = response.get("result", {})
    content = result.get("content", [])
    if not content:
        raise AssertionError(f"missing content in response id={resp_id}")
    text = content[0].get("text")
    if text is None:
        raise AssertionError(f"missing text payload in response id={resp_id}")
    return json.loads(text)

store_body = tool_body(102)
export_body = tool_body(103)
import_body = tool_body(201)
search_body = tool_body(202)
get_body = tool_body(301)

stored_id = store_body.get("id")
if not stored_id:
    raise AssertionError("store_memory did not return id")

jsonl = export_body.get("jsonl", "")
if fixture not in jsonl:
    raise AssertionError("export_memory jsonl missing fixture content")
if export_body.get("exported_count", 0) < 1:
    raise AssertionError("export_memory exported_count should be >= 1")

for line in [line for line in jsonl.splitlines() if line.strip()]:
    record = json.loads(line)
    if fixture in json.dumps(record, ensure_ascii=False):
        if "embedding" in record or "vector" in record or "vectors" in record or "embeddings" in record:
            raise AssertionError("exported migration record contains vector/embedding fields")

id_mappings = import_body.get("id_mappings", [])
if import_body.get("imported_count", 0) < 1:
    raise AssertionError("import_memory imported_count should be >= 1")
if not id_mappings:
    raise AssertionError("import_memory remap expected non-empty id_mappings")

mapping = None
for candidate in id_mappings:
    if candidate.get("old_id") == expected_old_id:
        mapping = candidate
        break
if mapping is None:
    raise AssertionError("expected remap entry for exported fixture old_id")
if mapping.get("new_id") == stored_id:
    raise AssertionError("remapped new_id must differ from old_id")

if fixture not in json.dumps(search_body, ensure_ascii=False):
    raise AssertionError("search_memory response missing fixture content")

if get_body.get("memory", {}).get("content") != fixture:
    raise AssertionError("get_memory for remapped id did not return fixture content")
if get_body.get("memory", {}).get("namespace") != source_project:
    raise AssertionError("get_memory namespace mismatch for imported fixture")

print("happy_path_assertions=ok")
print(f"stored_id={stored_id}")
print(f"imported_id={mapping.get('new_id')}")
PY
}

assert_error_path() {
  python3 - "$ERROR_LOG" <<'PY'
import json
import sys

path = sys.argv[1]
with open(path, "r", encoding="utf-8") as f:
    lines = [line.strip() for line in f if line.strip()]

responses = []
for line in lines:
    if not line.startswith("{"):
        continue
    try:
        responses.append(json.loads(line))
    except json.JSONDecodeError:
        pass

by_id = {item.get("id"): item for item in responses if isinstance(item, dict) and "id" in item}
response = by_id.get(2)
if not response:
    raise AssertionError("missing malformed import response id=2")

payload = response.get("result", {}).get("content", [{}])[0].get("text")
if payload is None:
    raise AssertionError("missing malformed import payload text")
body = json.loads(payload)

if body.get("imported_count") != 0:
    raise AssertionError("malformed import should have imported_count=0")
if body.get("failed_count", 0) < 1:
    raise AssertionError("malformed import should have failed_count>=1")
errors = body.get("errors") or []
if not errors:
    raise AssertionError("malformed import should include structured errors")
if errors[0].get("code") != "invalid_jsonl":
    raise AssertionError("first malformed import error code should be invalid_jsonl")

print("error_path_assertions=ok")
PY
}

echo "[task-7] running happy-path MCP smoke with Docker volume ${VOLUME}"
HAPPY_SEQ_1=$(cat <<JSON
{"jsonrpc":"2.0","id":101,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"task7-smoke","version":"0.1"}}}
{"jsonrpc":"2.0","method":"notifications/initialized"}
{"jsonrpc":"2.0","id":102,"method":"tools/call","params":{"name":"store_memory","arguments":{"content":"${FIXTURE}","memory_type":"semantic","namespace":"${SOURCE_PROJECT}"}}}
{"jsonrpc":"2.0","id":103,"method":"tools/call","params":{"name":"export_memory","arguments":{"project_id":"${SOURCE_PROJECT}"}}}
JSON
)

HAPPY_OUTPUT_1=$(run_mcp_sequence "${HAPPY_SEQ_1}")
printf '%s\n' "${HAPPY_OUTPUT_1}" >"${HAPPY_LOG}"

EXPORTED_JSONL=$(python3 - "$HAPPY_LOG" <<'PY'
import json
import sys

path = sys.argv[1]
responses = []
for raw in open(path, "r", encoding="utf-8"):
    raw = raw.strip()
    if not raw.startswith("{"):
        continue
    try:
        responses.append(json.loads(raw))
    except json.JSONDecodeError:
        pass

for item in responses:
    if item.get("id") == 103:
        text = item.get("result", {}).get("content", [{}])[0].get("text")
        if text:
            print(json.loads(text).get("jsonl", ""))
            break
PY
)

if [[ -z "${EXPORTED_JSONL}" ]]; then
  echo "failed to extract exported jsonl from initial run" >&2
  exit 1
fi

SAFE_EXPORTED_JSONL=$(python3 - "$EXPORTED_JSONL" <<'PY'
import json
import sys
print(json.dumps(sys.argv[1]))
PY
)

EXPORTED_OLD_ID=$(python3 - <<'PY' "$EXPORTED_JSONL" "$FIXTURE"
import json
import sys

jsonl, fixture = sys.argv[1], sys.argv[2]
for line in jsonl.splitlines():
    line = line.strip()
    if not line:
        continue
    record = json.loads(line)
    if record.get("content") == fixture:
        print(record.get("id", ""))
        break
PY
)

if [[ -z "${EXPORTED_OLD_ID}" ]]; then
  echo "failed to extract exported old_id for fixture" >&2
  exit 1
fi

HAPPY_SEQ_2=$(cat <<JSON
{"jsonrpc":"2.0","id":200,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"task7-smoke","version":"0.1"}}}
{"jsonrpc":"2.0","method":"notifications/initialized"}
{"jsonrpc":"2.0","id":201,"method":"tools/call","params":{"name":"import_memory","arguments":{"project_id":"${SOURCE_PROJECT}","jsonl":${SAFE_EXPORTED_JSONL}}}}
{"jsonrpc":"2.0","id":202,"method":"tools/call","params":{"name":"search_memory","arguments":{"query":"export-import-smoke@example.test","mode":"bm25","namespace":"${SOURCE_PROJECT}","limit":10}}}
JSON
)

HAPPY_OUTPUT_2=$(run_mcp_sequence "${HAPPY_SEQ_2}")

IMPORTED_ID=$(python3 - "$HAPPY_OUTPUT_2" "$EXPORTED_OLD_ID" <<'PY'
import json
import sys

expected_old_id = sys.argv[2]
responses = []
for raw in sys.argv[1].splitlines():
    raw = raw.strip()
    if not raw.startswith("{"):
        continue
    try:
        responses.append(json.loads(raw))
    except json.JSONDecodeError:
        pass

for item in responses:
    if item.get("id") == 201:
        text = item.get("result", {}).get("content", [{}])[0].get("text")
        if not text:
            continue
        body = json.loads(text)
        mappings = body.get("id_mappings") or []
        for mapping in mappings:
            if mapping.get("old_id") == expected_old_id:
                print(mapping.get("new_id", ""))
                raise SystemExit(0)
PY
)

if [[ -z "${IMPORTED_ID}" ]]; then
  echo "failed to resolve remapped imported id" >&2
  exit 1
fi

HAPPY_SEQ_3=$(cat <<JSON
{"jsonrpc":"2.0","id":300,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"task7-smoke","version":"0.1"}}}
{"jsonrpc":"2.0","method":"notifications/initialized"}
{"jsonrpc":"2.0","id":301,"method":"tools/call","params":{"name":"get_memory","arguments":{"id":"${IMPORTED_ID}"}}}
JSON
)

HAPPY_OUTPUT_3=$(run_mcp_sequence "${HAPPY_SEQ_3}")
{
  printf '%s\n' "${HAPPY_OUTPUT_1}"
  printf '%s\n' "${HAPPY_OUTPUT_2}"
  printf '%s\n' "${HAPPY_OUTPUT_3}"
} >"${HAPPY_LOG}"

assert_happy_path "${EXPORTED_OLD_ID}"

echo "[task-7] running malformed-import MCP smoke"
ERROR_PAYLOAD=$(cat <<JSON
{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"task7-smoke","version":"0.1"}}}
{"jsonrpc":"2.0","method":"notifications/initialized"}
{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"import_memory","arguments":{"project_id":"${SOURCE_PROJECT}","jsonl":"{not-jsonl"}}}
JSON
)

ERROR_OUTPUT=$(run_mcp_sequence "${ERROR_PAYLOAD}")
printf '%s\n' "${ERROR_OUTPUT}" >"${ERROR_LOG}"
assert_error_path

echo "task-7 MCP smoke passed"
echo "happy log: ${HAPPY_LOG}"
echo "error log: ${ERROR_LOG}"
