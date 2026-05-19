# @steinx/memory-mcp-1file

MCP memory server with semantic search, code indexing, and knowledge graph for AI agents.

## Quick Start

Authenticate to GitHub Packages first, because this package is published to the GitHub npm registry rather than npmjs.org.

Choose one authentication method:
- add a personal access token (classic) to `~/.npmrc`; or
- run `npm login` against `npm.pkg.github.com`.

```bash
# Option 1: configure ~/.npmrc with your token
cat <<'EOF' >> ~/.npmrc
@steinx:registry=https://npm.pkg.github.com
//npm.pkg.github.com/:_authToken=YOUR_GITHUB_PAT_CLASSIC
EOF

# Option 2: interactive login
npm login --scope=@steinx --auth-type=legacy --registry=https://npm.pkg.github.com
```

```bash
# Run directly (downloads binary automatically)
npx @steinx/memory-mcp-1file

# Or with bun
bunx @steinx/memory-mcp-1file
```

If you republish this wrapper from a fork, the release workflow automatically derives the published npm scope and repository metadata from the current GitHub repository. For local testing and non-release installs, keep the checked-in `repository` field in `npm/package.json` accurate so the postinstall hook downloads release assets from your repository, or override it at install time with `MEMORY_MCP_RELEASE_REPO=owner/repo`.

## What is this?

`memory-mcp` is a [Model Context Protocol](https://modelcontextprotocol.io/) server that provides AI agents with:

- **Semantic memory** — store and search memories with embeddings
- **Code indexing** — parse and index codebases with tree-sitter
- **Knowledge graph** — entity extraction and relationship tracking
- **Temporal awareness** — time-based memory queries

## Configuration

Use with Claude Code, Cursor, or any MCP-compatible client:

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

## CLI Options

```bash
memory-mcp --help          # Show all options
memory-mcp --data-dir /data # Custom database path
```

## Supported Platforms

| Platform | Architecture |
|---|---|
| Linux | x86_64 (musl), ARM64 (musl) |
| macOS | x86_64, ARM64 (Apple Silicon) |
| Windows | x86_64 |

## Links

- [GitHub Repository](https://github.com/SteinX/memory-mcp-1file)
- [Releases](https://github.com/SteinX/memory-mcp-1file/releases)
- [Architecture](https://github.com/SteinX/memory-mcp-1file/blob/master/ARCHITECTURE.md)

## License

MIT
