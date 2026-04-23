# Memory MCP Server Architecture

## High-Level Overview
Memory MCP Server is an autonomous memory system for AI agents, written in Rust. It combines semantic search (vectors), knowledge graph, and code indexing into a single binary without external dependencies.

### Key Components
1. **MCP Server**: Handles requests from clients (IDE, Agents).
2. **Embedding Architecture**: Generates vectors locally using `candle` / `ort`.
3. **Storage Layer**: Embedded SurrealDB for storing vectors, graphs, and metadata.
4. **Codebase Engine**: Indexes code using Tree-sitter (in development).

## Component Diagram (C4 Container)

```mermaid
graph TD
    User[AI Agent / IDE]
    
    subgraph "Memory MCP Server"
        Handler[MCP Handler]
        
        subgraph "Logic Layer"
            L_Mem[Memory Logic]
            L_Search[Search Logic]
            L_Graph[Graph Logic]
            L_Code[Code Logic]
        end
        
        subgraph "Embedding Subsystem"
            E_Service[Embedding Service]
            E_Queue[Adaptive Queue]
            E_Worker[Embedding Worker]
            E_Engine["Inference Engine (Candle)"]
            E_Cache["Embedding Store (L1/L2)"]
        end
        
        subgraph "Storage Layer"
            S_Surreal[SurrealDB Access]
        end

        User -- "MCP Tools" --> Handler

        Handler -- "store_memory" --> L_Mem
        Handler -- "recall/search" --> L_Search
        Handler -- "create_relation" --> L_Graph
        Handler -- "index_project" --> L_Code
        
        L_Mem -- "Embed Content" --> E_Service
        L_Search -- "Embed Query" --> E_Service
        
        E_Service --> E_Queue
        E_Queue -- "Batched Requests" --> E_Worker
        E_Worker -- "Check Cache" --> E_Cache
        E_Worker -- "Run Model" --> E_Engine
        
        L_Mem & L_Search & L_Graph & L_Code --> S_Surreal
        
        S_Surreal -.-> DB[(Embedded Files)]
        E_Cache -.-> DB
    end
```

## Component Details & Algorithms

### 1. Logic Layer
Responsible for request handling, routing, and business logic implementation.

*   **Reciprocal Rank Fusion (RRF)**: Algorithm for merging search results from different sources (Vector Search, BM25, Knowledge Graph).
    *   *Why*: Vector search is good for semantics ("meaning"), BM25 for exact keyword matches, and Graph for relationships. RRF allows taking the best of all three worlds without complex weight tuning.
    *   *Formula*: `score = 1.0 / (k + rank)`
*   **BM25**: Text search algorithm (Okapi BM25). Implemented on top of SurrealDB indexes.

### 2. Embedding Subsystem
Critical component for semantic search. Operates autonomously.

*   **Adaptive Queue**: Smart queue regulating vectorization request rate (Backpressure).
    *   *Algorithm*: Monitors queue depth and slows down new requests (`THROTTLE_DELAY_MS`) if the queue is filled > 80% (`HIGH_WATERMARK`).
    *   *Why*: Prevents OOM (Out of Memory) during massive file indexing.
*   **Inference Engine (Candle)**: Uses the `candle` library (Huggingface) to run BERT-like models (nomic-embed, e5) on CPU. Does not require Python.
*   **L1/L2 Cache**:
    *   L1: LRU Cache in RAM for most frequent requests.
    *   L2: Disk cache (Sled/SurrealDB) to avoid re-vectorizing unchanged content.

### 3. Graph Algorithms
Used for analyzing relationships between entities (files, functions, notes).

*   **Personalized PageRank (PPR)**: Algorithm for ranking graph nodes relative to "seed" nodes.
    *   *Application*: When a user searches for "Authorization", we find the "Authorization" node, and PPR finds all related concepts (e.g., "Login", "JWT", "OAuth"), even if the text doesn't contain the word "Authorization".
    *   *Hub Dampening*: Modification to reduce the weight of "super-nodes" (linked to everything) to avoid noise.
