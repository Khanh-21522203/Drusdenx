/// Debug test to understand search flow
use Drusdenx::core::config::Config;
use Drusdenx::core::types::{Document, DocId, FieldValue};
use Drusdenx::schema::schema::SchemaWithAnalyzer;
use Drusdenx::core::database::Database;
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let temp_dir = tempfile::tempdir()?;
    let mut config = Config::default();
    config.storage_path = PathBuf::from(temp_dir.path());
    config.writer_batch_size = 1; // Flush after every document
    
    let storage_path = config.storage_path.clone();
    println!("=== Debug Search Test ===");
    println!("Storage: {:?}\n", storage_path);
    
    let schema = SchemaWithAnalyzer::new()
        .add_text_field("content", None);
    
    let db = Database::open_with_schema(schema, config)?;
    
    // Add ONE document
    println!("Step 1: Adding document...");
    let mut doc = Document::new(DocId(1));
    doc.fields.insert("content".to_string(), FieldValue::Text("rust programming language".to_string()));
    db.add_document(doc)?;
    println!("  ✓ Document added\n");
    
    // Manually flush to ensure segment is written
    println!("Step 2: Flushing to disk...");
    db.flush()?;
    println!("  ✓ Flushed\n");
    
    // Check files
    println!("Step 3: Checking files...");
    use std::fs;
    let segments_path = storage_path.join("segments");
    if let Ok(entries) = fs::read_dir(&segments_path) {
        let count = entries.count();
        println!("  segments/: {} files", count);
        
        // List them
        if let Ok(entries2) = fs::read_dir(&segments_path) {
            for entry in entries2.filter_map(|e| e.ok()) {
                let size = entry.metadata().ok().map(|m| m.len()).unwrap_or(0);
                println!("    - {:?} ({} bytes)", entry.file_name(), size);
            }
        }
    }
    
    let idx_path = storage_path.join("idx");
    if let Ok(entries) = fs::read_dir(&idx_path) {
        let count = entries.count();
        println!("  idx/: {} files", count);
        
        if let Ok(entries2) = fs::read_dir(&idx_path) {
            for entry in entries2.filter_map(|e| e.ok()) {
                let size = entry.metadata().ok().map(|m| m.len()).unwrap_or(0);
                println!("    - {:?} ({} bytes)", entry.file_name(), size);
            }
        }
    }
    println!();
    
    // Try simple search
    println!("Step 4: Testing search...");
    println!("  Query: 'rust'");
    let results = db.search("rust")?;
    println!("  Results: {} hits", results.len());
    
    if results.is_empty() {
        println!("\n❌ NO RESULTS - This is the bug we need to fix!");
    } else {
        println!("\n✅ SUCCESS!");
        for hit in &results {
            println!("  - Doc {}: score={:.2}", hit.doc_id.0, hit.score);
        }
    }
    
    // Try fuzzy
    println!("\nStep 5: Testing fuzzy search...");
    println!("  Query: 'ruste~1' (fuzzy for 'rust')");
    let results = db.search("ruste~1")?;
    println!("  Results: {} hits", results.len());
    
    if !results.is_empty() {
        println!("  ✅ Fuzzy search works!");
        for hit in &results {
            println!("  - Doc {}: score={:.2}", hit.doc_id.0, hit.score);
        }
    }
    
    Ok(())
}
