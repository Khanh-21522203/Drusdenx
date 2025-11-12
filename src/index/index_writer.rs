use std::collections::HashMap;
use std::sync::Arc;
use crate::analysis::analyzer::Analyzer;
use crate::core::types::Document;
use crate::index::inverted::Term;
use crate::index::posting::Posting;
use crate::parallel::indexer::ParallelIndexer;
use crate::core::error::Result;

/// IndexWriter handles inverted index building
pub struct IndexWriter {
    pub parallel_indexer: Arc<ParallelIndexer>,
    pub analyzer: Arc<Analyzer>,
    pub inverted_index: HashMap<Term, Vec<Posting>>,
}

impl IndexWriter {
    pub fn new(
        parallel_indexer: Arc<ParallelIndexer>,
        analyzer: Arc<Analyzer>,
    ) -> Self {
        IndexWriter {
            parallel_indexer,
            analyzer,
            inverted_index: HashMap::new(),
        }
    }

    /// Index a document and add to inverted index
    pub fn index_document(&mut self, doc: &Document) -> Result<()> {
        // Index the document (tokenize and analyze)
        let indexed_docs = self.parallel_indexer.index_batch(vec![doc.clone()], &self.analyzer)?;
        
        if let Some(indexed_doc) = indexed_docs.first() {
            // Create term positions map
            let mut term_positions: HashMap<String, Vec<usize>> = HashMap::new();
            
            for (pos, token) in indexed_doc.tokens.iter().enumerate() {
                term_positions
                    .entry(token.text.clone())
                    .or_insert_with(Vec::new)
                    .push(pos);
            }
            
            // Create postings for each term
            for (term_text, positions) in term_positions {
                let term = Term::new(&term_text);
                let posting = Posting {
                    doc_id: doc.id,
                    term_freq: positions.len() as u32,
                    positions: positions.into_iter().map(|p| p as u32).collect(),
                    field_norm: 1.0 / (indexed_doc.terms.len() as f32).sqrt(),
                };
                
                self.inverted_index
                    .entry(term)
                    .or_insert_with(Vec::new)
                    .push(posting);
            }
        }
        
        Ok(())
    }

    /// Index multiple documents in batch
    pub fn index_documents_batch(&mut self, docs: Vec<Document>) -> Result<()> {
        for doc in docs {
            self.index_document(&doc)?;
        }
        Ok(())
    }

    /// Get the inverted index and reset
    pub fn take_index(&mut self) -> HashMap<Term, Vec<Posting>> {
        std::mem::take(&mut self.inverted_index)
    }

    /// Clear the inverted index
    pub fn clear(&mut self) {
        self.inverted_index.clear();
    }
}
