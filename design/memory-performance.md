## Memory And Performance Infrastructure

### Purpose

Provide memory-control and throughput optimization utilities used by write/search paths and benchmark workloads.

### Scope

**In scope:**
- Low-memory mode and adaptive reclamation.
- Buffer/memory pool helpers.
- Compression and integer encoding utilities.
- SIMD-like set/scoring helpers.
- Parallel indexing utilities.
- Exposed but not fully wired infrastructure (`mmap`, `parallel::merger`, streaming helpers).

**Out of scope:**
- Core API routing and facade semantics.
- Transaction state machine behavior.
- Query AST/planner details.

### Primary User Flow

1. Caller optionally enables low-memory mode through facade API.
2. Write path updates memory tracker and checks pressure during document ingestion.
3. Under pressure, adaptive manager clears caches/flushes buffers and may swap cold data.
4. Compression and SIMD utilities are used by storage/index helpers for throughput.
5. Parallel indexer processes document batches using rayon worker threads.

### System Flow

1. Entry points: `SearchIndex::{enable_low_memory_mode,get_memory_pressure,maybe_reclaim_memory}`.
2. Engine write path estimates document memory footprint and records allocation via `MemoryTracker`; allocation failures now propagate as write errors.
3. If pressure is high, engine triggers `LowMemoryMode::maybe_reclaim`.
4. Reclaim path calls `AdaptiveManager::{clear_caches,flush_buffers}` and optional `SwapManager::swap_cold_data`.
5. Segment/index persistence uses `CompressedBlock` and integer encoders for serialized artifacts; writer compression comes from `Config.compression` / `WriterConfig.compression`.
6. `SimdOps` supports union/intersection and scoring math helpers used by index/query helpers.

### Data Model

- `LowMemoryConfig` fields: `heap_limit`, `buffer_size`, `cache_size`, `batch_size`, `enable_compression`, `swap_to_disk`, `gc_threshold`.
- `LowMemoryMode` fields: `config`, `memory_tracker`, `adaptive_manager`, `swap_manager`.
- `MemoryPool` fields: `blocks`, `free_list`, `total_size`, `used_size`.
- `BufferPool` fields: size-class queues in `HashMap<usize, BufferQueue>`, `memory_limit`.
- `CompressedBlock` fields: `data`, `original_size`, `compression`.
- `EncodedIntegerBlock` fields: `data`, `original_count`, `encoding`.
- `ParallelIndexer` fields: `workers`, `batch_size`, `progress`.
- Persistence rule: these modules mostly provide runtime memory/CPU behavior; compressed outputs are persisted through segment/index writers.

### Interfaces and Contracts

- `LowMemoryMode::{new,is_enabled,memory_pressure,maybe_reclaim}`.
- `BufferPool::{new,get,return_buffer}` for pooled byte buffers.
- `MemoryTracker::{allocate,deallocate,current_usage}` returns `OutOfMemory` when configured limit is exceeded; write path now propagates this error instead of ignoring it.
- `CompressedBlock::{compress,decompress,compress_auto}` for block codecs (`None/LZ4/Zstd/Snappy`).
- `EncodedIntegerBlock::{encode,decode,compress_with_lz4}` for integer list encoding.
- `SimdOps::{intersect_sorted,union_sorted,score_documents,dot_product}`.
- `ParallelIndexer::{index_batch,build_inverted_index,get_progress}`.
- `MmapFile::open_read_only` and `PageCache::get_page` are exposed for mmap-based page access.

### Dependencies

**Internal modules:**
- `src/core/engine.rs` — low-memory hooks in write path.
- `src/storage/segment_writer.rs` / `src/index/posting.rs` — compression/encoding consumers.
- `src/index/inverted.rs` — SIMD set operation consumers.

**External services/libraries:**
- `rayon` — parallel indexing workers.
- `lz4`, `zstd`, `snap` — compression backends.
- `memmap2` — mmap-based file access.
- `tempfile` — swap directory management.

### Failure Modes and Edge Cases

- Engine write path now enforces `MemoryTracker::allocate` failures and returns deterministic `OutOfMemory` errors.
- Engine reclaim threshold check is hardcoded (`pressure > 0.8`) in addition to low-memory config threshold.
- `Config.compression` now controls segment and index block compression for newly written artifacts.
- `SwapManager` compression/decompression framing logic is partial and `swap_cold_data` is placeholder.
- `mmap` and `parallel::merger` modules are public but not wired into main read/write engine flow.
- `LazyIndexReader` currently deserializes full index upfront, reducing practical laziness.

### Observability and Debugging

- `SearchIndex::get_memory_pressure` exposes current pressure ratio.
- `ParallelIndexer::get_progress` tracks processed doc count in current batch.
- Benchmarks in `benches/database_benchmark.rs` and `benches/index_loading_benchmark.rs` provide empirical performance probes.
- No centralized metrics stream for pool utilization, swap IO, or reclaim events.

### Risks and Notes

- Performance modules are heterogeneous maturity levels: some are active in hot paths (buffer pool, compression, parallel indexing), others are scaffolding for future integration.
- Naming may imply full low-memory guarantees; core write-path allocation limits are now enforced, but broader reclaim/swap behavior remains partially implemented.

Changes:
