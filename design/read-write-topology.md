## Read Write Topology

### Purpose

Offer optional read/write separation wrappers around a single embedded database instance for load-distribution patterns.

### Scope

**In scope:**
- `ReadDatabase` search wrapper.
- `WriteDatabase` mutation wrapper.
- `ReadLoadBalancer` round-robin replica selection.
- `MasterSlaveDatabase` composition and routing.

**Out of scope:**
- True multi-node replication or network transport.
- Storage-layer durability internals.
- Query parsing/scoring algorithm details.

### Primary User Flow

1. Caller constructs `MasterSlaveDatabase::new(config, schema, read_replicas)`.
2. Writes (`add_document`, `delete_document`, `flush`, `commit`) route to write handle backed by the master.
3. Reads (`search`) route through read load balancer over created read wrappers.
4. Caller inspects stats/health from master.

### System Flow

1. `MasterSlaveDatabase::new` opens one master `Database` (`SearchIndex`).
2. `ReadDatabase::create_replicas` clones read wrappers sharing master components (reader pool, cache, parser, executor).
3. `ReadLoadBalancer::get_replica` chooses next replica index via atomic round-robin counter.
4. `ReadDatabase::search_with_limit` checks query cache, parses query, gets reader, executes query, then updates cache.
5. `WriteDatabase` methods lock writer and call writer operations directly.

### Data Model

- `ReadDatabase` fields: `reader_pool`, `query_cache`, `query_executor`, `query_parser`.
- `WriteDatabase` field: `writer (Arc<RwLock<IndexWriter>>)`.
- `ReadLoadBalancer` fields: `replicas (Vec<ReadDatabase>)`, `current (AtomicUsize)`.
- `MasterSlaveDatabase` fields: `master`, `read_balancer`, `write_db`.
- Persistence rule: wrappers are runtime routing objects only; persistence remains in shared master storage.

### Interfaces and Contracts

- `ReadDatabase::from_database(db)` and `create_replicas(db, count)`.
- `ReadDatabase::search(query)` / `search_with_limit(query, limit)`.
- `WriteDatabase::{add_document, add_documents_batch, delete_document, flush, commit, compact}`.
- `ReadLoadBalancer::search(query)` and `get_replica()`.
- `MasterSlaveDatabase::{search, add_document, delete_document, flush, commit, stats, health_check}`.

### Dependencies

**Internal modules:**
- `src/core/database_rw.rs` — topology wrappers.
- `src/core/facade.rs` — underlying master API.
- `src/query/cache.rs`, `src/reader/reader_pool.rs`, `src/search/executor.rs` — shared read-path components.

**External services/libraries:**
- None (all wrappers are in-process).

### Failure Modes and Edge Cases

- `ReadLoadBalancer::get_replica` uses modulo by `replicas.len()` and can panic when `read_replicas == 0`.
- Read replicas are logical wrappers over shared state, not isolated physical replicas; heavy read/write contention still targets one process and one storage backend.
- `ReadDatabase::reader_stats` currently reports hardcoded active-reader count `0`.

### Observability and Debugging

- Debug routing by inspecting `ReadLoadBalancer::current` increment behavior and selected index.
- Cache hit/miss behavior remains visible through master stats cache counters.
- No built-in per-replica latency or load metrics.

### Risks and Notes

- Naming suggests distributed master/slave semantics, but current implementation is an in-process abstraction over shared components.
- Useful as API-level separation pattern, not as horizontal scale-out replication mechanism.

Changes:

