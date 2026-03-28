use std::sync::Arc;
use regex::Regex;
use crate::core::types::{Document, FieldValue};
use crate::query::ast::{Query, TermQuery, PhraseQuery, BoolQuery, RangeQuery, PrefixQuery, FuzzyQuery, WildcardQuery};
use crate::core::error::Result;
use crate::core::utils::levenshtein_distance;
use crate::index::inverted::{InvertedIndex, Term};
use crate::search::results::ScoredDocument;
use crate::storage::segment_reader::SegmentReader;
use crate::storage::segment::SegmentHeader;
use crate::query::visitor::QueryVisitor;

/// Document matcher - implements query matching logic
/// This is the search functionality that M02 didn't have
pub struct DocumentMatcher {
    // Configuration for matching
    index: Arc<InvertedIndex>,
}

/// Thin per-call context. Stack-allocated. Carries doc reference without
/// polluting visitor method signatures for variants that don't need it.
struct MatchContext<'a> {
    matcher: &'a DocumentMatcher,
    doc: &'a Document,
}

impl QueryVisitor for MatchContext<'_> {
    type Output = bool;

    fn visit_term(&self, q: &TermQuery) -> Result<bool> {
        let value = &q.value;

        if q.field == "_all" {
            return Ok(self.matcher.doc_contains_text(self.doc, value));
        }

        Ok(self.matcher.field_contains_text(self.doc, &q.field, value))
    }

    fn visit_phrase(&self, q: &PhraseQuery) -> Result<bool> {
        let _field = &q.field;
        let phrase = &q.phrase;
        let slop = q.slop;

        let mut term_positions: Vec<Vec<u32>> = Vec::new();

        for term_text in phrase {
            let term = Term::new(term_text);

            if let Some(posting_list) = self.matcher.index.search_term(&term) {
                let mut found = false;
                for posting in &posting_list.iter()? {
                    if posting.doc_id == self.doc.id {
                        term_positions.push(posting.positions.clone());
                        found = true;
                        break;
                    }
                }

                if !found {
                    return Ok(false);
                }
            } else {
                return Ok(false);
            }
        }

        if slop == 0 {
            Ok(self.matcher.check_adjacent_positions(&term_positions))
        } else {
            Ok(self.matcher.check_proximity_positions(&term_positions, slop))
        }
    }

    fn visit_bool(&self, q: &BoolQuery) -> Result<bool> {
        // Must clauses: all must match (AND)
        for must_clause in &q.must {
            if !must_clause.accept(self)? {
                return Ok(false);
            }
        }

        // Must not clauses: none must match (NOT)
        for must_not_clause in &q.must_not {
            if must_not_clause.accept(self)? {
                return Ok(false);
            }
        }

        // Should clauses: at least one must match (OR)
        if !q.should.is_empty() {
            let mut any_match = false;
            for should_clause in &q.should {
                if should_clause.accept(self)? {
                    any_match = true;
                    break;
                }
            }
            if !any_match {
                return Ok(false);
            }
        }

        // Filter clauses: must match but don't affect score
        for filter_clause in &q.filter {
            if !filter_clause.accept(self)? {
                return Ok(false);
            }
        }

        Ok(true)
    }

    fn visit_range(&self, q: &RangeQuery) -> Result<bool> {
        if let Some(field_value) = self.doc.fields.get(&q.field) {
            match field_value {
                FieldValue::Number(num) => {
                    Ok(self.matcher.number_in_range(*num, q))
                },
                FieldValue::Date(_date) => {
                    Ok(true) // Placeholder
                },
                _ => Ok(false),
            }
        } else {
            Ok(false)
        }
    }

    fn visit_prefix(&self, q: &PrefixQuery) -> Result<bool> {
        if let Some(field_value) = self.doc.fields.get(&q.field) {
            match field_value {
                FieldValue::Text(text) => {
                    let words: Vec<&str> = text.split_whitespace().collect();
                    for word in words {
                        if word.starts_with(&q.prefix) {
                            return Ok(true);
                        }
                    }
                }
                _ => return Ok(false),
            }
        }
        Ok(false)
    }

    fn visit_wildcard(&self, q: &WildcardQuery) -> Result<bool> {
        if let Some(field_value) = self.doc.fields.get(&q.field) {
            match field_value {
                FieldValue::Text(text) => {
                    let pattern = q.pattern
                        .replace("*", ".*")
                        .replace("?", ".");

                    if let Ok(regex) = Regex::new(&pattern) {
                        return Ok(regex.is_match(text));
                    }
                }
                _ => return Ok(false),
            }
        }
        Ok(false)
    }

    fn visit_fuzzy(&self, q: &FuzzyQuery) -> Result<bool> {
        if let Some(field_value) = self.doc.fields.get(&q.field) {
            match field_value {
                FieldValue::Text(text) => {
                    let max_edits = q.max_edits.unwrap_or(2) as usize;
                    let query_term_lower = q.term.to_lowercase();

                    let words: Vec<&str> = text
                        .split_whitespace()
                        .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()))
                        .filter(|w| !w.is_empty())
                        .collect();

                    for word in words {
                        let word_lower = word.to_lowercase();
                        let distance = levenshtein_distance(&query_term_lower, &word_lower);

                        if distance <= max_edits {
                            return Ok(true);
                        }
                    }

                    Ok(false)
                }
                _ => Ok(false),
            }
        } else {
            Ok(false)
        }
    }

    fn visit_match_all(&self) -> Result<bool> {
        Ok(true)
    }
}

impl DocumentMatcher {
    pub fn new(index: Arc<InvertedIndex>) -> Self {
        DocumentMatcher { index }
    }

    /// The public interface collapses to a one-liner.
    pub fn matches(&self, doc: &Document, query: &Query) -> Result<bool> {
        let ctx = MatchContext { matcher: self, doc };
        query.accept(&ctx)
    }

    /// Check if term positions are adjacent (exact phrase match)
    fn check_adjacent_positions(&self, term_positions: &[Vec<u32>]) -> bool {
        if term_positions.is_empty() {
            return false;
        }

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

        for start_pos in &term_positions[0] {
            let mut current_pos = *start_pos;
            let mut found = true;

            for positions in &term_positions[1..] {
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

    fn number_in_range(&self, num: f64, range_query: &RangeQuery) -> bool {
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
}

/// Extension trait to add search to SegmentReader (from M02)
pub trait SegmentSearch {
    fn search(&self, query: &Query, matcher: &DocumentMatcher) -> Result<Vec<ScoredDocument>>;
}

impl SegmentSearch for SegmentReader {
    /// Search documents in segment using query
    /// This is the search() method that M02 didn't have
    fn search(&self, query: &Query, matcher: &DocumentMatcher) -> Result<Vec<ScoredDocument>> {
        use std::io::{Read, Seek, SeekFrom};
        use crate::compression::compress::CompressedBlock;
        let mut results = Vec::new();

        // Use lazy iteration pattern directly here
        let mut file = self.file.lock().unwrap();

        // Seek to start of documents (read header size to skip it)
        file.seek(SeekFrom::Start(0))?;
        // Skip header by deserializing it (variable length)
        let _: SegmentHeader = bincode::deserialize_from(&mut *file)?;

        // Now positioned right after header, ready to read documents

        // Iterate through documents one by one
        for _ in 0..self.header.doc_count {
            // Read document length (serialized CompressedBlock size)
            let mut len_buf = [0u8; 4];
            if file.read_exact(&mut len_buf).is_err() {
                break; // EOF
            }
            let len = u32::from_le_bytes(len_buf) as usize;

            // Read serialized CompressedBlock
            let mut block_buf = vec![0u8; len];
            file.read_exact(&mut block_buf)?;

            // Deserialize CompressedBlock (includes original_size metadata)
            let compressed_block: CompressedBlock = bincode::deserialize(&block_buf)?;
            let decompressed = compressed_block.decompress()?;

            // Deserialize document
            let doc: Document = bincode::deserialize(&decompressed)?;

            // Apply query matching
            if matcher.matches(&doc, query)? {
                results.push(ScoredDocument {
                    doc_id: doc.id,
                    score: 1.0,  // Simple scoring for now
                    document: Some(doc),
                    explanation: None,
                });
            }
        }

        Ok(results)
    }
}
