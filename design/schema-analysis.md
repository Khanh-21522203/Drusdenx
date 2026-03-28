## Schema And Text Analysis

### Purpose

Define field metadata and run text normalization/tokenization pipelines used during indexing.

### Scope

**In scope:**
- `SchemaWithAnalyzer` definitions and builder methods.
- Analyzer registry and default analyzers.
- Tokenizer/filter trait contracts and built-in implementations.

**Out of scope:**
- Query parsing/planning/execution logic.
- Segment/WAL persistence.
- MVCC and transaction control.

### Primary User Flow

1. Caller builds a schema using `SchemaWithAnalyzer::new()` and `add_text_field`.
2. Caller opens the database with this schema.
3. Write path obtains configured analyzer and tokenizes text fields into `Token` streams.
4. Tokens become index terms in the writer/indexer path.

### System Flow

1. Entry point: `src/schema/schema.rs:SchemaWithAnalyzer` construction.
2. Engine assembly (`src/core/components.rs`) resolves `schema.default_analyzer` from `AnalyzerRegistry`.
3. Write path (`src/parallel/indexer.rs:index_document`) iterates document text fields and calls `Analyzer::analyze`.
4. `Analyzer` runs tokenizer then each configured `TokenFilter`, returning normalized tokens.

### Data Model

- `SchemaWithAnalyzer` fields: `fields (Vec<FieldDefinitionWithAnalyzer>)`, `default_analyzer (String)`.
- `FieldDefinitionWithAnalyzer` fields: `name (String)`, `field_type (FieldType)`, `indexed (bool)`, `stored (bool)`, `analyzer (Option<String>)`.
- `Analyzer` fields: `tokenizer (Box<dyn Tokenizer>)`, `filters (Vec<Box<dyn TokenFilter>>)`.
- `Token` fields (`src/analysis/token.rs`): `text`, `position (u32)`, `offset (usize)`, `length (usize)`, `token_type (TokenType)`.
- Persistence rule: schema/analyzer config is runtime memory state; tokens are persisted indirectly as index postings.

### Interfaces and Contracts

- `SchemaWithAnalyzer::new() -> SchemaWithAnalyzer` sets default analyzer to `"standard"`.
- `SchemaWithAnalyzer::add_text_field(name, analyzer)` appends field metadata and returns updated schema.
- `SchemaWithAnalyzer::get_analyzer_for_field(field_name) -> Option<&String>` returns field-specific override if present.
- `AnalyzerRegistry::new()` registers `standard` and `vietnamese` analyzers.
- `AnalyzerRegistry::analyze(analyzer_name, text) -> Result<Vec<Token>>` fails with `ErrorKind::NotFound` for unknown analyzer names.
- `Tokenizer` trait contract: `tokenize`, `name`, `clone_box`.
- `TokenFilter` trait contract: transforms `Vec<Token> -> Vec<Token>`.

### Dependencies

**Internal modules:**
- `src/analysis/tokenizer.rs`, `src/analysis/filter.rs` — extension-point traits.
- `src/analysis/filters/*` — lowercase/stopword/stemmer/ngram implementations.
- `src/analysis/language/vietnamese.rs` — Vietnamese tokenizer implementation.
- `src/parallel/indexer.rs` — consumer of analyzer outputs.

**External services/libraries:**
- `unicode-segmentation` — Unicode word boundaries in `StandardTokenizer`.
- `rust-stemmers` — stemming support.

### Failure Modes and Edge Cases

- Unknown analyzer names return `NotFound` from registry lookup.
- `AnalyzerRegistry` uses `std::sync::RwLock` with `unwrap()`; poisoned locks can panic.
- Field-specific analyzers exist in schema but indexing path currently applies one resolved analyzer from engine assembly.
- Token offsets are approximate in `StandardTokenizer` because offsets are advanced by word length only.

### Observability and Debugging

- Debug analyzer selection in `src/core/components.rs` (where default analyzer is resolved).
- Debug token contents by instrumenting `Analyzer::analyze` and `ParallelIndexer::index_document`.
- No built-in metrics for analyzer throughput/error rates.

### Risks and Notes

- Per-field analyzer behavior appears incomplete from current code; schema stores field-level analyzer metadata but indexer path does not branch by field analyzer.
- Schema does not enforce strong field constraints (uniqueness/type-validation checks are not present at ingestion boundary).

Changes:

