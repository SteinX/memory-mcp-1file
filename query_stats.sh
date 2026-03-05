#!/bin/bash
docker exec -i memory-mcp-dev curl -X POST -H 'Content-Type: application/json' -d '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"search_code","arguments":{"query":"EmbeddingConfig"}}}' http://127.0.0.1:8000 2>/dev/null || echo "No http server"
