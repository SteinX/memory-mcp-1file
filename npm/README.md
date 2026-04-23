# memory-mcp-1file

MCP memory server with semantic search, code indexing, and knowledge graph for AI agents.

## Quick Start

```bash
# Run directly (downloads binary automatically)
npx memory-mcp-1file

# Or with bun
bunx memory-mcp-1file
```

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
      "args": ["-y", "memory-mcp-1file"]
    }
  }
}
```

## CLI Options

```bash
memory-mcp --help          # Show all options
memory-mcp --db-path /data # Custom database path
```

## Supported Platforms

| Platform | Architecture |
|---|---|
| Linux | x86_64 (musl), ARM64 (musl) |
| macOS | x86_64, ARM64 (Apple Silicon) |
| Windows | x86_64 |

## Links

- [GitHub Repository](https://github.com/pomazanbohdan/memory-mcp-1file)
- [Releases](https://github.com/pomazanbohdan/memory-mcp-1file/releases)
- [Architecture](https://github.com/pomazanbohdan/memory-mcp-1file/blob/master/ARCHITECTURE.md)

## License

MIT