*   **Leiden Algorithm**: Community Detection algorithm.
    *   *Why*: Groups closely related files or concepts into clusters. Helps understand the modular structure of the project.

### 4. Codebase Engine
Responsible for understanding code.

*   **Tree-Sitter Chunking**: Smart code splitting into fragments (chunks) based on Abstract Syntax Tree (AST), rather than just lines.
    *   *Logic*: Respects function and class boundaries. Large functions are broken down into smaller logical blocks, preserving context.
    *   *Why*: Vector search works better with logically complete code pieces than with arbitrary text slices.
*   **Content Hashing (Blake3)**: Fast hashing for deduplication. If a file hasn't changed, it's not re-indexed.

## Data Flow: Store Memory

```mermaid
sequenceDiagram
    participant Agent
    participant MCP as MCP Server
    participant Embed as Embedding Service
    participant DB as SurrealDB

    Agent->>MCP: store_memory(content: "...")
    MCP->>Embed: embed(content)
    Embed-->>MCP: [0.12, -0.45, ...] (Vector)
    MCP->>DB: CREATE memory SET content=..., embedding=...
    DB-->>MCP: Memory ID
    MCP-->>Agent: Memory ID
```

## Data Flow: Search (Recall / Hybrid Search)

```mermaid
sequenceDiagram
    participant Agent
    participant MCP as MCP Server
    participant Embed as Embedding Service
    participant DB as SurrealDB

    Agent->>MCP: recall(query: "...")
    par Vector Search
        MCP->>Embed: embed(query)
        Embed-->>MCP: Vector
        MCP->>DB: SELECT * FROM memory WHERE embedding <|5|> vector
    and Stats / Graph Search
        MCP->>DB: SELECT * FROM memory WHERE content CONTAINS query
    end
    DB-->>MCP: Results
    MCP->>MCP: Re-rank (RRF)
    MCP-->>Agent: Top Results
```

## Module Structure (Crate Structure)

* `src/main.rs`: Entry point, CLI initialization, and services.
* `src/server/`: MCP protocol implementation and tool routing.
* `src/embedding/`: Wrapper around `candle` for local model inference.
* `src/storage/`: Abstraction over SurrealDB.
* `src/graph/`: Graph algorithms (PageRank, Community Detection).
* `src/codebase/`: Code indexing and chunking logic.

## Plugin-Facing MCP Contract Notes

Phase 5A freezes the public integration contract for the current code/project read surfaces without introducing new product capabilities.

### Additive compatibility policy

- Public MCP responses use additive `contract` and `summary` metadata.
- Clients must ignore unknown fields and unknown enum values.
- Internal DB record shapes and local edge/chunk IDs are not public contract.

### Canonical reason taxonomy

`summary.partial.reason_code` is the machine-readable contract field for partial/degraded/unsupported states. The current exported values are:

- `missing`
- `stale`
- `partial`
- `degraded`
- `invalid_locator`
- `generation_mismatch`
- `unsupported`

Legacy string `summary.partial.reason` values are still emitted for compatibility, but plugin logic should prefer `reason_code`.

### Projection locator lifecycle

`project_info(action="projection")` returns an on-demand export-only projection plus a locator record. That locator is intentionally limited:

- scope: same-process ephemeral projection registry
- opaque: clients must not parse locator internals as contract
- non-persistable: locators are convenience handles, not durable IDs
- not generation-stable: a locator is bound to the semantic generation captured at projection creation time
- same-process readback only: `project_info(action="projection_by_locator")` resolves only locators present in the current process registry

The locator record now includes typed lifecycle and lookup metadata:

- `locator.lifecycle.*` describes survivability, scope, and persistence guarantees
- `locator.lookup.state` distinguishes `created`, `resolved`, and `missing`
- `locator.lookup.reason_code` is populated on structured miss/failure cases such as `invalid_locator`

### Stable vs transient identities

- Memory APIs: public memory IDs are stable read/list/search identities.
- Symbol APIs: symbol IDs are stable project-scoped identities.
- Code recall APIs: `results[].id` is local-only; the stable re-find locator is `project_id + file_path + start_line + end_line`.
- Projection locators: transient handles only, never promoted to stable public identity.
