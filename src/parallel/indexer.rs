use rayon::prelude::*;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::collections::HashMap;
use crate::analysis::analyzer::Analyzer;
use crate::analysis::token::Token;
use crate::core::types::{DocId, Document, FieldValue};
use crate::index::inverted::Term;
use crate::index::posting::Posting;
use crate::core::error::Result;

/// Parallel document indexer for high-throughput indexing
pub struct ParallelIndexer {
    pub workers: usize,
    pub batch_size: usize,
    pub progress: Arc<AtomicUsize>,
}

impl ParallelIndexer {
    pub fn new(workers: usize) -> Self {
        // Set number of threads for rayon
        rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build_global()
            .ok();
            
        ParallelIndexer {
            workers,
            batch_size: 1000,
            progress: Arc::new(AtomicUsize::new(0)),
        }
    }
    
    /// Get current progress
    pub fn get_progress(&self) -> usize {
        self.progress.load(Ordering::Relaxed)
    }

    /// Index batch of documents in parallel
    pub fn index_batch(&self, documents: Vec<Document>, analyzer: &Arc<Analyzer>) -> Result<Vec<IndexedDoc>> {
        self.progress.store(0, Ordering::Relaxed);
        let total_docs = documents.len();
        
        // Process documents in parallel batches
        let indexed: Vec<IndexedDoc> = documents
            .par_chunks(self.batch_size)
            .flat_map(|batch| {
                let batch_results: Vec<IndexedDoc> = batch
                    .par_iter()
                    .filter_map(|doc| {
                        let result = self.index_document(doc, analyzer);
                        self.progress.fetch_add(1, Ordering::Relaxed);
                        
                        // Log progress every 1000 documents
                        let current = self.progress.load(Ordering::Relaxed);
                        if current % 1000 == 0 {
                            let percent = (current * 100) / total_docs;
                            eprintln!("Indexing progress: {}% ({}/{})", percent, current, total_docs);
                        }
                        
                        result.ok()
                    })
                    .collect();
                batch_results
            })
            .collect();

        Ok(indexed)
    }
    
    /// Index documents and build inverted index structure in parallel
    pub fn build_inverted_index(
        &self,
        documents: Vec<Document>,
        analyzer: &Arc<Analyzer>,
    ) -> Result<HashMap<Term, Vec<Posting>>> {
        let indexed_docs = self.index_batch(documents, analyzer)?;
        
        // Build inverted index from indexed documents
        let mut inverted: HashMap<Term, Vec<Posting>> = HashMap::new();
        
        for indexed_doc in indexed_docs {
            // Group positions by term for this document
            let mut term_positions: HashMap<Term, Vec<u32>> = HashMap::new();
            
            for (pos, term) in indexed_doc.terms.iter().enumerate() {
                term_positions
                    .entry(term.clone())
                    .or_insert_with(Vec::new)
                    .push(pos as u32);
            }
            
            // Create postings for each term
            for (term, positions) in term_positions {
                let posting = Posting {
                    doc_id: indexed_doc.doc_id,
                    term_freq: positions.len() as u32,
                    positions,
                    field_norm: 1.0 / (indexed_doc.terms.len() as f32).sqrt(),
                };
                
                inverted
                    .entry(term)
                    .or_insert_with(Vec::new)
                    .push(posting);
            }
        }
        
        // Sort postings by doc_id for each term
        for postings in inverted.values_mut() {
            postings.sort_by_key(|p| p.doc_id);
        }
        
        Ok(inverted)
    }

    fn index_document(&self, doc: &Document, analyzer: &Arc<Analyzer>) -> Result<IndexedDoc> {
        let mut terms = Vec::new();
        let mut all_tokens = Vec::new();

        for (_field, value) in &doc.fields {
            if let FieldValue::Text(text) = value {
                let tokens = analyzer.analyze(text);
                all_tokens.extend(tokens);
            }
        }
        
        // Convert tokens to terms
        for token in &all_tokens {
            terms.push(Term::new(&token.text));
        }

        Ok(IndexedDoc {
            doc_id: doc.id,
            terms,
            tokens: all_tokens,
        })
    }
}

pub struct IndexedDoc {
    pub doc_id: DocId,
    pub terms: Vec<Term>,
    pub tokens: Vec<Token>,
}