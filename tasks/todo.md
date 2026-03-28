# load-design task plan

## Discovery

- [x] Glob all `design/*.md` feature files (excluding `index.md`) and parse non-empty `Changes:` blocks
- [x] Build pending-item inventory with per-feature mapping
- [x] Report pending inventory: 12 items across 6 features (`query-language`, `search-execution`, `writer-storage`, `reader-mvcc`, `transactions`, `memory-performance`)

## Implementation Order

- [x] Query parsing semantics
  - [x] Fix boolean `NOT` parsing to populate `must_not`
  - [x] Add explicit `PrefixQuery` parse path (e.g. `title:pre*`)
- [x] Optimization safety
  - [x] Guard planner/plan-to-query optimization roundtrip to preserve semantics for non-term query classes
  - [x] Add regression tests proving optimization-on/off equivalence behavior for phrase/range/wildcard/fuzzy paths
- [x] Search pipeline API safety
  - [x] Remove panic surface from `SearchPipeline::execute(&mut self)` by returning explicit error path
- [x] Writer durability and batch correctness
  - [x] Preserve full document fields in `add_documents_batch` large-batch path
  - [x] Strengthen WAL commit/recovery boundaries (commit marker + replay-after-last-commit logic)
  - [x] Add targeted tests for batch field preservation and WAL replay idempotency boundary behavior
- [x] Reader/MVCC robustness
  - [x] Record/log segment-open failures in reader construction and expose failure count through stats/health surfaces
  - [x] Rework MVCC lease-aware GC eviction checks for predictable old-version cleanup
  - [x] Add controlled MVCC GC tests for pinned vs unpinned version eviction behavior
- [x] Transaction correctness
  - [x] Implement snapshot-backed `Transaction::read` for non-write-set documents
  - [x] Upgrade repeatable-read/serializable validation to detect document-level conflicts
  - [x] Add transaction tests for snapshot reads and conflicting-write detection
- [x] Memory/compression behavior
  - [x] Enforce memory-tracker allocation failures in write path (propagate `OutOfMemory`)
  - [x] Wire `Config.compression` into segment/index artifact compression choice for new writes
  - [x] Add tests verifying compression config affects newly written segment/index blocks

## Docs + Reset

- [x] Update affected `design/*.md` sections so docs match implemented behavior
- [x] Clear only implemented `Changes:` items (leave blocked items with `Blocked:` prefix if any)

## Verification

- [x] Run targeted tests for each changed subsystem
- [x] Run full project test suite (or strongest available check) and record results
- [x] Add review notes and evidence to this file

## Review

- Implemented all 12 pending `Changes:` items across 6 feature docs with no blocked items.
- Query language/search execution updates:
  - Parser now maps boolean `NOT` into `must_not` and emits `PrefixQuery` for field-prefix patterns (e.g. `title:pre*`).
  - Executor/pipeline optimization now skips unsafe roundtrips and preserves original queries for non-term/must_not/filter cases.
  - `SearchPipeline::execute(&mut self)` no longer panics; it returns explicit `InvalidState` and points callers to `run(self, ...)`.
- Writer/storage updates:
  - `add_documents_batch` large-batch path preserves full document fields.
  - Commit now appends `Operation::Commit`; recovery replays only operations after last commit marker per WAL file.
  - Recovery applies operations through `IndexWriter::apply_recovered_operation` to avoid re-appending each recovered operation.
- Reader/MVCC updates:
  - Reader pool logs/counts segment-open failures (`segment_open_failure_count`) and exposes count via stats/health.
  - MVCC GC now evicts unpinned old versions predictably (pinned only when strong_count > 1), with controlled tests.
- Transaction updates:
  - `Transaction::read` now performs snapshot-backed segment reads for non-write-set docs.
  - Repeatable-read/serializable validation now checks document-level conflicts for tracked read/write operation doc IDs.
- Memory/compression updates:
  - Low-memory write path now propagates `OutOfMemory` from `MemoryTracker::allocate`.
  - `Config.compression` is wired into writer/segment/index block compression for new artifacts.
- Verification evidence:
  - Targeted tests run and passed for parser, optimization, pipeline API safety, writer batch preservation, compression wiring, reader failure tracking, MVCC GC, transaction read/conflict checks, and recovery boundary helper logic.
  - Full suite: `cargo test` passed (`17 passed, 0 failed`; doc-tests passed).
