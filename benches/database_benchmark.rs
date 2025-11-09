use criterion::{black_box, criterion_group, criterion_main, Criterion, BenchmarkId};
use Drusdenx::core::database::Database;
use Drusdenx::core::config::Config;
use Drusdenx::core::types::{Document, DocId, FieldValue};
use Drusdenx::schema::schema::SchemaWithAnalyzer;
use Drusdenx::mvcc::controller::IsolationLevel;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use std::thread;
use rand::Rng;

/// Helper to create test documents
fn create_test_document(id: u64, content_size: usize) -> Document {
    let mut rng = rand::thread_rng();
    let content: String = (0..content_size)
        .map(|_| {
            let words = ["the", "quick", "brown", "fox", "jumps", "over", "lazy", "dog"];
            words[rng.gen_range(0..words.len())]
        })
        .collect::<Vec<_>>()
        .join(" ");
    
    Document {
        id: DocId(id),
        fields: HashMap::from([
            ("title".to_string(), FieldValue::Text(format!("Document {}", id))),
            ("content".to_string(), FieldValue::Text(content)),
            ("category".to_string(), FieldValue::Text(format!("category_{}", id % 10))),
            ("score".to_string(), FieldValue::Number(rng.gen_range(0.0..100.0))),
        ]),
    }
}

/// Benchmark single document insertion
fn bench_single_insert(c: &mut Criterion) {
    let config = Config::default();
    let schema = SchemaWithAnalyzer::new();
    let db = Arc::new(Database::open_with_schema(schema, config).unwrap());
    
    c.bench_function("single_document_insert", |b| {
        let mut id = 0;
        b.iter(|| {
            let doc = create_test_document(id, 100);
            db.add_document(doc).unwrap();
            id += 1;
        });
    });
}

/// Benchmark batch insertion
fn bench_batch_insert(c: &mut Criterion) {
    let mut group = c.benchmark_group("batch_insert");
    
    for batch_size in [10, 50, 100, 500, 1000].iter() {
        group.bench_with_input(
            BenchmarkId::from_parameter(batch_size),
            batch_size,
            |b, &batch_size| {
                let config = Config::default();
                let schema = SchemaWithAnalyzer::new();
                let db = Arc::new(Database::open_with_schema(schema, config).unwrap());
                let mut id_counter = 0u64;
                
                b.iter(|| {
                    let docs: Vec<Document> = (0..batch_size)
                        .map(|_| {
                            let doc = create_test_document(id_counter, 100);
                            id_counter += 1;
                            doc
                        })
                        .collect();
                    
                    for doc in docs {
                        db.add_document(doc).unwrap();
                    }
                    db.flush().unwrap();
                });
            },
        );
    }
    group.finish();
}

/// Benchmark search performance
fn bench_search(c: &mut Criterion) {
    // Setup database with test data
    let config = Config::default();
    let schema = SchemaWithAnalyzer::new();
    let db = Arc::new(Database::open_with_schema(schema, config).unwrap());
    
    // Insert test documents with known content
    for i in 0..1000 {
        let doc = create_test_document(i, 100);
        db.add_document(doc).unwrap();
    }
    // Ensure data is committed and searchable
    db.flush().unwrap();
    db.commit().unwrap();
    
    // Wait a bit to ensure index is ready
    thread::sleep(Duration::from_millis(100));
    
    let mut group = c.benchmark_group("search");
    
    // Simple term search - search for words we know exist
    group.bench_function("simple_term_search", |b| {
        b.iter(|| {
            // Search for "content:" field which we know exists
            let _ = db.search(black_box("content:fox")).unwrap_or_default();
        });
    });
    
    // Boolean AND search
    group.bench_function("boolean_and_search", |b| {
        b.iter(|| {
            let _ = db.search(black_box("content:quick AND content:brown")).unwrap_or_default();
        });
    });
    
    // Boolean OR search
    group.bench_function("boolean_or_search", |b| {
        b.iter(|| {
            let _ = db.search(black_box("content:fox OR content:dog")).unwrap_or_default();
        });
    });
    
    // Complex boolean search
    group.bench_function("complex_boolean_search", |b| {
        b.iter(|| {
            let _ = db.search(black_box("(content:quick AND content:brown) OR (content:lazy AND content:dog)")).unwrap_or_default();
        });
    });
    
    // Prefix search for document titles
    group.bench_function("prefix_search", |b| {
        b.iter(|| {
            let _ = db.search(black_box("title:Document*")).unwrap_or_default();
        });
    });
    
    // Category search
    group.bench_function("category_search", |b| {
        b.iter(|| {
            let _ = db.search(black_box("category:category_5")).unwrap_or_default();
        });
    });
    
    // Wildcard search - find documents with any title starting with "Doc" and ending with number
    group.bench_function("wildcard_search", |b| {
        b.iter(|| {
            let _ = db.search(black_box("title:Doc*ment*")).unwrap_or_default();
        });
    });
    
    // Fuzzy search - find similar words with edit distance 1
    group.bench_function("fuzzy_search_distance_1", |b| {
        b.iter(|| {
            let _ = db.search(black_box("content:quik~1")).unwrap_or_default(); // Should match "quick"
        });
    });
    
    // Fuzzy search - find similar words with edit distance 2  
    group.bench_function("fuzzy_search_distance_2", |b| {
        b.iter(|| {
            let _ = db.search(black_box("content:brwn~2")).unwrap_or_default(); // Should match "brown"
        });
    });
    
    // Range query - find documents with score between 25 and 75
    group.bench_function("range_query_numeric", |b| {
        b.iter(|| {
            let _ = db.search(black_box("score:[25.0 TO 75.0]")).unwrap_or_default();
        });
    });
    
    // Phrase query - find exact phrase "quick brown fox"
    group.bench_function("phrase_query_exact", |b| {
        b.iter(|| {
            let _ = db.search(black_box("content:\"quick brown fox\"")).unwrap_or_default();
        });
    });
    
    // Phrase query with common words - "the quick"
    group.bench_function("phrase_query_common", |b| {
        b.iter(|| {
            let _ = db.search(black_box("content:\"the quick\"")).unwrap_or_default();
        });
    });
    
    group.finish();
}

/// Benchmark SIMD operations
fn bench_simd_operations(c: &mut Criterion) {
    use Drusdenx::simd::operation::SimdOps;
    
    let mut group = c.benchmark_group("simd_operations");
    
    for size in [100, 1000, 10000, 100000].iter() {
        // Generate sorted arrays
        let array1: Vec<u32> = (0..*size).step_by(2).collect();
        let array2: Vec<u32> = (0..*size).step_by(3).collect();
        
        // Benchmark intersection
        group.bench_with_input(
            BenchmarkId::new("intersection", size),
            &(array1.clone(), array2.clone()),
            |b, (a1, a2)| {
                b.iter(|| {
                    SimdOps::intersect_sorted(black_box(a1), black_box(a2))
                });
            },
        );
        
        // Benchmark union
        group.bench_with_input(
            BenchmarkId::new("union", size),
            &(array1.clone(), array2.clone()),
            |b, (a1, a2)| {
                b.iter(|| {
                    SimdOps::union_sorted(black_box(a1), black_box(a2))
                });
            },
        );
    }
    
    group.finish();
}

/// Benchmark transaction operations
fn bench_transactions(c: &mut Criterion) {
    let mut group = c.benchmark_group("transactions");
    
    // Benchmark transaction commit
    group.bench_function("transaction_commit", |b| {
        let config = Config::default();
        let schema = SchemaWithAnalyzer::new();
        let db = Arc::new(Database::open_with_schema(schema, config).unwrap());
        let mut id = 0;
        
        b.iter(|| {
            db.with_transaction(IsolationLevel::ReadCommitted, |tx| {
                for _ in 0..10 {
                    let doc = create_test_document(id, 100);
                    tx.insert(doc)?;
                    id += 1;
                }
                Ok(())
            }).unwrap();
        });
    });
    
    // Benchmark transaction rollback
    group.bench_function("transaction_rollback", |b| {
        let config = Config::default();
        let schema = SchemaWithAnalyzer::new();
        let db = Arc::new(Database::open_with_schema(schema, config).unwrap());
        let mut id = 0;
        
        b.iter(|| {
            let _: Result<(), _> = db.with_transaction(IsolationLevel::Serializable, |tx| {
                for _ in 0..10 {
                    let doc = create_test_document(id, 100);
                    tx.insert(doc)?;
                    id += 1;
                }
                // Force rollback
                Err(Drusdenx::core::error::Error::new(
                    Drusdenx::core::error::ErrorKind::InvalidState,
                    "Forced rollback".to_string(),
                ))
            });
        });
    });
    
    group.finish();
}

/// Benchmark concurrent operations (simplified without threads)
fn bench_concurrent_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("concurrent");
    
    // Simulated concurrent reads (sequential for now)
    group.bench_function("multiple_reads", |b| {
        let config = Config::default();
        let schema = SchemaWithAnalyzer::new();
        let db = Arc::new(Database::open_with_schema(schema, config).unwrap());
        
        // Insert test data
        for i in 0..1000 {
            let doc = create_test_document(i, 100);
            db.add_document(doc).unwrap();
        }
        db.flush().unwrap();
        
        b.iter(|| {
            // Simulate concurrent reads sequentially
            for _ in 0..4 {
                for _ in 0..10 {
                    let _ = db.search("fox").unwrap();
                }
            }
        });
    });
    
    group.finish();
}

/// Main throughput benchmark
fn bench_throughput(c: &mut Criterion) {
    let mut group = c.benchmark_group("throughput");
    group.sample_size(10); // Reduce sample size for long-running benchmarks
    group.measurement_time(Duration::from_secs(10)); // Reduce to 10s for faster results
    
    // Index throughput (documents per second)
    group.bench_function("index_throughput", |b| {
        b.iter_custom(|iters| {
            let config = Config::default();
            let schema = SchemaWithAnalyzer::new();
            let db = Arc::new(Database::open_with_schema(schema, config).unwrap());
            let mut id = 0;
            
            let start = Instant::now();
            for _ in 0..iters {
                for _ in 0..100 {
                    let doc = create_test_document(id, 100);
                    db.add_document(doc).unwrap();
                    id += 1;
                }
                if id % 1000 == 0 {
                    db.flush().unwrap();
                }
            }
            db.flush().unwrap();
            start.elapsed()
        });
    });
    
    // Query throughput (queries per second)
    group.bench_function("query_throughput", |b| {
        let config = Config::default();
        let schema = SchemaWithAnalyzer::new();
        let db = Arc::new(Database::open_with_schema(schema, config).unwrap());
        
        // Insert test data
        for i in 0..5000 {
            let doc = create_test_document(i, 50);
            db.add_document(doc).unwrap();
        }
        db.flush().unwrap();
        db.commit().unwrap();
        
        // Use field-specific queries
        let queries = vec![
            "content:fox",
            "content:quick AND content:brown",
            "content:lazy OR content:dog",
            "category:category_5",
            "title:Document*",
        ];
        let mut query_idx = 0;
        
        b.iter_custom(|iters| {
            let start = Instant::now();
            for _ in 0..iters {
                for _ in 0..100 {
                    let _ = db.search(queries[query_idx % queries.len()]).unwrap_or_default();
                    query_idx += 1;
                }
            }
            start.elapsed()
        });
    });
    
    group.finish();
}

criterion_group!(
    benches,
    bench_single_insert,
    bench_batch_insert,
    bench_search,
    bench_simd_operations,
    bench_transactions,
    bench_concurrent_operations,
    bench_throughput
);
criterion_main!(benches);
