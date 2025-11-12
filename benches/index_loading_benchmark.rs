use criterion::{black_box, criterion_group, criterion_main, Criterion, BenchmarkId};
use Drusdenx::core::types::{Document, DocId, FieldValue};
use Drusdenx::storage::layout::StorageLayout;
use Drusdenx::storage::segment::SegmentId;
use Drusdenx::index::index_reader::IndexReader;
use Drusdenx::index::lazy_index_reader::LazyIndexReader;
use Drusdenx::index::hybrid_index_reader::{HybridIndexReader, LoadingStrategy};
use Drusdenx::index::inverted::Term;
use std::sync::Arc;
use std::path::PathBuf;
use std::collections::HashMap;

// Helper to create a valid test segment with index
fn create_test_segment(storage: &StorageLayout, doc_count: usize) -> SegmentId {
    use Drusdenx::storage::segment_writer::SegmentWriter;
    use Drusdenx::memory::buffer_pool::BufferPool;
    use Drusdenx::index::posting::Posting;
    
    let buffer_pool = Arc::new(BufferPool::new(100 * 1024 * 1024));
    let segment_id = SegmentId::new();
    let mut writer = SegmentWriter::new(storage, segment_id, buffer_pool.clone()).unwrap();
    
    // Create test documents
    for i in 0..doc_count {
        let mut doc = Document::new(DocId(i as u64));
        doc.fields.insert(
            "title".to_string(),
            FieldValue::Text(format!("Document {} about rust programming search engine", i))
        );
        doc.fields.insert(
            "content".to_string(),
            FieldValue::Text(format!("This is document number {} with various terms like database index query", i))
        );
        writer.write_document(&doc).unwrap();
    }
    
    // Create properly formatted inverted index
    let mut inverted_index = HashMap::new();
    let terms = vec!["rust", "programming", "search", "engine", "database", "index", "query", "document"];
    
    for term_text in &terms {
        let term = Term::new(term_text);
        let mut postings = Vec::new();
        
        // Create realistic postings
        let posting_count = doc_count.min(50);
        for doc_id in 0..posting_count {
            postings.push(Posting {
                doc_id: DocId(doc_id as u64),
                term_freq: (doc_id % 5 + 1) as u32,
                positions: vec![5, 10, 15],
                field_norm: 0.5,
            });
        }
        
        inverted_index.insert(term, postings);
    }
    
    // Set index and finish (this will write .idx file with proper format)
    writer.inverted_index = inverted_index;
    writer.finish(storage).unwrap();
    
    segment_id
}

fn bench_index_loading(c: &mut Criterion) {
    let temp_dir = tempfile::tempdir().unwrap();
    let storage = StorageLayout::new(PathBuf::from(temp_dir.path())).unwrap();
    
    // Create test segments with different sizes
    let segment_small = create_test_segment(&storage, 100);  // Small segment
    let segment_medium = create_test_segment(&storage, 1000);  // Medium segment
    
    let mut group = c.benchmark_group("index_loading");
    
    // Benchmark: Eager loading (load all)
    group.bench_with_input(
        BenchmarkId::new("eager_load", "small_100_docs"),
        &segment_small,
        |b, &segment_id| {
            b.iter(|| {
                let reader = IndexReader::open(&storage, segment_id).unwrap();
                black_box(reader);
            });
        },
    );
    
    group.bench_with_input(
        BenchmarkId::new("eager_load", "medium_1000_docs"),
        &segment_medium,
        |b, &segment_id| {
            b.iter(|| {
                let reader = IndexReader::open(&storage, segment_id).unwrap();
                black_box(reader);
            });
        },
    );
    
    // Benchmark: Lazy loading (load on-demand)
    group.bench_with_input(
        BenchmarkId::new("lazy_load", "small_100_docs"),
        &segment_small,
        |b, &segment_id| {
            b.iter(|| {
                let reader = LazyIndexReader::open(&storage, segment_id, 1000).unwrap();
                black_box(reader);
            });
        },
    );
    
    group.bench_with_input(
        BenchmarkId::new("lazy_load", "medium_1000_docs"),
        &segment_medium,
        |b, &segment_id| {
            b.iter(|| {
                let reader = LazyIndexReader::open(&storage, segment_id, 1000).unwrap();
                black_box(reader);
            });
        },
    );
    
    // Benchmark: Hybrid adaptive
    group.bench_with_input(
        BenchmarkId::new("hybrid_adaptive", "small_100_docs"),
        &segment_small,
        |b, &segment_id| {
            b.iter(|| {
                let reader = HybridIndexReader::open(&storage, segment_id, LoadingStrategy::Adaptive).unwrap();
                black_box(reader);
            });
        },
    );
    
    group.finish();
}

fn bench_term_lookup(c: &mut Criterion) {
    let temp_dir = tempfile::tempdir().unwrap();
    let storage = StorageLayout::new(PathBuf::from(temp_dir.path())).unwrap();
    let segment_id = create_test_segment(&storage, 1000);
    
    let term_rust = Term::new("rust");
    
    let mut group = c.benchmark_group("term_lookup");
    
    // Eager: First lookup
    let eager_reader = IndexReader::open(&storage, segment_id).unwrap();
    group.bench_function("eager_lookup", |b| {
        b.iter(|| {
            let postings = eager_reader.get_postings(&term_rust);
            black_box(postings);
        });
    });
    
    // Lazy: First lookup (cold)
    group.bench_function("lazy_lookup_cold", |b| {
        b.iter(|| {
            let lazy_reader = LazyIndexReader::open(&storage, segment_id, 1000).unwrap();
            let postings = lazy_reader.get_postings(&term_rust).unwrap();
            black_box(postings);
        });
    });
    
    // Lazy: Warm lookup (from cache)
    let lazy_reader = LazyIndexReader::open(&storage, segment_id, 1000).unwrap();
    let _ = lazy_reader.get_postings(&term_rust);  // Warm cache
    group.bench_function("lazy_lookup_warm", |b| {
        b.iter(|| {
            let postings = lazy_reader.get_postings(&term_rust).unwrap();
            black_box(postings);
        });
    });
    
    group.finish();
}

criterion_group!(benches, bench_index_loading, bench_term_lookup);
criterion_main!(benches);
