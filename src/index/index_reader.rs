use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use crate::compression::compress::CompressedBlock;
use crate::index::inverted::Term;
use crate::index::posting::Posting;
use crate::storage::layout::StorageLayout;
use crate::storage::segment::SegmentId;
use crate::core::error::Result;

/// IndexReader reads inverted index from .idx files
pub struct IndexReader {
    pub segment_id: SegmentId,
    pub inverted_index: HashMap<Term, Vec<Posting>>,
}

impl IndexReader {
    /// Open and read index file
    pub fn open(storage: &StorageLayout, segment_id: SegmentId) -> Result<Self> {
        let index_path = storage.index_path(&segment_id);
        
        // Check if index file exists
        if !index_path.exists() {
            // Return empty index if file doesn't exist
            return Ok(IndexReader {
                segment_id,
                inverted_index: HashMap::new(),
            });
        }
        
        // Read compressed index file
        let mut index_file = File::open(index_path)?;
        let mut compressed_block_data = Vec::new();
        index_file.read_to_end(&mut compressed_block_data)?;
        
        // Deserialize CompressedBlock
        let compressed_block: CompressedBlock = bincode::deserialize(&compressed_block_data)?;
        
        // Decompress
        let decompressed = CompressedBlock::decompress(&compressed_block)?;
        
        // Deserialize inverted index
        let inverted_index: HashMap<Term, Vec<Posting>> = bincode::deserialize(&decompressed)?;
        
        Ok(IndexReader {
            segment_id,
            inverted_index,
        })
    }

    /// Get postings for a term
    pub fn get_postings(&self, term: &Term) -> Option<&Vec<Posting>> {
        self.inverted_index.get(term)
    }

    /// Check if term exists
    pub fn contains_term(&self, term: &Term) -> bool {
        self.inverted_index.contains_key(term)
    }

    /// Get all terms
    pub fn terms(&self) -> Vec<&Term> {
        self.inverted_index.keys().collect()
    }

    /// Get index statistics
    pub fn stats(&self) -> IndexStats {
        IndexStats {
            unique_terms: self.inverted_index.len(),
            total_postings: self.inverted_index.values().map(|v| v.len()).sum(),
        }
    }
}

pub struct IndexStats {
    pub unique_terms: usize,
    pub total_postings: usize,
}
