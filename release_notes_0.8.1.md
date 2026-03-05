## Release v0.8.1: Extreme Parsing Optimizations 🚀

This release focuses on removing artificial bottlenecks in the codebase indexer, allowing `memory-mcp` to fully utilize your CPU and index projects significantly faster without any data loss.

### What's New:
* **100% Data Retention (No Truncation):** We removed the `MAX_CHUNKS_PER_FILE` limit (previously set to 50). The system now extracts and generates vectors for **every single function and symbol** in massive files (like 10k-line auto-generated code or complex models), completely eliminating blind spots in semantic search!
* **Dynamic Multi-Threading:** The file parser now automatically scales `MAX_CONCURRENT_PARSES` based on your available CPU cores (`num_cpus / 2`), drastically speeding up the AST generation phase.
* **Throttle Removal:** We removed the artificial `sleep` delays in the indexer pipeline that were previously added to mitigate SurrealDB OCC conflicts.
* **Optimized Database Writes:** Increased the transaction batch size from 12 to 100. This dramatically reduces the frequency of write transactions, allowing SurrealDB to handle the massive influx of 768d vectors smoothly without needing sleep delays.

*Note: You may see `Transaction write conflict` warnings in the logs during initial indexing. This is normal, healthy behavior for SurrealDB's Optimistic Concurrency Control under heavy load. The system automatically retries and zero data is lost.*
