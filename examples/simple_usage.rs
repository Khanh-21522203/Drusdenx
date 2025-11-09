/// Complete Drusdenx API Demo
/// 
/// Demonstrates all major database operations:
/// - CRUD operations (Create, Read, Update, Delete)
/// - Search (simple, boolean, field-specific)
/// - Transactions
/// - Statistics and monitoring
/// - Health checks

use Drusdenx::core::database::Database;
use Drusdenx::core::config::Config;
use Drusdenx::core::types::{Document, DocId, FieldValue};
use Drusdenx::schema::schema::SchemaWithAnalyzer;
use Drusdenx::mvcc::controller::IsolationLevel;
use std::collections::HashMap;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("\n╔═══════════════════════════════════════════════╗");
    println!("║   Drusdenx Database - Complete API Demo      ║");
    println!("╚═══════════════════════════════════════════════╝\n");

    // Step 1: Create database
    println!("Creating database...");
    let config = Config::default();
    let schema = SchemaWithAnalyzer::new();
    let db = Database::open_with_schema(schema, config)?;
    println!("Done!\n");

    // Step 2: INSERT - Add documents
    println!("Step 2: INSERT - Adding documents...");
    
    let doc1 = create_document(1, "Rust Programming", "Learn Rust language");
    db.add_document(doc1)?;
    
    let doc2 = create_document(2, "Database Systems", "SQL and NoSQL databases");
    db.add_document(doc2)?;
    
    let doc3 = create_document(3, "Web Development", "Building web apps");
    db.add_document(doc3)?;
    
    db.flush()?;
    db.commit()?;
    println!("  Inserted 3 documents\n");

    // Step 3: SEARCH - Different query types
    println!("Step 3: SEARCH - Querying documents...");
    
    // Simple search
    match db.search("rust") {
        Ok(results) => println!("  'rust': {} results", results.len()),
        Err(_) => println!("  'rust': 0 results"),
    }
    
    // Field-specific search
    match db.search("title:Database") {
        Ok(results) => println!("  'title:Database': {} results", results.len()),
        Err(_) => println!("  'title:Database': 0 results"),
    }
    
    // Boolean search
    match db.search("rust AND programming") {
        Ok(results) => println!("  'rust AND programming': {} results", results.len()),
        Err(_) => println!("  'rust AND programming': 0 results"),
    }
    println!();

    // Step 4: UPDATE - Modify a document (delete + re-add)
    println!("Step 4: UPDATE - Updating document...");
    match db.delete_document(DocId(2)) {
        Ok(_) => {
            let updated_doc = create_document(2, "Advanced Databases", "Deep dive into database internals");
            match db.add_document(updated_doc) {
                Ok(_) => println!("  Updated document ID 2"),
                Err(e) => println!("  Re-add failed: {}", e),
            }
        },
        Err(e) => println!("  Update failed: {}", e),
    }
    db.flush()?;
    println!();

    // Step 5: DELETE - Remove a document
    println!("Step 5: DELETE - Removing document...");
    
    // Delete single document
    match db.delete_document(DocId(3)) {
        Ok(_) => println!("  Deleted document ID 3"),
        Err(e) => println!("  Delete failed: {}", e),
    }
    db.flush()?;
    println!();

    // Step 6: TRANSACTIONS - Atomic operations
    println!("Step 6: TRANSACTIONS - Using transactions...");
    
    let tx_result = db.with_transaction(IsolationLevel::Serializable, |tx| {
        let doc4 = create_document(4, "Transaction Test", "This is in a transaction");
        tx.insert(doc4)?;
        
        let doc5 = create_document(5, "Another Doc", "Also in transaction");
        tx.insert(doc5)?;
        
        Ok(())
    });
    
    match tx_result {
        Ok(_) => println!("  Transaction committed successfully"),
        Err(e) => println!("  Transaction failed: {}", e),
    }
    println!();

    // Step 7: COMPACT - Clean up deleted documents
    println!("Step 7: COMPACT - Cleaning up...");
    match db.compact() {
        Ok(_) => println!("  Database compacted"),
        Err(e) => println!("  Compact failed: {}", e),
    }
    println!();

    // Step 8: STATS - Detailed statistics
    println!("Step 8: STATISTICS - Database metrics:");
    println!("  ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    if let Ok(stats) = db.stats() {
        println!("  Total Documents:     {}", stats.total_documents);
        println!("  Deleted Documents:   {}", stats.deleted_documents);
        println!("  Segments:            {}", stats.segment_count);
        println!("  Index Size (bytes):  {}", stats.index_size_bytes);
        println!("  Queries Per Second:  {}", stats.queries_per_second);
        println!("  Writes Per Second:   {}", stats.writes_per_second);
        println!("  Avg Query Latency:   {} ms", stats.avg_query_latency_ms);
    }
    println!();

    // Step 9: HEALTH CHECK - System health
    println!("Step 9: HEALTH CHECK - System status:");
    println!("  ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    if let Ok(health) = db.health_check() {
        println!("  Status: {:?}", health.status);
        println!("  Total Checks: {}", health.checks.len());
        for check in &health.checks {
            println!("    - {}: {:?} ({}ms)", check.name, check.status, check.latency_ms);
        }
    }

    println!("\n╔════════════════════════════════════════╗");
    println!("║    All API Operations Completed!      ║");
    println!("╚════════════════════════════════════════╝\n");

    Ok(())
}

/// Helper function to create a document
fn create_document(id: u64, title: &str, content: &str) -> Document {
    Document {
        id: DocId(id),
        fields: HashMap::from([
            ("title".to_string(), FieldValue::Text(title.to_string())),
            ("content".to_string(), FieldValue::Text(content.to_string())),
        ]),
    }
}
