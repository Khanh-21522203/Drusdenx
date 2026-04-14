use Drusdenx::core::config::Config;
use Drusdenx::core::types::{Document, DocId, FieldValue};
use Drusdenx::schema::schema::SchemaWithAnalyzer;
use Drusdenx::core::database::Database;
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let temp_dir = tempfile::tempdir()?;
    let mut config = Config::default();
    config.storage_path = PathBuf::from(temp_dir.path());
    
    let storage_path = config.storage_path.clone();
    println!("Storage path: {:?}", storage_path);
    
    let schema = SchemaWithAnalyzer::new()
        .add_text_field("content", None);
    
    let db = Database::open_with_schema(schema, config)?;
    
    // Add test document
    let mut doc = Document::new(DocId(1));
    doc.fields.insert("content".to_string(), FieldValue::Text("rust programming".to_string()));
    println!("Adding document...");
    db.add_document(doc)?;
    
    println!("Flushing...");
    db.flush()?;
    
    // Check if segment files exist
    use std::fs;
    println!("\nChecking storage...");
    let segments_path = storage_path.join("segments");
    if segments_path.exists() {
        let entries: Vec<_> = fs::read_dir(&segments_path)?
            .filter_map(|e| e.ok())
            .collect();
        println!("  segments/ has {} files", entries.len());
    }
    let idx_path = storage_path.join("idx");
    if idx_path.exists() {
        let entries: Vec<_> = fs::read_dir(&idx_path)?
            .filter_map(|e| e.ok())
            .collect();
        println!("  idx/ has {} files", entries.len());
    }
    
    println!("\nTesting searches:");
    
    // Test exact match
    let results = db.search("rust")?;
    println!("Exact 'rust': {} hits", results.len());
    for (i, result) in results.iter().enumerate() {
        println!("  Hit {}: doc_id={}, score={}", i+1, result.doc_id.0, result.score);
    }
    
    // Test fuzzy
    let results = db.search("ruste~1")?;
    println!("\nFuzzy 'ruste~1': {} hits", results.len());
    for (i, result) in results.iter().enumerate() {
        println!("  Hit {}: doc_id={}, score={}", i+1, result.doc_id.0, result.score);
    }
    
    Ok(())
}
