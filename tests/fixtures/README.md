# Learning memory contract fixtures

- `learning_memory_contract.json` is the canonical plugin-facing fixture set.
- Stable public contract fields are the `contract`, `summary`, and `learning_summary` objects inside each response.
- The `record` payloads are compatibility examples for plugin consumers and may vary by implementation.
- Non-happy-path coverage includes `unsupported`, `degraded`, `stale`, and `generation_mismatch` reason codes.
