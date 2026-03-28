## Search Execution And Scoring

### Purpose

Execute parsed queries over snapshot readers, collect top-k results, and compute relevance scores.

### Scope

**In scope:**
- `QueryExecutor` execution pipeline.
- Execution configuration modes (default/simple/debug/scoring variants).
- Collector/result models (`TopKCollector`, `SearchResults`).
- Scoring abstraction and BM25/TF-IDF implementations.
- Generic `SearchPipeline` builder APIs.

**Out of scope:**
- Query parsing and logical planning internals.
- WAL/segment write lifecycle.
- Transaction coordination.

### Primary User Flow

1. Caller invokes `search`, `search_n`, or `search_debug` on `SearchIndex`.
2. Engine resolves or populates query cache for `(query, limit, offset)`.
3. Executor validates and optionally optimizes query.
4. Executor scans segment readers, filters deleted docs, scores matches, and returns top-k hits.
5. Caller receives `Vec<ScoredDocument>` or full `SearchResults` (debug path).

### System Flow

1. Entry point: `src/core/engine.rs:run_search`.
2. Cache lookup via `src/query/cache.rs:QueryCache::get_by_str`.
3. Query parse via `QueryParser::parse`.
4. Snapshot reader acquisition via `ReaderPool::get_reader`.
5. `QueryExecutor::execute` (`src/search/executor.rs`) runs: validate -> safe-optimize (or preserve original query when roundtrip is unsafe) -> segment iteration -> collect results.
6. Scoring path calls `BM25Scorer`/`TfIdfScorer` through `Scorer` trait.
7. Results are cached back with `QueryCache::put_by_str`.

```
SearchIndex::search_n
  └── SearchEngine::run_search
        ├── QueryCache hit -> return cached SearchResults
        └── parse + get_reader + QueryExecutor::execute
              └── TopKCollector -> SearchResults { hits, total_hits, max_score, took_ms }
```

### Data Model

- `ExecutionConfig` fields: `scoring`, `enable_optimization`, `enable_validation`, `collect_explanations`, `timeout_ms`.
- `SearchResults` fields: `hits (Vec<ScoredDocument>)`, `total_hits (usize)`, `max_score (f32)`, `took_ms (u64)`.
- `ScoredDocument` fields: `doc_id`, `score`, `document (Option<Document>)`, `explanation (Option<ScoreExplanation>)`.
- `TopKCollector` fields: `heap`, `k`, `min_score`, `total_collected`.
- `ScoringContext` fields: `doc_id`, `posting`, `term_info`, `doc_stats`, `query_boost`.
- Persistence rule: executor data is transient; cache stores cloned `SearchResults` in memory LRU.

### Interfaces and Contracts

- `QueryExecutor::execute(reader, query, limit, config) -> Result<SearchResults>`.
- `QueryExecutor::execute_simple(reader, query, limit) -> Result<SearchResults>`.
- `ExecutionConfig::default/simple/debug/bm25/tfidf` convenience constructors.
- `Scorer` trait contract: `score_ctx`, optional `score_batch`, `name`.
- `SearchPipeline::run(reader, query) -> Result<C::Output>` consumes pipeline and returns collector output.
- `SearchPipeline::execute(&mut self, ..)` is now an explicit invalid-state error path; callers must use consuming `run(self, ..)`.
- `QueryCache::{get_by_str, put_by_str, stats}` provides in-memory cached result access.

### Dependencies

**Internal modules:**
- `src/query/*` — parser/planner/optimizer/validator/matcher dependencies.
- `src/reader/reader_pool.rs` — snapshot-segment reader source.
- `src/scoring/scorer.rs` — score formulas.
- `src/search/results.rs`, `src/search/collector.rs` — collection contracts.

**External services/libraries:**
- `lru` — LRU cache backing for query cache.

### Failure Modes and Edge Cases

- Validation failures bubble up as `ErrorKind::InvalidInput`.
- `SearchPipeline::execute(&mut self)` returns `ErrorKind::InvalidState` instead of panicking, steering callers to `run(self, ...)`.
- `QueryCache::new(0)` would panic via `NonZeroUsize::new(...).unwrap()`.
- `TopKCollector::max_score` uses heap peek with reversed ordering semantics; reported max score can be inconsistent.
- Sorting by `partial_cmp(...).unwrap()` can panic if a NaN score appears.

### Observability and Debugging

- Use `SearchIndex::search_debug` to enable explanation collection (`ExecutionConfig::debug`).
- Inspect `QueryCache::stats` (`hit_count`, `miss_count`, `size`, `capacity`) for cache behavior.
- `SearchResults::took_ms` and database `queries_per_second` help runtime performance triage.
- No built-in tracing of optimization decisions or per-segment timings.

### Risks and Notes

- Executor/pipeline optimization now guards plan-to-query conversion and keeps original queries for unsupported roundtrip classes, preventing broadening to `MatchAll`.
- Timeout field exists in execution config but is not actively enforced in executor loop.

Changes:
