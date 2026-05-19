# Roadmap

This repository currently focuses on the MCP server itself: memory persistence,
retrieval, lifecycle safety, code intelligence, and public tool contracts.

## Current Status

| Phase | Status |
|---|---|
| Phase 0: Baseline foundations | Complete |
| Phase 1: Canonical contract foundation | Complete |
| Phase 2: Public surface normalization | Complete |
| Phase 3: Later-phase contract freeze and MVP preparation | Complete for the MCP server repo |
| Phase 4: Projection builder, non-plugin scope | Complete for the MCP server repo |
| Phase 5: Plugin-facing workflow integration | Out of scope unless future work explicitly moves it here |

## Repository Closure Status

From the MCP server repository perspective, remaining work is handoff and
maintenance rather than major new server capability:

- public `contract` and `summary` metadata is in place across memory, graph,
  code search, symbol, and project surfaces;
- projection/materialization semantics are explicit;
- projection locator read-back is same-process and ephemeral;
- stable vs transient identity rules are documented;
- plugin orchestration, stale UX, retry policy, and workflow commands belong
  outside this repo unless future scope changes.

See also [ARCHITECTURE.md](../ARCHITECTURE.md) for plugin-facing MCP contract
notes.

## Research Ideas

These are not current promises.

### Meta-Cognitive Reflection

Raw memories accumulate noise. A future reflection process could scan recent
memories, de-duplicate redundant entries, resolve conflicts, and synthesize
low-level facts into higher-level insights.

### Temporal Decay

Old memories can drown out current context in semantic search. A future ranking
layer could add time decay to Reciprocal Rank Fusion and give recency-sensitive
queries a controlled boost.

### Namespaced Memory Banks

A future namespace or project scope could let one server host isolated memory
banks for multiple projects or agent personas without one container per project.

### Epistemic Confidence

Future memory schemas could support confidence scores so agents can distinguish
verified decisions from hypotheses.
