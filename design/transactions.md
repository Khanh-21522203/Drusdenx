## Transactions And Isolation

### Purpose

Provide an ACID-style API for batching document mutations with isolation-level checks before applying operations.

### Scope

**In scope:**
- `Transaction` lifecycle (`begin`, `insert/update/delete`, `commit`, `rollback`).
- `TransactionManager` tracking and stats.
- `SearchIndex::{begin_transaction,transaction,with_transaction}` wrappers.

**Out of scope:**
- Low-level WAL and segment write mechanics.
- Query scoring/search pipeline internals.
- Reader cache behavior.

### Primary User Flow

1. Caller starts a transaction with an isolation level.
2. Caller stages operations (`insert`, `update`, `delete`) in transaction state.
3. Caller commits; transaction validates read set for non-read-committed levels.
4. Engine wrapper applies returned operations to the main writer path and flushes.
5. On failure, caller rolls back and staged state is cleared.

### System Flow

1. Entry points: `SearchIndex::begin_transaction` and `SearchIndex::with_transaction` (`src/core/facade.rs`).
2. Transaction object initializes snapshot/read-write sets in `Transaction::begin`.
3. Mutations append `TransactionOp` entries in in-memory operation log.
4. `Transaction::commit` performs prepare phase and optional optimistic validation by comparing tracked document states between transaction snapshot and current snapshot.
5. `SearchEngine::with_transaction` executes committed ops by invoking write/delete APIs, then flushes segments.

### Data Model

- `TransactionState`: `Active`, `Preparing`, `Committed`, `Aborted`.
- `TransactionOp`: `Insert(Document)`, `Update(DocId, Document)`, `Delete(DocId)`.
- `Transaction` fields: `id`, `isolation_level`, `state`, `operations`, `snapshot`, `read_set (HashMap<DocId, u64>)`, `write_set (HashMap<DocId, Document>)`, `storage`, `mvcc`.
- `TransactionStats` fields: `total`, `active`, `preparing`.
- Persistence rule: transaction state is in-memory; persistence occurs only when engine applies operations through writer path.

### Interfaces and Contracts

- `Transaction::begin(mvcc, isolation_level) -> Transaction`.
- `Transaction::insert(doc)`, `update(doc_id, doc)`, `delete(doc_id)` require `Active` state.
- `Transaction::commit() -> Result<Vec<TransactionOp>>` returns ops to caller/executor.
- `Transaction::rollback() -> Result<()>` sets aborted state and clears staged sets.
- `TransactionManager::begin_transaction(isolation_level) -> Arc<Transaction>`.
- `SearchIndex::transaction` and `with_transaction` execute user closure and apply/rollback automatically.
- `Transaction::read(doc_id)` now reads staged writes first, then resolves visible documents from snapshot segment files.

### Dependencies

**Internal modules:**
- `src/mvcc/controller.rs` — snapshot version references for validation.
- `src/core/engine.rs` — executes returned transaction operations.
- `src/core/types.rs` — document/ID types.

**External services/libraries:**
- None beyond standard synchronization primitives.

### Failure Modes and Edge Cases

- Any mutation call on non-active transaction returns `ErrorKind::InvalidState`.
- Repeatable-read/serializable validation now performs document-level conflict checks across read/write-tracked document IDs.
- `Transaction::read` now resolves non-write-set documents from snapshot segment files; segment read failures surface as transaction read errors.
- Engine-side `with_transaction` applies each operation via regular write/delete APIs; partial failures can occur mid-application if a write fails.

### Observability and Debugging

- Inspect transaction state transitions in `src/core/transaction.rs`.
- `TransactionManager::stats()` provides active/preparing counts.
- No persistent audit log of transaction boundaries or operation traces.

### Risks and Notes

- Transaction behavior now includes snapshot-backed reads and document-level optimistic conflict detection for repeatable-read/serializable paths.
- Two transaction structs exist (`core::transaction::Transaction` and `mvcc::controller::Transaction`) with different roles, which can confuse maintenance.

Changes:
