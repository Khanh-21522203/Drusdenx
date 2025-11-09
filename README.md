# Drusdenx

A high-performance, full-text search engine and database written in Rust.

## 📚 Overview

Drusdenx is a modern search engine implementation that combines the power of inverted indexes, SIMD optimizations, and MVCC (Multi-Version Concurrency Control) to deliver fast and reliable full-text search capabilities. Built entirely in Rust, it focuses on memory safety and performance.

### Key Design Goals

- **Performance**: Sub-microsecond search operations with SIMD optimizations
- **Concurrency**: MVCC-based architecture for lock-free reads
- **Reliability**: WAL (Write-Ahead Logging) for crash recovery
- **Modern**: Written in idiomatic Rust with zero-cost abstractions

## ✨ Features

### Core Functionality

- **Full-Text Search**
  - Inverted index with posting lists
  - Boolean queries (AND, OR, NOT)
  - Field-specific searches
  - Prefix and wildcard matching
  - BM25 scoring algorithm

- **CRUD Operations**
  - Document insertion and updates
  - Soft deletes with RoaringBitmap
  - Batch operations
  - Database compaction

- **ACID Transactions**
  - Serializable isolation level
  - Optimistic concurrency control
  - Rollback support
  - WAL-based durability

- **Advanced Features**
  - SIMD-optimized set operations
  - Query result caching
  - Reader pool for connection reuse
  - Segment merging policies (Tiered, Log-Structured)
  - Real-time statistics and monitoring
  - Health check system


## 🚀 Getting Started

### Prerequisites

- Rust 1.70 or higher
- Cargo

### Installation

Add Drusdenx to your `Cargo.toml`:

```toml
[dependencies]
Drusdenx = { path = "path/to/Drusdenx" }
```

### Basic Usage

```rust
use Drusdenx::core::database::Database;
use Drusdenx::core::config::Config;
use Drusdenx::core::types::{Document, DocId, FieldValue};
use Drusdenx::schema::schema::SchemaWithAnalyzer;
use std::collections::HashMap;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Create database
    let config = Config::default();
    let schema = SchemaWithAnalyzer::new();
    let db = Database::open_with_schema(schema, config)?;

    // 2. Insert documents
    let doc = Document {
        id: DocId(1),
        fields: HashMap::from([
            ("title".to_string(), FieldValue::Text("Hello World".to_string())),
            ("content".to_string(), FieldValue::Text("Learning Rust".to_string())),
        ]),
    };
    db.add_document(doc)?;
    db.flush()?;
    db.commit()?;

    // 3. Search
    let results = db.search("rust")?;
    println!("Found {} results", results.len());

    // 4. Statistics
    let stats = db.stats()?;
    println!("Total documents: {}", stats.total_documents);

    Ok(())
}
```

### Configuration

The database can be configured through the `Config` struct:

```rust
use Drusdenx::core::config::Config;

let config = Config {
    data_dir: "./data".to_string(),
    max_memory_mb: 1024,
    enable_wal: true,
    cache_size: 10000,
    ..Default::default()
};
```

### Running Examples

The project includes several examples demonstrating different features:

```bash
# Basic API demo (all operations)
cargo run --example simple_usage

# Monitoring and health checks
cargo run --example monitoring_example

# Transaction and SIMD operations
cargo run --example transaction_simd_example
```

## 📊 Benchmark Results

Performance benchmarks using Criterion.rs on a standard Linux system:

### Write Performance

| Operation | Throughput | Latency |
|-----------|------------|---------|
| Single Document Insert | **104,000 docs/sec** | 9.6 µs |
| Batch Insert (100) | **61,000 docs/sec** | 1.64 ms |
| Batch Insert (1000) | **88,000 docs/sec** | 11.37 ms |
| **Peak Throughput** | **100,600 docs/sec** | - |

### Search Performance

| Query Type | Throughput | Latency |
|------------|------------|---------|
| Simple Term | **24.4M queries/sec** | 40.9 ns |
| Boolean AND | **25.7M queries/sec** | 38.9 ns |
| Boolean OR | **29.8M queries/sec** | 33.5 ns |
| Complex Boolean | **24.6M queries/sec** | 40.7 ns |
| Prefix Search | **28.7M queries/sec** | 34.9 ns |
| Field-Specific | **30.2M queries/sec** | 33.1 ns |

### SIMD Operations (10K elements)

| Operation | Throughput | Latency |
|-----------|------------|---------|
| Intersection | **87K ops/sec** | 11.46 µs |
| Union | **159K ops/sec** | 6.29 µs |


> **Note**: These benchmarks represent in-memory performance with small datasets. Real-world performance may vary based on data size, query complexity, and hardware.

### Running Benchmarks

```bash
# Run all benchmarks
cargo bench --bench database_benchmark

# Run specific benchmark group
cargo bench --bench database_benchmark -- search

# View results
./display_database_benchmark.sh
```

Detailed benchmark reports are available in:
- `benchmark_results/database_benchmark_report.md` - Full analysis
- `target/criterion/report/index.html` - Interactive HTML report

## 🏗️ Architecture

### High-Level Overview

```
┌─────────────────────────────────────────────────────────┐
│                     Database API                         │
├─────────────────────────────────────────────────────────┤
│  IndexWriter  │  ReaderPool  │  QueryExecutor  │  MVCC  │
├─────────────────────────────────────────────────────────┤
│           Inverted Index + Posting Lists                 │
├─────────────────────────────────────────────────────────┤
│  Scoring (BM25)  │  Text Analysis  │  SIMD Operations   │
├─────────────────────────────────────────────────────────┤
│          Storage Layer (Segments + WAL)                  │
├─────────────────────────────────────────────────────────┤
│  Memory Pool  │  Buffer Management  │  Compression      │
└─────────────────────────────────────────────────────────┘
```

### Key Components

1. **IndexWriter**: Handles document indexing and segment creation
2. **ReaderPool**: Manages index readers with version-based caching
3. **MVCCController**: Provides snapshot isolation for concurrent reads
4. **QueryExecutor**: Executes search queries and applies scoring
5. **InvertedIndex**: Core data structure with posting lists
6. **WAL**: Write-ahead log for durability and crash recovery
7. **SegmentWriter/Reader**: Segment-based storage management

## 📖 Documentation

### API Reference

The main API is provided through the `Database` struct:

```rust
// Document operations
db.add_document(doc)?;
db.delete_document(doc_id)?;
db.search(query)?;

// Batch operations
db.flush()?;
db.commit()?;
db.compact()?;

// Transactions
db.with_transaction(isolation_level, |tx| {
    tx.insert(doc)?;
    Ok(())
})?;

// Monitoring
let stats = db.stats()?;
let health = db.health_check()?;
```

### Project Structure

```
src/
├── analysis/        # Text analysis (tokenization, filtering)
├── core/           # Core types and database implementation
├── index/          # Inverted index and posting lists
├── query/          # Query parsing and execution
├── search/         # Search algorithms and scoring
├── storage/        # Segment storage and WAL
├── mvcc/           # Multi-version concurrency control
├── writer/         # Index writer and segment management
├── reader/         # Index readers and query execution
├── memory/         # Memory management and pools
├── simd/           # SIMD-optimized operations
├── compression/    # Data compression utilities
└── parallel/       # Parallel processing utilities

examples/          # Example programs
benches/           # Performance benchmarks
```

## 🧪 Testing

Run the test suite:

```bash
# All tests
cargo test

# Specific test module
cargo test --lib core

# With output
cargo test -- --nocapture

# Examples
cargo run --example simple_usage

## 🛠️ Development

### Building

```bash
# Debug build
cargo build

# Release build (optimized)
cargo build --release

# With specific features
cargo build --features "simd,compression"
```

## 🤝 Contributing

This is an educational project, but contributions are welcome! Please note:

1. The project is for learning purposes
2. Code should be well-documented
3. Add tests for new features
4. Run benchmarks for performance-critical changes
5. Follow Rust best practices

## 📄 License

This project is available under the MIT License. See LICENSE file for details.

## 📧 Contact

This is a learning project. For questions or discussions:
- Open an issue on GitHub
- Check existing examples and documentation

---

**Remember**: This is an **educational project** for learning search engine internals. Use at your own risk and do not deploy to production without extensive testing and security review.
