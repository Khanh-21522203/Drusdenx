## Query Language And Planning

### Purpose

Convert user query strings into typed query structures, validate them, and optimize/logically plan execution.

### Scope

**In scope:**
- Query AST types and visitor-based dispatch.
- String parser and supported syntax.
- Structural validation rules.
- Logical planner and optimizer rules.
- Document-level query matching semantics.

**Out of scope:**
- Segment iteration and result collection mechanics.
- Score calculation algorithms.
- WAL/storage side effects.

### Primary User Flow

1. Caller submits a query string such as `"title:rust"` or `"foo AND bar"`.
2. Parser builds a `Query` AST.
3. Validator checks depth/clause constraints.
4. Planner maps AST to a `LogicalPlan`.
5. Optimizer applies rewrite rules before execution layer converts plan back to query form.

### System Flow

1. Entry point: `src/query/parser.rs:QueryParser::parse`.
2. Parser emits `Query::{Term,Phrase,Bool,Range,Prefix,Wildcard,Fuzzy,MatchAll}` based on token patterns; field expressions like `title:pre*` now map to `PrefixQuery`.
3. Validator (`src/query/types.rs:QueryValidator`) enforces max depth and max bool clause limits.
4. Planner (`src/query/planner.rs`) maps query variants into `LogicalPlan` nodes.
5. Optimizer (`src/query/optimizer.rs`) applies rule passes (`FilterPushdownRule`, `LimitMergeRule`) only when query classes can roundtrip safely through planner/plan-to-query conversion.
6. Matcher (`src/query/matcher.rs:DocumentMatcher`) evaluates AST against deserialized documents.

```
Query string
  └── QueryParser::parse
        ├── [invalid range syntax] -> ErrorKind::Parse
        └── Query AST
              └── QueryValidator::validate
                    ├── [too deep/too many clauses] -> ErrorKind::InvalidInput
                    └── QueryPlanner::plan
                          └── QueryOptimizer::optimize
```

### Data Model

- `Query` enum variants: `Term`, `Phrase`, `Bool`, `Range`, `Prefix`, `Wildcard`, `Fuzzy`, `MatchAll`.
- `BoolQuery` fields: `must`, `should`, `must_not`, `filter`, `minimum_should_match`, `boost`.
- `RangeQuery` fields: `gt`, `gte`, `lt`, `lte` over `FieldValue`.
- `ValidationConfig` fields: `max_bool_clauses`, `max_query_depth`, `max_wildcard_terms`, `allow_leading_wildcard`.
- `LogicalPlan` variants: `Scan`, `IndexSeek`, `Filter`, `Sort`, `Limit`, `Union`, `Intersection`, `Difference`.
- Persistence rule: query objects and logical plans are transient in-memory structures only.

### Interfaces and Contracts

- `QueryParser::parse(input) -> Result<Query>` supports phrase, boolean keywords (`AND`/`OR`/`NOT`), field syntax, range syntax, fuzzy (`~`), wildcard (`*`/`?`), and field prefix patterns (`field:pre*`) via `PrefixQuery`.
- `QueryValidator::validate(query) -> Result<()>` enforces structural constraints.
- `QueryPlanner::plan(query) -> LogicalPlan` returns a scan fallback when visitor evaluation errors.
- `QueryOptimizer::optimize(plan) -> LogicalPlan` applies rewrite rules once in order; execution-side optimization skips unsupported query classes and preserves the original AST when roundtrip conversion is unsafe.
- `DocumentMatcher::matches(doc, query) -> Result<bool>` evaluates AST over document fields and postings.

### Dependencies

**Internal modules:**
- `src/query/ast.rs` and `src/query/visitor.rs` — core model and visitor trait.
- `src/query/types.rs` — validation config/statistics/cost model.
- `src/index/inverted.rs` — term/posting lookups for phrase matching and term-level helpers.
- `src/core/utils.rs` — Levenshtein distance helper for fuzzy match fallback.

**External services/libraries:**
- `regex` — wildcard pattern matching against text values.
- `chrono` — RFC3339 parsing for date-like range values.

### Failure Modes and Edge Cases

- Invalid range clauses return `ErrorKind::Parse`.
- Excessive query depth or bool clauses return `ErrorKind::InvalidInput`.
- Boolean parser populates `must_not` for `NOT` clauses; evaluation excludes matched `must_not` documents.
- Non-term variants (phrase/range/prefix/wildcard/fuzzy) bypass unsafe optimize-roundtrip conversion and execute with original semantics.
- Date range matching in `DocumentMatcher` is placeholder behavior (`Ok(true)` for date fields).

### Observability and Debugging

- Start from `QueryParser::parse` for syntax interpretation issues.
- Inspect `QueryValidator::validate_depth_recursive` and `visit_bool` for clause/depth rejections.
- Inspect `QueryPlanner::visit_*` methods to understand why queries become scans.
- No query-plan logging is emitted by default.

### Risks and Notes

- Query language behavior is partially implemented; AST supports more variants than parser/planner handle precisely.
- `src/query/validator.rs` exists as an empty module file while actual validator implementation is in `src/query/types.rs`, which can mislead code navigation.

Changes:
