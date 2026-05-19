# Client Configuration

This page keeps client wiring examples out of the homepage. Use Docker when you
want isolation and a mounted project root. Use `npx`/`bunx` when you want the
server to run directly on the local machine.

## Docker

The published image defaults to HTTP/SSE on port `8080`. MCP desktop and CLI
clients usually speak stdio, so override the image command with
`memory-mcp --data-dir /data --stdio` for those clients.

Use this image name for the upstream repository:

```bash
ghcr.io/steinx/memory-mcp-1file:latest
```

Key mounts:

| Mount | Purpose |
|---|---|
| `-v mcp-data:/data` | Persists SurrealDB data and Docker model cache under `/data/models`. |
| `-v /host/project:/project:ro` | Makes the project visible to the server for code indexing. |

### HTTP Mode

HTTP mode is useful for a standalone local server, a remote agent, or a
containerized backend.

```bash
docker run -d \
  --name memory-mcp \
  --memory=3g \
  -p 8080:8080 \
  -v mcp-data:/data \
  -v /absolute/path/to/host/project:/project:ro \
  -e PROJECT_PATH=/project \
  ghcr.io/steinx/memory-mcp-1file:latest
```

Important path rules:

- The server can only index paths visible to the server process.
- Docker users must mount the project and refer to the mounted path, usually
  `/project`.
- HTTP clients cannot make the server read arbitrary client-local paths unless
  those paths are mounted into the server environment.
- Project binding is handled through explicit `project_info` actions, not HTTP
  headers or query parameters.

`GET /health` is a liveness probe for the HTTP process. It returns `200 OK`
when the server is accepting requests and does not block on database or
embedding readiness. Use the MCP `get_status` tool for deeper readiness data.

### Stdio Mode

Use stdio for Claude Desktop, Claude Code, Cursor, OpenCode, Gemini CLI, Cline,
Roo Code, and other MCP clients that launch the server as a subprocess.

```bash
docker run --init -i --rm --memory=4g \
  -v mcp-data:/data \
  -v "$(pwd):/project:ro" \
  ghcr.io/steinx/memory-mcp-1file:latest \
  memory-mcp --data-dir /data --stdio
```

For JSON-based MCP config:

```json
{
  "mcpServers": {
    "memory": {
      "command": "docker",
      "args": [
        "run",
        "--init",
        "-i",
        "--rm",
        "--memory=4g",
        "-v",
        "mcp-data:/data",
        "-v",
        "/absolute/path/to/your/project:/project:ro",
        "ghcr.io/steinx/memory-mcp-1file:latest",
        "memory-mcp",
        "--data-dir",
        "/data",
        "--stdio"
      ]
    }
  }
}
```

Prefer absolute host paths in desktop client config. Some clients support
variables such as `${workspaceFolder}`, but absolute paths are more predictable
for Docker.

## Cursor

Docker command:

```bash
docker run --init -i --rm --memory=4g -v mcp-data:/data -v "/Users/yourname/projects/current:/project:ro" ghcr.io/steinx/memory-mcp-1file:latest memory-mcp --data-dir /data --stdio
```

`npx` command:

```bash
npx -y @steinx/memory-mcp-1file
```

Or `.cursor/mcp.json`:

```json
{
  "mcpServers": {
    "memory": {
      "command": "npx",
      "args": ["-y", "@steinx/memory-mcp-1file"]
    }
  }
}
```

## Claude Desktop

Docker:

```json
{
  "mcpServers": {
    "memory": {
      "command": "docker",
      "args": [
        "run",
        "--init",
        "-i",
        "--rm",
        "--memory=4g",
        "-v",
        "mcp-data:/data",
        "-v",
        "/absolute/path/to/your/project:/project:ro",
        "ghcr.io/steinx/memory-mcp-1file:latest",
        "memory-mcp",
        "--data-dir",
        "/data",
        "--stdio"
      ]
    }
  }
}
```

`npx`:

```json
{
  "mcpServers": {
    "memory": {
      "command": "npx",
      "args": ["-y", "@steinx/memory-mcp-1file"]
    }
  }
}
```

## Claude Code

```bash
claude mcp add memory -- npx -y @steinx/memory-mcp-1file
```

## OpenCode / CLI

```bash
docker run --init -i --rm --memory=4g \
  -v mcp-data:/data \
  -v "$(pwd):/project:ro" \
  ghcr.io/steinx/memory-mcp-1file:latest \
  memory-mcp --data-dir /data --stdio
```

## Windsurf / VS Code

```json
{
  "mcpServers": {
    "memory": {
      "command": "npx",
      "args": ["-y", "@steinx/memory-mcp-1file"]
    }
  }
}
```

## Gemini CLI

```json
{
  "mcpServers": {
    "memory": {
      "command": "npx",
      "args": ["-y", "@steinx/memory-mcp-1file"]
    }
  }
}
```

Docker variant:

```json
{
  "mcpServers": {
    "memory": {
      "command": "docker",
      "args": [
        "run",
        "--init",
        "-i",
        "--rm",
        "--memory=4g",
        "-v",
        "mcp-data:/data",
        "-v",
        "${workspaceFolder}:/project:ro",
        "ghcr.io/steinx/memory-mcp-1file:latest",
        "memory-mcp",
        "--data-dir",
        "/data",
        "--stdio"
      ]
    }
  }
}
```

## NPX / Bunx

The GitHub Packages npm wrapper downloads the right prebuilt binary for the
current platform.

Because the package is published to GitHub Packages, configure the `@steinx`
scope first:

```bash
# Option 1: configure ~/.npmrc with a GitHub PAT classic
cat <<'EOF' >> ~/.npmrc
@steinx:registry=https://npm.pkg.github.com
//npm.pkg.github.com/:_authToken=YOUR_GITHUB_PAT_CLASSIC
EOF

# Option 2: interactive login
npm login --scope=@steinx --auth-type=legacy --registry=https://npm.pkg.github.com
```

Run through `npx`:

```bash
npx -y @steinx/memory-mcp-1file
```

Run through `bunx`:

```json
{
  "mcpServers": {
    "memory": {
      "command": "bunx",
      "args": ["@steinx/memory-mcp-1file"]
    }
  }
}
```

Unlike Docker, `npx`/`bunx` runs locally and can already see the local
filesystem. To customize storage:

```json
{
  "mcpServers": {
    "memory": {
      "command": "npx",
      "args": [
        "-y",
        "@steinx/memory-mcp-1file",
        "--",
        "--data-dir",
        "/path/to/data"
      ]
    }
  }
}
```

Downloaded HuggingFace model files are shared across local `--data-dir` values
by default through the platform app-data location. If the selected model already
exists under the legacy `${data_dir}/models` layout, that cache is reused. Use
`--model-cache-dir` only when you need a custom shared model location.
