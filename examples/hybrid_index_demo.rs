/// Example demonstrating Hybrid Index Loading strategies
/// 
/// This example shows:
/// 1. Eager loading (load all index)
/// 2. Lazy loading with LRU cache
/// 3. Hybrid adaptive strategy

use std::sync::Arc;
use std::path::PathBuf;
use Drusdenx::core::types::{Document, DocId, FieldValue};
use Drusdenx::storage::layout::StorageLayout;
use Drusdenx::storage::segment::SegmentId;
use Drusdenx::storage::segment_writer::SegmentWriter;
use Drusdenx::memory::buffer_pool::BufferPool;
use Drusdenx::index::index_reader::IndexReader;
use Drusdenx::index::lazy_index_reader::LazyIndexReader;
use Drusdenx::index::hybrid_index_reader::{HybridIndexReader, LoadingStrategy};
use Drusdenx::index::inverted::Term;
use Drusdenx::index::posting::Posting;
use std::collections::HashMap;

fn create_test_index(storage: &StorageLayout) -> Result<SegmentId, Box<dyn std::error::Error>> {
    let buffer_pool = Arc::new(BufferPool::new(100 * 1024 * 1024));
    let segment_id = SegmentId::new();
    let mut writer = SegmentWriter::new(storage, segment_id, buffer_pool)?;
    
    // Create test documents
    for i in 0..1000 {
        let mut doc = Document::new(DocId(i));
        doc.fields.insert(
            "title".to_string(),
            FieldValue::Text(format!("Document {} about rust programming", i))
        );
        writer.write_document(&doc)?;
    }
    
    // Create inverted index
    let mut inverted_index = HashMap::new();
    for term_text in &["rust", "programming", "search", "engine", "database"] {
        let term = Term::new(term_text);
        let mut postings = Vec::new();
        for doc_id in 0..100 {
            postings.push(Posting {
                doc_id: DocId(doc_id),
                term_freq: 2,
                positions: vec![5, 10],
                field_norm: 0.5,
            });
        }
        inverted_index.insert(term, postings);
    }
    
    writer.inverted_index = inverted_index;
    writer.finish(storage)?;
    
    Ok(segment_id)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Hybrid Index Loading Demo ===\n");
    
    // Setup
    let temp_dir = tempfile::tempdir()?;
    let storage = Arc::new(StorageLayout::new(PathBuf::from(temp_dir.path()))?);
    
    println!("Creating test index with 1000 documents...");
    let segment_id = create_test_index(&storage)?;
    println!("✅ Index created: {}\n", segment_id.0);
    
    // Example 1: Eager loading (load all)
    println!("Example 1: Eager Loading (Load ALL)");
    println!("-------------------------------------");
    let start = std::time::Instant::now();
    let eager_reader = IndexReader::open(&storage, segment_id)?;
    let eager_load_time = start.elapsed();
    println!("Load time: {:?}", eager_load_time);
    println!("Unique terms: {}", eager_reader.stats().unique_terms);
    println!("Memory: Full index in RAM\n");
    
    // Example 2: Lazy loading
    println!("Example 2: Lazy Loading (On-demand with LRU)");
    println!("----------------------------------------------");
    let start = std::time::Instant::now();
    let lazy_reader = LazyIndexReader::open(&storage, segment_id, 100)?;
    let lazy_load_time = start.elapsed();
    println!("Load time: {:?} (only dictionary)", lazy_load_time);
    println!("Unique terms: {}", lazy_reader.stats().unique_terms);
    
    // First lookup (cold)
    let term_rust = Term::new("rust");
    let start = std::time::Instant::now();
    let _postings = lazy_reader.get_postings(&term_rust)?;
    let cold_lookup = start.elapsed();
    println!("First lookup (cold): {:?}", cold_lookup);
    
    // Second lookup (warm)
    let start = std::time::Instant::now();
    let _postings = lazy_reader.get_postings(&term_rust)?;
    let warm_lookup = start.elapsed();
    println!("Second lookup (warm): {:?}", warm_lookup);
    
    let cache_stats = lazy_reader.cache_stats();
    println!("Cache hit rate: {:.2}%", cache_stats.hit_rate * 100.0);
    println!("Memory: {} terms cached\n", cache_stats.size);
    
    // Example 3: Hybrid adaptive
    println!("Example 3: Hybrid Adaptive (Auto-select)");
    println!("-----------------------------------------");
    let start = std::time::Instant::now();
    let hybrid_reader = HybridIndexReader::open(&storage, segment_id, LoadingStrategy::Adaptive)?;
    let hybrid_load_time = start.elapsed();
    println!("Load time: {:?}", hybrid_load_time);
    println!("Strategy used: {:?}", hybrid_reader.strategy());
    
    let stats = hybrid_reader.stats();
    println!("Unique terms: {}", stats.unique_terms);
    if let Some(hit_rate) = stats.cache_hit_rate {
        println!("Cache hit rate: {:.2}%\n", hit_rate * 100.0);
    }
    
    // Comparison
    println!("📊 Performance Comparison:");
    println!("┌─────────────────┬──────────────┬──────────────┐");
    println!("│ Strategy        │ Load Time    │ Memory       │");
    println!("├─────────────────┼──────────────┼──────────────┤");
    println!("│ Eager           │ {:>10?} │ High (full)  │", eager_load_time);
    println!("│ Lazy            │ {:>10?} │ Low (cache)  │", lazy_load_time);
    println!("│ Hybrid          │ {:>10?} │ Adaptive     │", hybrid_load_time);
    println!("└─────────────────┴──────────────┴──────────────┘\n");
    
    println!("✅ Lazy loading is {}x faster on startup!",
        eager_load_time.as_micros() / lazy_load_time.as_micros().max(1));
    
    Ok(())
}
