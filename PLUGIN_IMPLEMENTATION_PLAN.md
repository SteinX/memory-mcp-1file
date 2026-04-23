# Plugin Implementation Plan

## Objective

Implement the plugin-side consumption of the current MCP server contract without reopening already-finished server contract work.

This plan assumes:

- the MCP server in this repository already owns canonical truth,
- plugin code is implemented **outside** this repository,
- plugin work should consume current `contract` / `summary` / projection / locator semantics as authoritative.

## Ownership split

### Server owns

- lifecycle / generation truth
- relation metadata truth
- identity semantics
- frontier semantics
- projection envelope and materialization contract semantics
- projection request/options contract
- ephemeral locator creation and same-process locator read-back semantics

### Plugin owns

- workflow orchestration
- stale / degraded UX behavior
- retry / refresh policy
- local cache policy
- local persistence of safe identifiers only
- operator commands / shortcuts / automation
- rollout and observability on the plugin side

### Plugin must not do

- treat projection locators as durable IDs
- reinterpret server identity semantics
- depend on local-only server fields as stable references
- assume server-side orchestration or persistence exists where the contract says it does not

## Server contract assumptions

The plugin should be built against these assumptions:

1. **Additive compatibility**
   - `contract` and `summary` are additive-first.
   - Unknown fields / enum values must be ignored.

2. **Stable identities**
   - memory surfaces: stable public memory IDs
   - symbol surfaces: stable project-scoped symbol IDs
   - code recall surfaces: stable re-find locator = `project_id + file_path + start_line + end_line`

3. **Transient identities**
   - `recall_code.results[].id` is local-only
   - edge IDs are local-only / non-persistable
   - projection locators are opaque, same-process, non-persistable, and not generation-stable

4. **Projection semantics**
   - `project_info(action="projection")` returns an on-demand export-only projection document plus locator metadata
   - `project_info(action="projection_by_locator")` resolves only locators still present in the same process
   - `projection.summary.partial` is the canonical outward truth for stale / degraded / not-fully-current projection payloads

5. **Reason handling**
   - prefer machine-readable `summary.partial.reason_code`
   - treat legacy `summary.partial.reason` as compatibility-only text

## Plugin implementation phases

### Phase P1 — Contract ingestion layer

Build one adapter layer that parses raw MCP JSON into plugin-side domain models.

Tasks:

- define plugin-side types for:
  - contract metadata
  - summary envelope
  - projection envelope
  - projection materialization semantics
  - shaping semantics
  - locator lifecycle / lookup state
  - identity classes (stable public / stable project-scoped / local-only / transient)
- centralize parsing of `reason_code`
- centralize tolerance of unknown fields / enum values

Done when:

- plugin logic never reads raw MCP JSON directly outside the adapter layer

### Phase P2 — Projection request / read orchestration

Implement the minimum working read path.

Tasks:

- request projections with:
  - default `relation_scope = all`
  - default `sort_mode = canonical`
- support explicit scopes:
  - `calls`
  - `imports`
  - `type_links`
  - `none`
- parse and store the returned locator only in short-lived in-memory plugin state
- implement immediate same-process read-back path via `projection_by_locator`

Done when:

- plugin can request a projection, read it back by locator, and still function if read-back is unavailable

### Phase P3 — Locator lifecycle handling

Treat locator limitations as first-class plugin behavior.

Tasks:

- classify locator states:
  - created
  - resolved
  - missing / invalid
- never persist locator as a durable bookmark
- on locator miss:
  - request a fresh projection instead of retrying indefinitely
- explicitly model that locator does **not** survive generation changes or process restart

Done when:

- the plugin has no code path that assumes locator durability

### Phase P4 — Degraded / stale UX behavior

The plugin must tell the truth about projection freshness.

Tasks:

- map `summary.partial.reason_code` to UI states:
  - stale
  - partial
  - degraded
  - invalid_locator
  - generation_mismatch
  - missing
  - unsupported
- define what the user sees when projection is:
  - current
  - stale but usable
  - locator missing
  - fresh projection required
- ensure stale projection is never shown as unquestionably current

Done when:

- user-visible projection state is driven by machine-readable contract fields, not guessed heuristics

### Phase P5 — Cache policy and safe persistence

Implement plugin-local caching only on top of safe identity rules.

Tasks:

- cache projection payloads only as plugin-local transient or policy-bounded state
- persist only safe identities where needed:
  - memory IDs
  - symbol IDs
  - code re-find tuples (`project_id + file_path + start_line + end_line`)
- never persist:
  - locators
  - local-only result IDs
  - local-only edge IDs

Done when:

- plugin cache cannot outlive or contradict server truth semantics

### Phase P6 — Workflow integration

Embed the read path into real plugin workflows.

Tasks:

- choose workflow entrypoints where projection is actually needed
- integrate:
  - build projection
  - optional immediate read-back
  - stale/degraded display
  - refresh behavior
- keep workflow logic entirely plugin-side

Done when:

- end-to-end plugin flows work without requiring new server orchestration capability

### Phase P7 — Validation and rollout

Prove the plugin is consuming the server contract correctly.

Tests to include:

- happy path projection request
- immediate read-back success path
- locator missing / invalid path
- stale projection path
- `none` / `calls` / `imports` / `type_links` scope behavior
- unknown future additive fields ignored safely
- no persistence of local-only IDs or locators

Rollout steps:

- gate behind a feature flag if needed
- collect telemetry on:
  - projection request success/failure
  - locator read-back success/miss
  - stale/degraded occurrences
  - refresh frequency

## Recommended execution order

1. P1 Contract ingestion layer
2. P2 Projection request / read orchestration
3. P3 Locator lifecycle handling
4. P4 Degraded / stale UX behavior
5. P5 Cache policy and safe persistence
6. P6 Workflow integration
7. P7 Validation and rollout

## Risks

### Risk 1 — Locator treated as durable identity
Mitigation:
- make locator type transient-only in plugin code
- do not serialize/store it

### Risk 2 — Plugin persists local-only code result IDs
Mitigation:
- persist only `project_id + file_path + start_line + end_line`

### Risk 3 — Stale projection shown as current
Mitigation:
- always drive UX from `summary.partial` and lifecycle/projection contract

### Risk 4 — Plugin reinterprets server identity semantics
Mitigation:
- one authoritative adapter/domain layer
- no alternative plugin-defined identity mapping

### Risk 5 — Scope drift back into server
Mitigation:
- keep workflow, retries, cache policy, and UX state plugin-side

## Handoff checklist

Before plugin work starts, confirm these artifacts exist and are treated as authoritative:

- `README.md`
- `ARCHITECTURE.md`
- `SERVER_PLUGIN_BOUNDARY_STATUS.md`
- this file: `PLUGIN_IMPLEMENTATION_PLAN.md`

And confirm these server surfaces are the basis for integration:

- `project_info(action="status")`
- `project_info(action="stats")`
- `project_info(action="projection")`
- `project_info(action="projection_by_locator")`
- `recall_code`
- `search_symbols`
- `symbol_graph`
- `knowledge_graph(action="get_related")`

## Final recommendation

For this repository:

- do only final closure / handoff work
- do **not** reopen server feature development unless plugin integration reveals a real contract mismatch

For the plugin side:

- proceed directly against the current server contract
- treat the server as the source of truth for lifecycle, generation, projection freshness, identity classes, and locator semantics
- keep orchestration, cache, retry, and UX entirely plugin-side
