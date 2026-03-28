## Write Path And Storage Durability

### Purpose

Persist document mutations durably using WAL + segment files and publish new readable snapshots.

### Scope

**In scope:**
- `writer::IndexWriter` mutation lifecycle.
- WAL append/sync/replay mechanics.
- Segment `.seg` and `.idx` write format handling.
- Merge policy and compaction behavior.
- Storage layout/path conventions.

**Out of scope:**
- Query parser/scorer logic.
- Reader-pool caching policy.
- Transaction API state machine internals.

### Primary User Flow

1. Caller invokes `add_document` or `delete_document`.
2. Writer appends operation to WAL and mutates in-memory segment buffers.
3. Caller invokes `flush` or reaches batch threshold, producing new segment files.
4. Caller invokes `commit` to sync durability boundary.
5. On startup recovery, caller may invoke `recover` to replay only operations after the latest commit marker in each WAL file.

### System Flow

1. Entry points: `src/writer/index_writer.rs:{add_document,delete_document,flush,commit,compact}`.
2. Add flow: writer lock acquired -> parallel indexing -> `WAL::append(Operation::AddDocument)` -> `SegmentWriter::write_document` -> optional segment rollover -> `MVCCController::create_snapshot`.
3. Delete flow: lock acquired -> `WAL::append(Operation::DeleteDocument)` -> new snapshot with updated `deleted_docs` bitmap.
4. Flush flow: finalize current segment via `SegmentWriter::finish`, optionally trigger async merge policy evaluation, then publish snapshot.
5. Commit flow: `flush` then `WAL::append(Operation::Commit)` then `WAL::sync`.
6. Recovery flow: discover WAL files (`WAL::find_wal_files`), read length-prefixed entries (`WAL::read_entries`), replay only post-commit tail operations using recovery-specific writer paths that do not re-append each recovered operation.

```
add_document
  └── IndexWriter::add_document
        ├── WAL::append(AddDocument)
        ├── SegmentWriter::write_document
        └── [batch threshold reached]
              ├── SegmentWriter::finish -> .seg + optional .idx
              └── MVCCController::create_snapshot
```

### Data Model

- `WriterConfig` fields: `batch_size`, `commit_interval (Duration)`, `max_segment_size`, `compression`.
- `StorageLayout` fields: `base_dir`, `segments_dir`, `idx_dir`, `wal_dir`, `meta_dir`.
- `WAL` fields: `file`, `position`, `sync_mode`, `sequence`.
- `WALEntry` fields: `sequence`, `operation`, `timestamp`.
- `Operation` enum: `AddDocument(Document)`, `UpdateDocument(Document)`, `DeleteDocument(DocId)`, `Commit`.
- `Segment` fields: `id`, `doc_count`, `metadata`.
- `SegmentMetadata` fields: `created_at`, `size_bytes`, `min_doc_id`, `max_doc_id`.
- Persistence rule: WAL is append-only binary log; segments persist compressed document blocks and per-segment inverted index files.

### Interfaces and Contracts

- `IndexWriter::add_document(doc) -> Result<()>` writes WAL before data segment mutation.
- `IndexWriter::add_documents_batch(docs) -> Result<()>` bulk path with parallel indexing for large batches.
- `IndexWriter::flush() -> Result<()>` seals current segment and publishes snapshot if non-empty.
- `IndexWriter::commit() -> Result<()>` flush + append commit marker + WAL sync.
- `IndexWriter::delete_document(doc_id) -> Result<()>` soft delete only.
- `IndexWriter::compact() -> Result<()>` rewrites segments excluding deleted docs and resets deleted bitmap.
- `WAL::open(storage, sequence)`, `append`, `sync`, `rotate`, `read_entries`, `find_wal_files`.
- `SegmentWriter::new`, `write_document`, `add_index_entry`, `finish`.

### Dependencies

**Internal modules:**
- `src/storage/wal.rs` — durability log.
- `src/storage/segment_writer.rs` and `src/storage/segment_reader.rs` — segment persistence.
- `src/storage/merge_policy.rs` — tiered/log-structured merge policy selection.
- `src/mvcc/controller.rs` — snapshot publication.
- `src/memory/buffer_pool.rs` and `src/parallel/indexer.rs` — write path performance helpers.

**External services/libraries:**
- Filesystem APIs (`std::fs`, `std::io`) for persistence.
- `bincode` for binary serialization.
- `crc32fast` for segment checksum.

### Failure Modes and Edge Cases

- WAL replay reads can hit corrupt entries; deserialization warnings are printed and replay continues.
- `SegmentReader` and query-side segment reading mix fixed header-size seeking with deserialize-to-skip patterns, creating format-offset consistency risk.
- `add_documents_batch` large-doc path now writes original `Document` instances so stored fields are preserved.
- `WAL::open` initializes `position` to zero even when appending existing files, making position-based stats/sync heuristics approximate.
- Commit markers now define replay boundaries; cleanly committed WAL prefixes are skipped during recovery.

### Observability and Debugging

- Recovery path logs warnings and final replay operation count to stderr.
- `DatabaseStats::wal_size_bytes` reads from writer WAL position.
- Start debugging at `IndexWriter::add_document` and `SegmentWriter::finish` for ingestion/durability inconsistencies.
- No structured counters for flush failures, merge failures, or replay errors.

### Risks and Notes

- Recovery applies operations through writer recovery entry points without re-appending each recovered operation into WAL.
- There are multiple overlapping writer abstractions (`IndexWriter`, `DataWriter`, `ParallelWriter`, session-based writer API), increasing maintenance complexity.

Changes:
