import re

# Read mobile-odoo AGENTS.md
with open('/home/unnamed/project/mobile-odoo/AGENTS.md', 'r') as f:
    mobile_content = f.read()

# Read memory-mcp-1file AGENTS.md
with open('/home/unnamed/project/tools/memory-mcp-1file/AGENTS.md', 'r') as f:
    memory_content = f.read()

# Extract from mobile-odoo: from L0 INVARIANTS until STATE MANAGEMENT
start_mobile = mobile_content.find('## ⛔ L0 INVARIANTS')
end_mobile = mobile_content.find('## 📿 STATE MANAGEMENT')
thinking_algorithms = mobile_content[start_mobile:end_mobile].strip()

# Extract from memory-mcp-1file: from Memory Protocol until Rules Summary
start_memory = memory_content.find('# 🧠 Memory Protocol')
end_memory = memory_content.find('## 📋 Rules Summary')
memory_protocol = memory_content[start_memory:end_memory].strip()

# Construct the new AGENTS.md
new_content = f"""# 🤖 AGENTS.md — AI Agent Master Protocol (MCP + VIDA Thinking)

<identity>
You are an AI agent operating with the **Memory MCP** and **VIDA Thinking Framework**.
You must adhere to strict workflows, utilize specialized tools, and maintain context across sessions.
Communication language with the user: Ukrainian.
</identity>

---

{thinking_algorithms}

---

{memory_protocol}

---

## 📋 Rules Summary

| Rule | Description |
|------|-------------|
| **Communication language** | Ukrainian only |
| **Memory: start** | REQUIRED `search_text` + show to user |
| **Memory: completion** | REQUIRED `invalidate` + `store_memory` |
| **Memory: deletion** | FORBIDDEN `delete_memory`, only `invalidate` |
| **Thinking: Boot** | REQUIRED to read algorithms after context wipe |
| **Thinking: Routing** | REQUIRED to identify phases and load specific commands |

---

*Last updated: 2026-02-28*
"""

with open('/home/unnamed/project/tools/memory-mcp-1file/AGENTS.md', 'w') as f:
    f.write(new_content)

print("AGENTS.md successfully updated!")
