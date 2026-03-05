## Release v0.8.0: The Gemma Migration & Extreme Memory Optimization 🚀

This release marks a fundamental shift in the architecture of `memory-mcp`, solving the most critical issue reported by users: **out-of-memory (OOM) crashes** during codebase indexing.

### What's New:
* **Model Migration (Qwen → Gemma2):** We migrated the default embedding model from `Qwen3-1.5B` to `unsloth/embeddinggemma-300m-qat-q4_0-unquantized`. This drops the memory footprint of the model from ~3GB down to just ~195MB while maintaining top-tier retrieval performance!
* **Zero Configuration:** The new Gemma model is fully open. You no longer need a HuggingFace account, HF_TOKEN, or any license agreements to run `memory-mcp`. It just works out of the box.
* **Mimalloc Allocator:** Replaced the system allocator with `mimalloc`. This drastically reduces memory fragmentation (especially on Alpine/Musl) and significantly boosts multi-threaded processing speeds.
* **SurrealDB Stability (Throttling):** We implemented smart batch-throttling during indexation. The indexer now pauses for 100-150ms after inserting vectors, completely eliminating `Transaction write conflict` (OCC Retries) inside SurrealDB.
* **768d Vectors:** The model natively generates and searches against 768-dimensional vectors with `last_token_pooling` for immense accuracy. The database schema dynamically rebuilds its `HNSW` indices to accommodate the new dimension.
* **Hardware Acceleration:** Native release builds now enable `x86-64-v3` target optimizations, speeding up the underlying tensor math via AVX2.
* **Cleanup:** Removed the broken `accelerate` feature from Cargo to ensure proper compilation on Linux.

### Performance:
On a standard system, the container now sits comfortably at **~350MB of RAM usage** (down from ~4GB!) during massive codebase indexation, keeping your system fast and responsive.
