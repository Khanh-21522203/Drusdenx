use crate::core::types::DocId;

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
    pub postings: Vec<Posting>,  // Sorted by doc_id
    // Note: SkipList optimization will be added in M09
}

impl PostingList {
    pub fn new() -> Self {
        PostingList {
            postings: Vec::new(),
        }
    }

    pub fn add_posting(&mut self, posting: Posting) {
        // Keep sorted by doc_id for efficient merging
        match self.postings.binary_search_by_key(&posting.doc_id.0, |p| p.doc_id.0) {
            Ok(pos) => {
                // Update existing posting
                self.postings[pos] = posting;
            }
            Err(pos) => {
                // Insert new posting
                self.postings.insert(pos, posting);
            }
        }
        // Note: Postings are kept sorted for efficient intersection
    }

    pub fn len(&self) -> usize {
        self.postings.len()
    }

    pub fn is_empty(&self) -> bool {
        self.postings.is_empty()
    }

    pub fn doc_freq(&self) -> u32 {
        self.postings.len() as u32
    }

    pub fn total_freq(&self) -> u64 {
        self.postings.iter().map(|p| p.term_freq as u64).sum()
    }

    /// Intersect two posting lists (simple linear merge)
    /// Note: SkipList optimization will be added in M09 for O(√n) performance
    pub fn intersect(&self, other: &PostingList) -> Vec<Posting> {
        let mut result = Vec::new();
        let mut i = 0;
        let mut j = 0;

        while i < self.postings.len() && j < other.postings.len() {
            let doc_id1 = self.postings[i].doc_id.0;
            let doc_id2 = other.postings[j].doc_id.0;

            if doc_id1 == doc_id2 {
                result.push(self.postings[i].clone());
                i += 1;
                j += 1;
            } else if doc_id1 < doc_id2 {
                i += 1;
            } else {
                j += 1;
            }
        }

        result
    }
}