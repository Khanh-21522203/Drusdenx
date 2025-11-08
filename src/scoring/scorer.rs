use crate::index::inverted::TermInfo;
use crate::index::posting::Posting;

/// Scorer trait
pub trait Scorer: Send + Sync {
    fn score(&self, posting: &Posting, term_info: &TermInfo, doc_stats: &DocStats) -> f32;

    fn name(&self) -> &str;

    fn requires_positions(&self) -> bool {
        false
    }
}

/// Document statistics for scoring
#[derive(Debug, Clone)]
pub struct DocStats {
    pub doc_length: usize,    // Number of tokens in document
    pub avg_doc_length: f32,  // Average document length in collection
    pub total_docs: usize,    // Total number of documents
}

/// TF-IDF Scorer
pub struct TfIdfScorer {
    pub normalize: bool,
}

impl TfIdfScorer {
    pub fn new(normalize: bool) -> Self {
        TfIdfScorer { normalize }
    }
}

impl Scorer for TfIdfScorer {
    fn score(&self, posting: &Posting, term_info: &TermInfo, doc_stats: &DocStats) -> f32 {
        // TF = term frequency / document length (if normalized)
        let tf = if self.normalize {
            posting.term_freq as f32 / doc_stats.doc_length as f32
        } else {
            posting.term_freq as f32
        };

        // TF-IDF = TF * IDF
        tf * term_info.idf
    }

    fn name(&self) -> &str {
        "tfidf"
    }
}

/// BM25 Scorer
pub struct BM25Scorer {
    pub k1: f32,  // Term frequency saturation (default: 1.2)
    pub b: f32,   // Length normalization strength (default: 0.75)
}

impl Default for BM25Scorer {
    fn default() -> Self {
        BM25Scorer {
            k1: 1.2,
            b: 0.75,
        }
    }
}

impl Scorer for BM25Scorer {
    fn score(&self, posting: &Posting, term_info: &TermInfo, doc_stats: &DocStats) -> f32 {
        let tf = posting.term_freq as f32;
        let doc_len = doc_stats.doc_length as f32;
        let avg_doc_len = doc_stats.avg_doc_length;

        // BM25 formula
        let numerator = term_info.idf * tf * (self.k1 + 1.0);
        let denominator = tf + self.k1 * (1.0 - self.b + self.b * (doc_len / avg_doc_len));

        numerator / denominator
    }

    fn name(&self) -> &str {
        "bm25"
    }
}