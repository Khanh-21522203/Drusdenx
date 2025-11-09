use crate::compression::compress::{EncodedIntegerBlock, IntegerEncodingType};
use crate::core::types::DocId;
use crate::core::error::Result;

#[derive(Debug, Clone)]
pub struct Posting {
    pub doc_id: DocId,
    pub term_freq: u32,       // Term frequency in document
    pub positions: Vec<u32>,  // Token positions for phrase queries
    pub field_norm: f32,      // Length normalization factor
}

/// Posting list for a term
/// Note: Sorted by doc_id for efficient merging
pub struct PostingList {
    pub doc_ids: EncodedIntegerBlock,
    pub term_freqs: Vec<u32>,
    pub positions: Vec<EncodedIntegerBlock>,
}

impl PostingList {
    pub fn new(postings: Vec<Posting>) -> Result<Self> {
        // Extract sorted doc IDs
        let doc_ids: Vec<u32> = postings.iter().map(|p| p.doc_id.0 as u32).collect();

        // Delta ENCODING (best for sorted integers)
        let encoded_ids = EncodedIntegerBlock::encode(
            &doc_ids,
            IntegerEncodingType::Delta  // Exploits sorted property
        )?;

        // VByte ENCODING for positions (small integers)
        let mut positions = Vec::new();
        for posting in &postings {
            let encoded = EncodedIntegerBlock::encode(
                &posting.positions,
                IntegerEncodingType::VByte  // Exploits small values
            )?;
            positions.push(encoded);
        }

        Ok(PostingList {
            doc_ids: encoded_ids,
            term_freqs: postings.iter().map(|p| p.term_freq).collect(),
            positions,
        })
    }

    pub fn decode_doc_ids(&self) -> Result<Vec<u32>> {
        self.doc_ids.decode()
    }

    pub fn get_posting(&self, index: usize) -> Result<Posting> {
        let doc_ids = self.doc_ids.decode()?;
        let positions = self.positions[index].decode()?;

        Ok(Posting {
            doc_id: DocId(doc_ids[index] as u64),
            term_freq: self.term_freqs[index],
            positions,
            field_norm: 1.0,
        })
    }

    /// Number of documents containing this term (document frequency)
    pub fn doc_freq(&self) -> u32 {
        self.term_freqs.len() as u32
    }

    /// Total occurrences across all documents (term frequency)
    pub fn total_freq(&self) -> u64 {
        self.term_freqs.iter().map(|&f| f as u64).sum()
    }

    /// Check if posting list is empty
    pub fn is_empty(&self) -> bool {
        self.term_freqs.is_empty()
    }

    /// Number of postings
    pub fn len(&self) -> usize {
        self.term_freqs.len()
    }

    /// Iterate over all postings (decodes on-demand)
    /// ⚠️ Expensive: Decodes all data. Use sparingly!
    pub fn iter(&self) -> Result<Vec<Posting>> {
        let doc_ids = self.doc_ids.decode()?;
        let mut postings = Vec::with_capacity(self.len());

        for i in 0..self.len() {
            let positions = self.positions[i].decode()?;
            postings.push(Posting {
                doc_id: DocId(doc_ids[i] as u64),
                term_freq: self.term_freqs[i],
                positions,
                field_norm: 1.0,
            });
        }

        Ok(postings)
    }

    /// Get doc ID at index without full decode
    /// More efficient than get_posting() if you only need doc ID
    pub fn get_doc_id(&self, index: usize) -> Result<DocId> {
        let doc_ids = self.doc_ids.decode()?;
        Ok(DocId(doc_ids[index] as u64))
    }

    /// Binary search for document ID (requires decode)
    pub fn find_doc(&self, target: DocId) -> Result<Option<usize>> {
        let doc_ids = self.doc_ids.decode()?;
        let target_u32 = target.0 as u32;

        Ok(doc_ids.binary_search(&target_u32).ok())
    }
}