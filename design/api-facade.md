## Search Index Facade

### Purpose

Provide the public embedded-database API (`SearchIndex`/`Database`) and delegate all behavior to the internal `SearchEngine` coordinator.

### Scope

**In scope:**
- Opening an index with schema and config.
- User-facing CRUD/search/admin methods on `SearchIndex`.
- Stats, health checks, low-memory toggles, and transaction entrypoints.

**Out of scope:**
- Concrete WAL/segment/index persistence details.
- Query planning, matching, and scoring internals.
- MVCC snapshot implementation details.

### Primary User Flow

1. Caller creates `Config` and `SchemaWithAnalyzer`.
2. Caller opens `SearchIndex` with `SearchIndex::open` or `open_with_schema`.
3. Caller mutates and queries through facade methods (`add_document`, `search`, `flush`, `commit`, etc.).
4. Caller inspects runtime state via `stats` and `health_check`.

### System Flow

1. Entry point: `src/core/facade.rs:SearchIndex::open`.
2. `SearchEngine::new` builds components through `EngineComponents::assemble` (`src/core/engine.rs`, `src/core/components.rs`).
3. Facade methods delegate directly to engine methods (`write_document`, `run_search`, `collect_stats`, `run_health_check`).
4. Engine uses writer, reader pool, parser, executor, and cache components to perform side effects and return results.

### Data Model

- `Config` (`src/core/config.rs`) fields: `storage_path (PathBuf)`, `memory_limit (usize)`, `cache_size (usize)`, `writer_batch_size (usize)`, `writer_commit_interval_secs (u64)`, `writer_max_segment_size (usize)`, `max_readers (usize)`, `buffer_pool_size (Option<usize>)`, `indexing_threads (Option<usize>)`, `compression (compression::CompressionType)`, `merge_policy (MergePolicyType)`.
- `Document` (`src/core/types.rs`) fields: `id (DocId)`, `fields (HashMap<String, FieldValue>)`. Persisted via segment/WAL subsystems.
- `FieldValue`: `Text(String)`, `Number(f64)`, `Date(DateTime<Utc>)`, `Boolean(bool)`.
- `Error` (`src/core/error.rs`) fields: `kind (ErrorKind)`, `context (String)`.

### Interfaces and Contracts

- `SearchIndex::open(schema, config) -> Result<SearchIndex>`.
- `SearchIndex::add_document(doc) -> Result<()>` writes through single-writer path.
- `SearchIndex::delete_document(id) -> Result<()>` performs soft delete via MVCC deleted bitmap.
- `SearchIndex::search(query) -> Result<Vec<ScoredDocument>>` returns top-10 hits by default.
- `SearchIndex::search_n(query, limit) -> Result<Vec<ScoredDocument>>` returns top-N hits.
- `SearchIndex::search_debug(query, limit) -> Result<SearchResults>` includes timing and optional explanations via executor config.
- `SearchIndex::flush() -> Result<()>` seals current segment buffer.
- `SearchIndex::commit() -> Result<()>` flushes then fsyncs WAL in current implementation.
- `SearchIndex::recover() -> Result<()>` replays discovered WAL files.
- `SearchIndex::stats() -> Result<DatabaseStats>` and `health_check() -> Result<HealthCheckResult>` provide runtime snapshots.

### Dependencies

**Internal modules:**
- `src/core/engine.rs` — engine coordinator receiving all facade calls.
- `src/core/components.rs` — component assembly and wiring.
- `src/schema/schema.rs` — schema object passed at open time.
- `src/search/results.rs` — returned hit/result types.

**External services/libraries:**
- None (embedded library, local filesystem only).

### Failure Modes and Edge Cases

- Open fails with `ErrorKind::Io` when storage directories cannot be created (`StorageLayout::new`).
- Search fails with parse/validation errors from parser/executor (`ErrorKind::Parse`, `ErrorKind::InvalidInput`).
- Recovery can partially succeed and print warnings on failed replayed operations (`src/core/engine.rs:recover`).
- `search` returns empty vectors when no results; no special-case error for no-hit queries.

### Observability and Debugging

- Start at `src/core/facade.rs` to map user API to engine call sites.
- `SearchIndex::search_debug` allows explanation-enabled execution path (`ExecutionConfig::debug`).
- `SearchIndex::stats` includes cache hit/miss counters, QPS/WPS counters, segment counts.
- `SearchIndex::health_check` aggregates component checks (`WAL`, `ReaderPool`, `QueryCache`, `DiskSpace`, optional `Memory`).

### Risks and Notes

- Several facade operations expose behavior that is partially implemented in downstream modules (for example, recovery and transaction read semantics).
- `Database` has multiple compatibility aliases (`core::Database`, `core::database::Database`), which preserves API stability but can hide real implementation location.

Changes:

