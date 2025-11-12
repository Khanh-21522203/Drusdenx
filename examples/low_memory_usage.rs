/// Example: Using LowMemoryMode with Database
/// 
/// This demonstrates how to enable and use low memory mode for constrained environments

use Drusdenx::core::config::Config;
use Drusdenx::core::types::{Document, DocId, FieldValue};
use Drusdenx::memory::low_memory::LowMemoryConfig;
use Drusdenx::schema::schema::SchemaWithAnalyzer;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Create database with normal config
    let mut config = Config::default();
    config.memory_limit = 50 * 1024 * 1024;  // 50MB
    config.storage_path = std::path::PathBuf::from("./data");
    
    let schema = SchemaWithAnalyzer::new();
    let mut db = Drusdenx::core::database::Database::open_with_schema(schema, config)?;
    
    // 2. Enable low memory mode with custom config
    let low_mem_config = LowMemoryConfig {
        heap_limit: 50 * 1024 * 1024,      // 50MB limit
        buffer_size: 4 * 1024,              // 4KB buffers
        cache_size: 5 * 1024 * 1024,        // 5MB cache
        batch_size: 100,                    // Small batches
        enable_compression: true,           // Force compression
        swap_to_disk: true,                 // Use disk for overflow
        gc_threshold: 0.8,                  // Trigger GC at 80%
    };
    
    db.enable_low_memory_mode(low_mem_config);
    println!("✓ Low memory mode enabled");
    
    // 3. Check if enabled
    if db.is_low_memory_mode_enabled() {
        println!("✓ Low memory mode is active");
    }
    
    // 4. Add documents (will auto-reclaim memory when pressure > 0.8)
    for i in 0..1000 {
        let mut doc = Document::new(DocId::new(i as u64));
        doc.add_field("title".to_string(), FieldValue::Text(format!("Document {}", i)));
        doc.add_field("content".to_string(), FieldValue::Text("Lorem ipsum dolor sit amet...".to_string()));
        
        db.add_document(doc)?;
        
        // Check memory pressure periodically
        if i % 100 == 0 {
            if let Some(pressure) = db.get_memory_pressure() {
                println!("Document {}: Memory pressure = {:.1}%", i, pressure * 100.0);
            }
        }
    }
    
    // 5. Manually trigger reclamation if needed
    println!("\nManually triggering memory reclamation...");
    db.maybe_reclaim_memory()?;
    println!("✓ Memory reclamation completed");
    
    // 6. Check final memory pressure
    if let Some(pressure) = db.get_memory_pressure() {
        println!("Final memory pressure: {:.1}%", pressure * 100.0);
    }
    
    // 7. Flush and commit
    db.flush()?;
    db.commit()?;
    println!("✓ Database flushed and committed");
    
    Ok(())
}
