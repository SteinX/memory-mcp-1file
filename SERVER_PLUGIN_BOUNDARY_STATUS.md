# Server / Plugin Boundary Status

## Current state

The MCP server repository is effectively **feature-complete for the current non-plugin scope**.

Implemented on the server side:

- lifecycle / generation contract
- relation metadata contract
- public `contract + summary` normalization across outward-facing surfaces
- code-search truthfulness fixes
- identity / frontier semantics
- projection envelope and materialization contract semantics
- export-only on-demand projection builder
- deterministic shaping semantics
- projection request/options contract
- same-process ephemeral projection locator + read-back path
- operator-facing README / help / architecture alignment

## What this repository still owns

Only **closure and truthfulness maintenance** items remain here unless new requirements appear:

- keep docs/help aligned with actual behavior
- preserve additive compatibility discipline
- fix real server-side contract mismatches if plugin integration uncovers them

## What is out of scope here

The following are intentionally **plugin-side responsibilities** and are not implemented in this repository:

- workflow orchestration
- stale/degraded UX
- retry / refresh policy
- plugin-local cache policy
- plugin automation / commands
- durable consumer-side state management

## Safe consumer assumptions

- `contract` and `summary` are additive-first surfaces
- unknown fields / enum values must be ignored by clients
- memory IDs are stable public identities on memory surfaces
- symbol IDs are stable project-scoped identities on symbol surfaces
- `recall_code.results[].id` is local-only; stable re-find locator is `project_id + file_path + start_line + end_line`
- projection locators are opaque, same-process, non-persistable, and not generation-stable
- `project_info(action="projection")` returns an on-demand export-only projection document
- `project_info(action="projection_by_locator")` resolves only locators still present in the current process registry

## Recommended next step

If plugin implementation stays outside this repository, the next actionable artifact is:

- [`PLUGIN_IMPLEMENTATION_PLAN.md`](./PLUGIN_IMPLEMENTATION_PLAN.md)

If future plugin work reveals a real server-side contract defect, reopen this repository only for:

1. contract truthfulness fixes,
2. docs/help alignment,
3. narrow compatibility work.
