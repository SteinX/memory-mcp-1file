# DECISION: Workaround for SurrealDB v3 FULLTEXT search bug

**Date:** 2026-02-24
**Status:** Accepted

## Context
During the implementation of the hybrid search (RRF) for code chunks, we discovered that BM25 scoring was constantly returning `0.0`. 

Upon investigation, we found that:
1. **SurrealDB v3.0.0 has a broken `search::score()` function** for FULLTEXT indexes (tracked in upstream issues like #6852 / #6946).
2. The fallback operator `CONTAINS` conflicts with `FULLTEXT` indexes. If a `FULLTEXT` index exists on a field, `CONTAINS` queries either fail, ignore the index, or return empty results unless the specific `@@` match operator is used (which triggers the broken `search::score()`).

## Decision
1. **Removed `FULLTEXT` index** (`idx_chunks_fts`) from `code_chunks` in `schema.surql` to allow `CONTAINS` to work via sequential scan.
2. **Implemented Rust-side Term Frequency (TF) scoring** for lexical matches as a temporary fallback, since SurrealDB cannot rank results natively right now.
3. **Future Architecture:** We decided to migrate lexical search to **Tantivy** using an in-memory `RamDirectory`. Tantivy will handle BM25 scoring and custom code tokenization (camelCase/snake_case), while SurrealDB will remain the primary store for vectors and graph relations.

## Implications
- Do NOT add `DEFINE INDEX ... FULLTEXT` to `code_chunks` until SurrealDB fixes the `search::score()` upstream bug.
- Lexical search currently uses a basic Rust-side TF calculation (`count / length * 1000.0`), which lacks Inverse Document Frequency (IDF).
- Next major search quality update will introduce `tantivy` to the stack.
