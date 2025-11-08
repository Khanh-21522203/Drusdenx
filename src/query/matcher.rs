use std::sync::Arc;
use regex::Regex;
use crate::core::types::{Document, FieldValue};
use crate::query::ast::{Query, TermQuery, PhraseQuery, BoolQuery, RangeQuery, PrefixQuery, FuzzyQuery, WildcardQuery};
use crate::core::error::Result;
use crate::core::utils::levenshtein_distance;
use crate::index::inverted::{InvertedIndex, Term};
use crate::search::results::ScoredDocument;
use crate::storage::segment_reader::SegmentReader;

/// Document matcher - implements query matching logic
/// This is the search functionality that M02 didn't have
pub struct DocumentMatcher {
    // Configuration for matching
    index: Arc<InvertedIndex>,
}

impl DocumentMatcher {
    pub fn new(index: Arc<InvertedIndex>) -> Self {
        DocumentMatcher { index }
    }

    /// Check if document matches query
    /// This replaces the search() method that was removed from M02
    pub fn matches(&self, doc: &Document, query: &Query) -> Result<bool> {
        match query {
            Query::MatchAll => Ok(true),

            Query::Term(term_query) => {
                Ok(self.matches_term(doc, term_query))
            },

            Query::Phrase(phrase_query) => {
                Ok(self.matches_phrase(doc, phrase_query))
            },

            Query::Bool(bool_query) => {
                self.matches_bool(doc, bool_query)
            },

            Query::Range(range_query) => {
                Ok(self.matches_range(doc, range_query))
            },

            Query::Prefix(prefix_query) => {
                Ok(self.matches_prefix(doc, prefix_query))
            },

            Query::Wildcard(wildcard_query) => {
                Ok(self.matches_wildcard(doc, wildcard_query))
            },

            Query::Fuzzy(fuzzy_query) => {
                Ok(self.matches_fuzzy(doc, fuzzy_query))
            },
        }
    }

    /// Match term query - field-specific search
    fn matches_term(&self, doc: &Document, term_query: &TermQuery) -> bool {
        let field = &term_query.field;
        let value = &term_query.value;

        // Special field "_all" searches all text fields
        if field == "_all" {
            return self.doc_contains_text(doc, value);
        }

        // Field-specific search
        self.field_contains_text(doc, field, value)
    }

    /// Match phrase query - proximity search with slop
    fn matches_phrase(&self, doc: &Document, phrase_query: &PhraseQuery) -> bool {
        let field = &phrase_query.field;
        let phrase = &phrase_query.phrase;
        let slop = phrase_query.slop;

        // Get term positions from inverted index (M04)
        // InvertedIndex stores positions for each term in each document
        let mut term_positions: Vec<Vec<u32>> = Vec::new();

        for term_text in phrase {
            // Create Term from text
            let term = Term::new(term_text);

            // Query index for posting list of this term
            if let Some(posting_list) = self.index.search_term(&term) {
                // Find posting for this specific document
                let mut found = false;
                for posting in &posting_list.postings {
                    if posting.doc_id == doc.id {
                        term_positions.push(posting.positions.clone());
                        found = true;
                        break;
                    }
                }

                if !found {
                    // Term not found in this document
                    return false;
                }
            } else {
                // Term not in index at all
                return false;
            }
        }

        // Check if positions satisfy phrase constraint
        if slop == 0 {
            // Exact phrase: terms must be adjacent
            // E.g., "hello world" requires pos(world) == pos(hello) + 1
            self.check_adjacent_positions(&term_positions)
        } else {
            // Proximity match: allow gaps up to slop
            // E.g., "hello world"~2 allows up to 2 words between hello and world
            self.check_proximity_positions(&term_positions, slop)
        }
    }

    /// Check if term positions are adjacent (exact phrase match)
    fn check_adjacent_positions(&self, term_positions: &[Vec<u32>]) -> bool {
        if term_positions.is_empty() {
            return false;
        }

        // Try all combinations of positions
        // For "hello world", check if any pos(hello) + 1 == pos(world)
        for start_pos in &term_positions[0] {
            let mut current_pos = *start_pos;
            let mut found = true;

            for positions in &term_positions[1..] {
                current_pos += 1;
                if !positions.contains(&current_pos) {
                    found = false;
                    break;
                }
            }

            if found {
                return true;
            }
        }

        false
    }

    /// Check if term positions satisfy proximity constraint with slop
    fn check_proximity_positions(&self, term_positions: &[Vec<u32>], slop: u32) -> bool {
        if term_positions.is_empty() {
            return false;
        }

        // Try all combinations of positions
        // For "hello world"~2, allow distance <= 2 between terms
        for start_pos in &term_positions[0] {
            let mut current_pos = *start_pos;
            let mut found = true;

            for positions in &term_positions[1..] {
                // Find nearest position within slop distance
                let min_pos = current_pos + 1;
                let max_pos = current_pos + slop + 1;

                if let Some(&next_pos) = positions.iter()
                    .find(|&&p| p >= min_pos && p <= max_pos)
                {
                    current_pos = next_pos;
                } else {
                    found = false;
                    break;
                }
            }

            if found {
                return true;
            }
        }

        false
    }

