/// Fuzzy Search Demo - Typo-tolerant search
///
/// This example demonstrates fuzzy search with edit distance (Levenshtein)
/// 
/// Features:
/// - Automatic typo correction
/// - Configurable edit distance (1-2)
/// - Query syntax: "term~" or "term~2"

use Drusdenx::core::config::{Config, MergePolicyType};
use Drusdenx::core::types::{Document, DocId, FieldValue};
use Drusdenx::schema::schema::SchemaWithAnalyzer;
use Drusdenx::core::database::Database;
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Fuzzy Search Demo ===\n");
    
    // Setup database
    let temp_dir = tempfile::tempdir()?;
    let mut config = Config::default();
    config.storage_path = PathBuf::from(temp_dir.path());
    config.merge_policy = MergePolicyType::Tiered;
    
    // Create schema
    let schema = SchemaWithAnalyzer::new()
        .add_text_field("title", None)
        .add_text_field("content", None);
    
    let db = Database::open_with_schema(schema, config)?;
    
    println!("Adding test documents with typos and variations...");
    
    // Add documents with variations
    let docs = vec![
        ("Rust Programming Language", "Learn Rust programming with examples"),
        ("Python Development", "Python is great for scripting and automation"),
        ("JavaScript Tutorial", "Modern JavaScript with ES6 features"),
        ("Database Design", "Learn about database normalization"),
        ("Search Engine Optimization", "SEO best practices for websites"),
        ("Machine Learning Basics", "Introduction to machine learning algorithms"),
    ];
    
    for (id, (title, content)) in docs.iter().enumerate() {
        let mut doc = Document::new(DocId(id as u64));
        doc.fields.insert("title".to_string(), FieldValue::Text(title.to_string()));
        doc.fields.insert("content".to_string(), FieldValue::Text(content.to_string()));
        db.add_document(doc)?;
    }
    
    db.flush()?;
    println!("✅ Indexed {} documents\n", docs.len());
    
    // Example 1: Exact match (baseline)
    println!("Example 1: Exact Match");
    println!("----------------------");
    let query = "rust";
    let results = db.search(query)?;
    println!("Query: \"{}\"", query);
    println!("Results: {} hits", results.len());
    for hit in results.iter().take(3) {
        if let Some(doc) = &hit.document {
            if let Some(FieldValue::Text(title)) = doc.fields.get("title") {
                println!("  - {} (score: {:.2})", title, hit.score);
            }
        }
    }
    println!();
    
    // Example 2: Fuzzy search with 1 edit (typo)
    println!("Example 2: Fuzzy Search (1 edit)");
    println!("---------------------------------");
    let typo_queries = vec![
        ("ruste", "rust"),   // Missing 'e'
        ("pythno", "python"), // Transposed 'n' and 'o'
        ("srcipt", "script"), // Missing 's'
    ];
    
    for (typo, correct) in &typo_queries {
        let query = format!("{}~1", typo);  // ~1 = max 1 edit
        let results = db.search(&query)?;
        println!("Query: \"{}\" (typo for \"{}\")", query, correct);
        println!("Results: {} hits", results.len());
        if results.len() > 0 {
            println!("  ✅ Found match despite typo!");
            for hit in results.iter().take(2) {
                if let Some(doc) = &hit.document {
                    if let Some(FieldValue::Text(title)) = doc.fields.get("title") {
                        println!("    - {}", title);
                    }
                }
            }
        } else {
            println!("  ❌ No matches");
        }
        println!();
    }
    
    // Example 3: Fuzzy search with 2 edits (more forgiving)
    println!("Example 3: Fuzzy Search (2 edits)");
    println!("----------------------------------");
    let query = "machne~2";  // "machine" with 2 typos
    let results = db.search(&query)?;
    println!("Query: \"{}\" (typo for \"machine\")", query);
    println!("Results: {} hits", results.len());
    if results.len() > 0 {
        println!("  ✅ Found match despite 2 typos!");
        for hit in results.iter().take(3) {
            if let Some(doc) = &hit.document {
                if let Some(FieldValue::Text(content)) = doc.fields.get("content") {
                    println!("    - {}", content);
                }
            }
        }
    }
    println!();
    
    // Example 4: Default fuzzy (~ without number = distance 1)
    println!("Example 4: Default Fuzzy");
    println!("-------------------------");
    let query = "databse~";  // "database" with typo
    let results = db.search(&query)?;
    println!("Query: \"{}\" (default = 1 edit)", query);
    println!("Results: {} hits", results.len());
    for hit in results.iter().take(2) {
        if let Some(doc) = &hit.document {
            if let Some(FieldValue::Text(title)) = doc.fields.get("title") {
                println!("  - {}", title);
            }
        }
    }
    println!();
    
    // Performance comparison
    println!("📊 Performance Characteristics:");
    println!("┌──────────────────┬──────────────┬────────────────┐");
    println!("│ Query Type       │ Tolerance    │ Use Case       │");
    println!("├──────────────────┼──────────────┼────────────────┤");
    println!("│ Exact (term)     │ None         │ Known spelling │");
    println!("│ Fuzzy ~1         │ 1 typo       │ User input     │");
    println!("│ Fuzzy ~2         │ 2 typos      │ Very forgiving │");
    println!("└──────────────────┴──────────────┴────────────────┘\n");
    
    println!("✨ Key Benefits:");
    println!("  1. Better UX - Users don't need perfect spelling");
    println!("  2. 10-20% of queries have typos (real-world data)");
    println!("  3. Fast matching with Levenshtein distance");
    println!("  4. Configurable tolerance (1-2 edits recommended)");
    
    println!("\n✅ Fuzzy search demo complete!");
    
    Ok(())
}
