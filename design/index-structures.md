## Inverted Index And Index Readers

### Purpose

Store term-to-document mappings and provide multiple index loading/lookup strategies for search.

### Scope

**In scope:**
- In-memory inverted index data structures (`InvertedIndex`, `TermDictionary`, `PostingList`).
- Prefix/wildcard/fuzzy term expansion helpers.
- Eager/lazy/hybrid on-disk index readers and caches.

**Out of scope:**
- Query parsing and scoring policy decisions.
- Segment/WAL write transaction semantics.
- Reader pool snapshot management.

### Primary User Flow

1. Write path indexes document text into term postings.
2. Search path consults postings by term and may expand candidates via prefix/wildcard/fuzzy helpers.
3. On-disk `.idx` artifacts can be loaded eagerly or lazily by benchmark/operator tooling.

### System Flow

1. Write path calls `InvertedIndex::add_document` (`src/index/inverted.rs`) to group tokens and update postings/dictionary/skip lists.
2. `PostingList::new` compresses doc IDs and positions using integer encoding (`Delta`/`VByte`).
3. For prefix support, `InvertedIndex::build_prefix_index` builds `PrefixIndex` (`fst` map).
4. Search helpers call `search_term`, `prefix_search`, `wildcard_search`, or `fuzzy_search`.
5. Persistent index readers (`IndexReader`, `LazyIndexReader`, `HybridIndexReader`) deserialize `.idx` files for lookup.

### Data Model

- `Term(Vec<u8>)` with UTF-8 conversion via `as_str() -> Result<&str>`.
- `InvertedIndex` fields: `dictionary`, `postings`, `skip_lists`, `doc_count`, `total_tokens`, `prefix_index`.
- `TermInfo` fields: `doc_freq`, `total_freq`, `idf`, `posting_offset`, `posting_size`.
- `Posting` fields: `doc_id`, `term_freq`, `positions`, `field_norm`.
- `PostingList` fields: `doc_ids (EncodedIntegerBlock)`, `term_freqs`, `positions (Vec<EncodedIntegerBlock>)`.
- Persistence rule: in-memory structures are rebuilt/updated at runtime; segment writer serializes compressed per-segment inverted index data to `.idx` files.

### Interfaces and Contracts

- `InvertedIndex::add_document(doc_id, tokens) -> Result<()>` updates postings and stats.
- `InvertedIndex::search_term(term) -> Option<&PostingList>` point lookup.
- `InvertedIndex::intersect_terms(terms) -> Result<Vec<DocId>>` and `union_terms(terms)` use `SimdOps` utilities.
- `InvertedIndex::wildcard_search(pattern) -> Result<Vec<String>>` regex-based term expansion.
- `InvertedIndex::fuzzy_search(term, max_distance, prefix_length) -> Result<Vec<(String, u8)>>` Levenshtein-based expansion.
- `IndexReader::open(storage, segment_id) -> Result<IndexReader>` eager load.
- `LazyIndexReader::open(storage, segment_id, cache_size)` and `get_postings(term)` expose lazy-style API.
- `HybridIndexReader::open(..., LoadingStrategy)` auto-selects eager/lazy mode.

### Dependencies

**Internal modules:**
- `src/index/posting.rs` — compressed posting representation.
- `src/compression/compress.rs` — integer encoding wrappers.
- `src/search/prefix.rs` — FST prefix index implementation.
- `src/simd/operation.rs` — union/intersection primitives.

**External services/libraries:**
- `fst` — prefix index data structure.
- `regex` — wildcard pattern matching.

### Failure Modes and Edge Cases

- `Term::as_str` fails with parse error for non-UTF8 term bytes.
- `prefix_search` returns `InvalidState` if prefix index has not been built.
- `wildcard_search` returns `InvalidInput` on invalid regex pattern compilation.
- `LazyIndexReader` currently re-reads/deserializes full index content for term loads; lazy behavior is partial.
- `HybridIndexReader::Adaptive` threshold is file-size based only (50MB), not workload aware.

### Observability and Debugging

- `InvertedIndex::stats()` reports `doc_count`, `total_tokens`, `unique_terms`, `avg_doc_length`.
- `LazyIndexReader::cache_stats()` exposes hit/miss/hit_rate for posting cache.
- `HybridIndexCache::stats()` reports eager vs lazy segment counts and average lazy hit rate.
- No built-in integrity checks for dictionary/posting consistency beyond deserialization errors.

### Risks and Notes

- Index reader modules are feature-rich, but primary runtime search path currently uses `ReaderPool` + `SegmentSearch` over segment docs rather than directly relying on eager/lazy/hybrid reader stack.
- Some `.idx` reader capabilities are mainly benchmark-facing today.

Changes:

