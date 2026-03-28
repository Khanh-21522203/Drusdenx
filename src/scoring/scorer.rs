use crate::index::inverted::TermInfo;
use crate::index::posting::Posting;

/// Read-only view passed to scorers — decouples them from InvertedIndex internals.
pub struct ScoringContext<'a> {
    pub doc_id: crate::core::types::DocId,
    pub posting: &'a Posting,
    pub term_info: &'a TermInfo,
    pub doc_stats: DocStats,
    pub query_boost: f32,
}

/// Scorer trait
pub trait Scorer: Send + Sync {
    /// Score using the new ScoringContext API.
    fn score_ctx(&self, ctx: &ScoringContext<'_>) -> f32;

    /// Score using the legacy API (posting + term_info + doc_stats).
    /// Default implementation delegates to score_ctx.
    fn score(&self, posting: &Posting, term_info: &TermInfo, doc_stats: &DocStats) -> f32 {
        let ctx = ScoringContext {
            doc_id: posting.doc_id,
            posting,
            term_info,
            doc_stats: doc_stats.clone(),
            query_boost: 1.0,
        };
        self.score_ctx(&ctx)
    }

    fn name(&self) -> &str;

    fn requires_positions(&self) -> bool {
        false
    }

    /// Optional SIMD-friendly batch path. Default impl calls score_ctx() per element.
    fn score_batch(&self, contexts: &[ScoringContext<'_>], out: &mut [f32]) {
        for (ctx, slot) in contexts.iter().zip(out.iter_mut()) {
            *slot = self.score_ctx(ctx);
        }
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
    fn score_ctx(&self, ctx: &ScoringContext<'_>) -> f32 {
        // TF = term frequency / document length (if normalized)
        let tf = if self.normalize {
            ctx.posting.term_freq as f32 / ctx.doc_stats.doc_length as f32
        } else {
            ctx.posting.term_freq as f32
        };

        // TF-IDF = TF * IDF
        tf * ctx.term_info.idf * ctx.query_boost
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
    fn score_ctx(&self, ctx: &ScoringContext<'_>) -> f32 {
        let tf = ctx.posting.term_freq as f32;
        let doc_len = ctx.doc_stats.doc_length as f32;
        let avg_doc_len = ctx.doc_stats.avg_doc_length;

        // BM25 formula
        let numerator = ctx.term_info.idf * tf * (self.k1 + 1.0);
        let denominator = tf + self.k1 * (1.0 - self.b + self.b * (doc_len / avg_doc_len));

        (numerator / denominator) * ctx.query_boost
    }

    fn name(&self) -> &str {
        "bm25"
    }
}
