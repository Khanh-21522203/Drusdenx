use std::collections::HashMap;
use std::path::PathBuf;
use serde::{Deserialize, Serialize};
use crate::core::types::{Document, FieldValue};
use crate::index::inverted::Term;
use crate::memory::low_memory::LowMemoryConfig;
use crate::storage::segment::SegmentId;
use crate::core::error::Result;

/// Incremental indexer for memory-constrained indexing
pub struct IncrementalIndexer {
    pub config: LowMemoryConfig,
    pub delta_segments: Vec<DeltaSegment>,
    pub merge_threshold: usize,
    pub temp_dir: PathBuf,
}

/// Small in-memory segment
#[derive(Serialize, Deserialize)]
pub struct DeltaSegment {
    pub id: SegmentId,
    pub postings: HashMap<Term, SmallPostingList>,
    pub doc_count: usize,
    pub memory_usage: usize,
}

/// Compact posting list for low memory
#[derive(Serialize, Deserialize)]
pub struct SmallPostingList {
    pub docs: Vec<u32>,  // Use u32 instead of DocId
    pub compressed: bool,
}

impl IncrementalIndexer {
    pub fn new(config: LowMemoryConfig) -> Self {
        IncrementalIndexer {
            config,
            delta_segments: Vec::new(),
            merge_threshold: 10,
            temp_dir: std::env::temp_dir().join("index"),
        }
    }

    /// Index document incrementally
    pub fn index_document(&mut self, doc: Document) -> Result<()> {
        // Check if we need a new segment
        let needs_new_segment = self.delta_segments.is_empty() || self.should_create_new_segment();
        
        if needs_new_segment {
            let segment = DeltaSegment {
                id: SegmentId::new(),
                postings: HashMap::new(),
                doc_count: 0,
                memory_usage: 0,
            };
            self.delta_segments.push(segment);
        }

        // Get the last segment index
        let segment_index = self.delta_segments.len() - 1;
        
        // Add document to segment
        self.add_to_segment_by_index(segment_index, doc)?;

        // Check if should flush
        let should_flush = self.delta_segments[segment_index].memory_usage > self.config.buffer_size * 10;
        if should_flush {
            self.flush_segment_by_index(segment_index)?;
        }

        Ok(())
    }

    /// Index batch with memory constraints
    pub fn index_batch(&mut self, documents: Vec<Document>) -> Result<()> {
        let batch_size = self.config.batch_size;

        for chunk in documents.chunks(batch_size) {
            for doc in chunk {
                self.index_document(doc.clone())?;
            }

            // Check memory and potentially flush
            if self.get_memory_usage() > self.config.heap_limit / 2 {
                self.flush_all()?;
            }
        }

        Ok(())
    }

    fn should_create_new_segment(&self) -> bool {
        if let Some(last) = self.delta_segments.last() {
            last.memory_usage > self.config.buffer_size * 10
        } else {
            true
        }
    }


    fn add_to_segment_by_index(&mut self, segment_index: usize, doc: Document) -> Result<()> {
        // Extract terms before getting mutable segment reference
        let terms = self.extract_terms(&doc);
        let doc_id = doc.id.0 as u32;
        
        // Now get mutable segment reference
        let segment = &mut self.delta_segments[segment_index];

        // Update postings
        for term in terms {
            let posting = segment.postings
                .entry(term)
                .or_insert_with(|| SmallPostingList {
                    docs: Vec::new(),
                    compressed: false,
                });

            posting.docs.push(doc_id);

            // Compress if getting large
            if posting.docs.len() > 100 && !posting.compressed {
                // Inline compression logic to avoid borrowing issues
                posting.docs.sort_unstable();
                posting.docs.dedup();
                posting.compressed = true;
            }
        }

        segment.doc_count += 1;
        segment.memory_usage += std::mem::size_of::<Document>();

        Ok(())
    }

    fn compress_posting(&self, posting: &mut SmallPostingList) -> Result<()> {
        // Sort and delta encode
        posting.docs.sort_unstable();
        posting.docs.dedup();

        // Mark as compressed
        posting.compressed = true;

        Ok(())
    }

    fn flush_segment_by_index(&mut self, segment_index: usize) -> Result<()> {
        let segment = &mut self.delta_segments[segment_index];
        // Write segment to disk
        let path = self.temp_dir.join(format!("{}.seg", segment.id.to_string()));

        // Serialize segment
        let bytes = bincode::serialize(segment)?;
        std::fs::write(path, bytes)?;

        // Clear memory
        segment.postings.clear();
        segment.memory_usage = 0;

        Ok(())
    }

    fn flush_all(&mut self) -> Result<()> {
        for i in 0..self.delta_segments.len() {
            self.flush_segment_by_index(i)?;
        }
        Ok(())
    }

    fn get_memory_usage(&self) -> usize {
        self.delta_segments.iter()
            .map(|s| s.memory_usage)
            .sum()
    }

    fn extract_terms(&self, doc: &Document) -> Vec<Term> {
        // Simple term extraction
        let mut terms = Vec::new();

        for (_, value) in &doc.fields {
            if let FieldValue::Text(text) = value {
                for word in text.split_whitespace() {
                    terms.push(Term::new(word));
                }
            }
        }

        terms
    }
}