    /// Match boolean query
    fn matches_bool(&self, doc: &Document, bool_query: &BoolQuery) -> Result<bool> {
        // Must clauses: all must match (AND)
        for must_clause in &bool_query.must {
            if !self.matches(doc, must_clause)? {
                return Ok(false);
            }
        }

        // Must not clauses: none must match (NOT)
        for must_not_clause in &bool_query.must_not {
            if self.matches(doc, must_not_clause)? {
                return Ok(false);
            }
        }

        // Should clauses: at least one must match (OR)
        if !bool_query.should.is_empty() {
            let mut any_match = false;
            for should_clause in &bool_query.should {
                if self.matches(doc, should_clause)? {
                    any_match = true;
                    break;
                }
            }
            if !any_match {
                return Ok(false);
            }
        }

        // Filter clauses: must match but don't affect score
        for filter_clause in &bool_query.filter {
            if !self.matches(doc, filter_clause)? {
                return Ok(false);
            }
        }

        Ok(true)
    }

    /// Match range query
    fn matches_range(&self, doc: &Document, range_query: &RangeQuery) -> bool {
        if let Some(field_value) = doc.fields.get(&range_query.field) {
            match field_value {
                FieldValue::Number(num) => {
                    self.number_in_range(*num, range_query)
                },
                FieldValue::Date(date) => {
                    // Date range matching
                    // Implementation depends on range_query having date values
                    true  // Placeholder
                },
                _ => false,
            }
        } else {
            false
        }
    }

    fn number_in_range(&self, num: f64, range_query: &RangeQuery) -> bool {
        // Check gt/gte
        if let Some(FieldValue::Number(gt)) = &range_query.gt {
            if num <= *gt {
                return false;
            }
        }
        if let Some(FieldValue::Number(gte)) = &range_query.gte {
            if num < *gte {
                return false;
            }
        }

        // Check lt/lte
        if let Some(FieldValue::Number(lt)) = &range_query.lt {
            if num >= *lt {
                return false;
            }
        }
        if let Some(FieldValue::Number(lte)) = &range_query.lte {
            if num > *lte {
                return false;
            }
        }

        true
    }

    /// Check if specific field contains text (case-insensitive)
    fn field_contains_text(&self, doc: &Document, field: &str, text: &str) -> bool {
        if let Some(field_value) = doc.fields.get(field) {
            if let FieldValue::Text(s) = field_value {
                return s.to_lowercase().contains(&text.to_lowercase());
            }
        }
        false
    }

    /// Check if any text field contains text (case-insensitive)
    fn doc_contains_text(&self, doc: &Document, text: &str) -> bool {
        let text_lower = text.to_lowercase();

        for (_field_name, field_value) in &doc.fields {
            if let FieldValue::Text(s) = field_value {
                if s.to_lowercase().contains(&text_lower) {
                    return true;
                }
            }
        }

        false
    }

    fn matches_prefix(&self, doc: &Document, query: &PrefixQuery) -> bool {
        if let Some(field_value) = doc.fields.get(&query.field) {
            match field_value {
                FieldValue::Text(text) => {
                    // Check if text starts with prefix
                    // For multi-word fields, check if any word starts with prefix
                    let words: Vec<&str> = text.split_whitespace().collect();
                    for word in words {
                        if word.starts_with(&query.prefix) {
                            return true;
                        }
                    }
                }
                _ => return false,
            }
        }
        false
    }

    fn matches_wildcard(&self, doc: &Document, query: &WildcardQuery) -> bool {
        // Get field value from document
        if let Some(field_value) = doc.fields.get(&query.field) {
            match field_value {
                FieldValue::Text(text) => {
                    // Convert wildcard pattern to regex
                    // * -> .*, ? -> .
                    let pattern = query.pattern
                        .replace("*", ".*")
                        .replace("?", ".");

                    if let Ok(regex) = Regex::new(&pattern) {
                        return regex.is_match(text);
                    }
                }
                _ => return false,
            }
        }
        false
    }

    fn matches_fuzzy(&self, doc: &Document, query: &FuzzyQuery) -> bool {
        // Get field value from document
        if let Some(field_value) = doc.fields.get(&query.field) {
            return match field_value {
                FieldValue::Text(text) => {
                    // Calculate Levenshtein distance
                    let max_edits = query.max_edits;
                    let distance = levenshtein_distance(&query.term, text);
                    distance <= max_edits.unwrap() as usize
                }
                _ => false,
            }
        }
        false
    }

}

/// Extension trait to add search to SegmentReader (from M02)
pub trait SegmentSearch {
    fn search(&mut self, query: &Query, matcher: &DocumentMatcher) -> Result<Vec<ScoredDocument>>;
}

impl SegmentSearch for SegmentReader {
    /// Search documents in segment using query
    /// This is the search() method that M02 didn't have
    fn search(&mut self, query: &Query, matcher: &DocumentMatcher) -> Result<Vec<ScoredDocument>> {
        let mut results = Vec::new();

        // Use M02's read_all_documents()
        let docs = self.read_all_documents()?;

        for doc in docs {
            // Apply query matching (M05's logic)
            if matcher.matches(&doc, query)? {
                results.push(ScoredDocument {
                    doc_id: doc.id,
                    score: 1.0,  // Simple scoring for now, real scoring uses M04's BM25
                    document: Some(doc),
                    explanation: None,
                });
            }
        }

        Ok(results)
    }
}