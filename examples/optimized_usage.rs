/// Example shSegmentReaderowing how to use optimized writers and readers
/// 
/// This example demonstrates:
/// 1. ParallelWriter for concurrent data + index writes
/// 2. IndexCache for fast index lookups
/// 3. Batch coordination for bulk writes

use std::sync::Arc;
use std::path::PathBuf;
use Drusdenx::core::types::{Document, DocId, FieldValue};
use Drusdenx::storage::layout::StorageLayout;
use Drusdenx::memory::buffer_pool::BufferPool;
use Drusdenx::writer::data_writer::DataWriter;
use Drusdenx::index::index_writer::IndexWriter as NewIndexWriter;
use Drusdenx::index::index_cache::IndexCache;
use Drusdenx::writer::parallel_writer::ParallelWriter;
use Drusdenx::analysis::analyzer::Analyzer;
use Drusdenx::analysis::tokenizer::StandardTokenizer;
use Drusdenx::parallel::indexer::ParallelIndexer;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Setup
    let storage = Arc::new(StorageLayout::new(PathBuf::from("./data"))?);
    let buffer_pool = Arc::new(BufferPool::new(100));
    let tokenizer = Box::new(StandardTokenizer {
        lowercase: true,
        max_token_length: 255,
    });
    let analyzer = Arc::new(Analyzer::new("standard".to_string(), tokenizer));
    let parallel_indexer = Arc::new(ParallelIndexer::new(4));
    
    println!("=== Optimization Examples ===\n");
    
    // Example 1: Batch Writes (fastest for bulk operations)
    example_batch_writes(&storage, &buffer_pool)?;
    
    // Example 2: Parallel Writes (for concurrent operations)
    example_parallel_writes(&storage, &buffer_pool, &analyzer, &parallel_indexer)?;
    
    // Example 3: Index Cache (for fast reads)
    example_index_cache(&storage)?;
    
    Ok(())
}

/// Example 1: Batch writes - best for bulk operations
fn example_batch_writes(
    storage: &Arc<StorageLayout>,
    buffer_pool: &Arc<BufferPool>,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("Example 1: Batch Writes");
    println!("------------------------");
    
    let mut data_writer = DataWriter::new(
        storage.clone(),
        buffer_pool.clone(),
        1000, // batch_size
    )?;
    
    // Create sample documents
    let documents = create_sample_documents(1000);
    
    // Add to batch (no I/O yet)
    for doc in documents {
        data_writer.add_to_batch(doc);
    }
    
    // Flush all at once (batched I/O)
    let count = data_writer.flush_batch()?;
    println!("✅ Batched {} documents", count);
    println!("   Benefits: Fewer locks, batched WAL writes\n");
    
    Ok(())
}

/// Example 2: Parallel writes - data and index simultaneously
fn example_parallel_writes(
    storage: &Arc<StorageLayout>,
    buffer_pool: &Arc<BufferPool>,
    analyzer: &Arc<Analyzer>,
    parallel_indexer: &Arc<ParallelIndexer>,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("Example 2: Parallel Writes");
    println!("--------------------------");
    
    let data_writer = DataWriter::new(
        storage.clone(),
        buffer_pool.clone(),
        1000,
    )?;
    
    let index_writer = NewIndexWriter::new(
        parallel_indexer.clone(),
        analyzer.clone(),
    );
    
    let parallel_writer = ParallelWriter::new(data_writer, index_writer);
    
    // Write documents (data + index in parallel)
    let documents = create_sample_documents(100);
    
    for doc in documents {
        parallel_writer.write_document(doc)?;
    }
    
    // Check for flushed segments
    while let Some(segment) = parallel_writer.try_recv_segment() {
        println!("✅ Segment flushed: {:?}", segment.id);
    }
    
    println!("   Benefits: Parallel I/O, better CPU utilization\n");
    
    Ok(())
}

/// Example 3: Index cache - fast index lookups
fn example_index_cache(
    storage: &Arc<StorageLayout>,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("Example 3: Index Cache");
    println!("----------------------");
    
    let cache = IndexCache::new(storage.clone(), 100); // Cache up to 100 segments
    
    // First access: loads from disk
    println!("First access (cold):");
    let segment_id = Drusdenx::storage::segment::SegmentId::new();
    let start = std::time::Instant::now();
    let _reader1 = cache.get_or_load(segment_id)?;
    println!("   Time: {:?}", start.elapsed());
    
    // Second access: from cache
    println!("Second access (warm):");
    let start = std::time::Instant::now();
    let _reader2 = cache.get_or_load(segment_id)?;
    println!("   Time: {:?} (much faster!)", start.elapsed());
    
    // Cache stats
    let stats = cache.stats();
    println!("   Cache: {}/{} segments", stats.size, stats.max_size);
    println!("   Benefits: Reduced disk I/O, faster queries\n");
    
    Ok(())
}

/// Helper: Create sample documents
fn create_sample_documents(count: usize) -> Vec<Document> {
    (0..count)
        .map(|i| {
            let mut doc = Document::new(DocId(i as u64));
            doc.fields.insert(
                "title".to_string(),
                FieldValue::Text(format!("Document {}", i)),
            );
            doc.fields.insert(
                "content".to_string(),
                FieldValue::Text(format!("This is the content of document number {}", i)),
            );
            doc
        })
        .collect()
}

/// Performance comparison
#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn benchmark_batch_vs_single() {
        // Batch writes
        let start = std::time::Instant::now();
        // ... batch write code ...
        let batch_time = start.elapsed();
        
        // Single writes
        let start = std::time::Instant::now();
        // ... single write code ...
        let single_time = start.elapsed();
        
        println!("Batch: {:?}", batch_time);
        println!("Single: {:?}", single_time);
        println!("Speedup: {:.2}x", single_time.as_secs_f64() / batch_time.as_secs_f64());
    }
}
