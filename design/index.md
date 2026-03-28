# Drusdenx — Design Index

Drusdenx is an embedded Rust search engine/database library (not a standalone server) that provides in-process document indexing, querying, and persistence using WAL + segment files. Runtime behavior is coordinated by a facade (`SearchIndex`) over writer, reader, query, and MVCC components, with optional low-memory and performance-oriented subsystems.

## Feature Matrix

| Feature | Description | File | Status |
|---------|-------------|------|--------|
| Search Index Facade | Public API surface for open/search/mutate/admin operations | [api-facade.md](api-facade.md) | Stable |
| Schema And Text Analysis | Field schema metadata and tokenization/filter pipelines | [schema-analysis.md](schema-analysis.md) | In Progress |
| Query Language And Planning | Parser, AST, validation, planning, and matcher semantics | [query-language.md](query-language.md) | In Progress |
| Search Execution And Scoring | Query executor flow, collectors, result model, and scorer contracts | [search-execution.md](search-execution.md) | In Progress |
| Inverted Index And Index Readers | Posting/dictionary structures and eager/lazy/hybrid index readers | [index-structures.md](index-structures.md) | In Progress |
| Write Path And Storage Durability | WAL, segment persistence, merge/compact behavior, and recovery | [writer-storage.md](writer-storage.md) | In Progress |
| Reader Pool And MVCC Snapshots | Snapshot versioning, reader caching, and deleted-doc filtering | [reader-mvcc.md](reader-mvcc.md) | In Progress |
| Transactions And Isolation | Transaction state machine, staged ops, validation, and commit wrapper | [transactions.md](transactions.md) | In Progress |
| Read Write Topology | Read/write wrapper APIs and round-robin read distribution | [read-write-topology.md](read-write-topology.md) | In Progress |
| Memory And Performance Infrastructure | Low-memory mode, pools, compression, SIMD, and parallel indexing utilities | [memory-performance.md](memory-performance.md) | In Progress |

## Cross-Cutting Concerns

- Error model is centralized in `src/core/error.rs` (`ErrorKind` + context string), with most modules propagating `Result<T>` directly.
- Snapshot consistency is cross-cutting: write operations publish new MVCC snapshots and read/search paths depend on snapshot version + deleted bitmap filtering (`src/mvcc/controller.rs`, `src/reader/reader_pool.rs`, `src/query/matcher.rs`).
- Query caching (`src/query/cache.rs`) is used by both `SearchEngine::run_search` and `ReadDatabase` wrappers, affecting correctness/performance tradeoffs for repeated queries.
- Multiple partially overlapping subsystems exist for writing and performance (session-based writer pipeline, `DataWriter`/`ParallelWriter`, mmap, lazy/hybrid readers), so feature maturity differs across modules.

## Notes


