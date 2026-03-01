#!/bin/bash
docker exec -i memory-mcp-dev memory-mcp << 'JSON'
{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"search","arguments":{"query":"Gemma"}}}
JSON
