#!/bin/bash
for i in {1..3}; do
  echo "--- Try $i ---"
  docker exec -i memory-mcp-dev memory-mcp << 'JSON'
{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"search_code","arguments":{"query":"Gemma"}}}
JSON
  echo ""
  sleep 5
done
