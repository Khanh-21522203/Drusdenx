use std::collections::BinaryHeap;
use std::cmp::Ordering;
use crate::core::types::{DocId, Document};
use crate::search::collector::{Collector, CollectDecision, IntoResults, MatchedDocument};

/// Search results container
#[derive(Debug, Clone)]
pub struct SearchResults {
    pub hits: Vec<ScoredDocument>,
    pub total_hits: usize,
    pub max_score: f32,
    pub took_ms: u64,
}

/// Document with relevance score
#[derive(Debug, Clone)]
pub struct ScoredDocument {
    pub doc_id: DocId,
    pub score: f32,
    pub document: Option<Document>,  // Optionally include full document
    pub explanation: Option<ScoreExplanation>,
}

// Implement ordering for heap
impl PartialEq for ScoredDocument {
    fn eq(&self, other: &Self) -> bool {
        self.score == other.score
    }
}

impl Eq for ScoredDocument {}

impl PartialOrd for ScoredDocument {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        // Reverse order for max-heap
        other.score.partial_cmp(&self.score)
    }
}

impl Ord for ScoredDocument {
    fn cmp(&self, other: &Self) -> Ordering {
        self.partial_cmp(other).unwrap_or(Ordering::Equal)
    }
}

/// Score explanation for debugging
#[derive(Debug, Clone)]
pub struct ScoreExplanation {
    pub value: f32,
    pub description: String,
    pub details: Vec<ScoreExplanation>,
}

/// Top-K collector for efficient result collection
pub struct TopKCollector {
    pub heap: BinaryHeap<ScoredDocument>,
    pub k: usize,
    pub min_score: f32,
    pub total_collected: usize,  // Track total documents processed
}

impl TopKCollector {
    pub fn new(k: usize) -> Self {
        TopKCollector {
            heap: BinaryHeap::with_capacity(k + 1),
            k,
            min_score: 0.0,
            total_collected: 0,
        }
    }

    pub fn collect(&mut self, scored_doc: ScoredDocument) {
        self.total_collected += 1;  // Increment count

        if scored_doc.score > self.min_score || self.heap.len() < self.k {
            self.heap.push(scored_doc);

            if self.heap.len() > self.k {
                self.heap.pop();
                if let Some(min_doc) = self.heap.peek() {
                    self.min_score = min_doc.score;
                }
            }
        }
    }

    pub fn get_results(self) -> Vec<ScoredDocument> {
        let mut results: Vec<_> = self.heap.into_iter().collect();
        results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap());
        results
    }

    pub fn max_score(&self) -> f32 {
        self.heap.peek().map(|doc| doc.score).unwrap_or(0.0)
    }
}

// Implement the new Collector trait for TopKCollector
impl Collector for TopKCollector {
    fn collect(&mut self, doc: MatchedDocument) -> CollectDecision {
        let scored_doc = ScoredDocument {
            doc_id: doc.doc_id,
            score: doc.score,
            document: doc.document,
            explanation: doc.explanation,
        };
        self.total_collected += 1;

        if scored_doc.score > self.min_score || self.heap.len() < self.k {
            self.heap.push(scored_doc);

            if self.heap.len() > self.k {
                self.heap.pop();
                if let Some(min_doc) = self.heap.peek() {
                    self.min_score = min_doc.score;
                }
            }
        }

        CollectDecision::Continue
    }
}

impl IntoResults for TopKCollector {
    type Output = SearchResults;

    fn into_results(self) -> SearchResults {
        let total_hits = self.total_collected;
        let max_score = self.heap.peek().map(|d| d.score).unwrap_or(0.0);
        let hits = {
            let mut results: Vec<_> = self.heap.into_iter().collect();
            results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap());
            results
        };

        SearchResults {
            hits,
            total_hits,
            max_score,
            took_ms: 0,
        }
    }
}
