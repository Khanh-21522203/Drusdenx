use crate::core::types::{DocId, Document};
use crate::search::results::ScoreExplanation;

/// A document matched by a query, ready to be collected.
pub struct MatchedDocument {
    pub doc_id: DocId,
    pub score: f32,
    pub document: Option<Document>,
    pub explanation: Option<ScoreExplanation>,
}

/// Signal returned by a collector's `collect` method.
pub enum CollectDecision {
    /// Continue scanning more documents.
    Continue,
    /// Stop scanning — enough documents have been collected.
    Terminate,
}

/// Trait for collecting matched documents during query execution.
pub trait Collector: Send {
    fn collect(&mut self, doc: MatchedDocument) -> CollectDecision;
    fn finish(&mut self) {}
}

/// Trait to convert a collector's internal state into a final result type.
pub trait IntoResults {
    type Output;
    fn into_results(self) -> Self::Output;
}
