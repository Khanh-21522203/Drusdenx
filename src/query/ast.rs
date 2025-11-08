use serde::{Serialize, Deserialize};
use chrono::{DateTime, Utc};
use crate::core::types::FieldValue;
use crate::index::inverted::InvertedIndex;
use crate::search::fuzzy::FuzzyAutomaton;
use crate::search::prefix::PrefixIndex;

/// Main query enum representing all query types
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Query {
    Term(TermQuery),         // Single term search
    Phrase(PhraseQuery),     // Exact phrase match
    Bool(BoolQuery),         // Boolean combinations
    Range(RangeQuery),       // Numeric/date range
    Prefix(PrefixQuery),
    Wildcard(WildcardQuery), // Pattern matching (defined in M07)
    Fuzzy(FuzzyQuery),       // Typo tolerance (defined in M07)
    MatchAll,                // Match all documents
}

/// Single term query
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TermQuery {
    pub field: String,
    pub value: String,
    pub boost: Option<f32>,
}

/// Phrase query for exact phrase matching
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PhraseQuery {
    pub field: String,
    pub phrase: Vec<String>,
    pub slop: u32,  // Max distance between terms
    pub boost: Option<f32>,
}

/// Boolean query with must/should/must_not clauses
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BoolQuery {
    pub must: Vec<Query>,      // All must match (AND)
    pub should: Vec<Query>,    // At least one must match (OR)
    pub must_not: Vec<Query>,  // None must match (NOT)
    pub filter: Vec<Query>,    // Must match but don't affect score
    pub minimum_should_match: Option<u32>,
    pub boost: Option<f32>,
}

/// Range query for numeric and date fields
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RangeQuery {
    pub field: String,
    pub gt: Option<FieldValue>,   // Greater than
    pub gte: Option<FieldValue>,  // Greater than or equal
    pub lt: Option<FieldValue>,   // Less than
    pub lte: Option<FieldValue>,  // Less than or equal
    pub boost: Option<f32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PrefixQuery {
    pub field: String,
    pub prefix: String,
    pub boost: Option<f32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WildcardQuery {
    pub field: String,
    pub pattern: String, //Pattern with wildcards (* and ?)
    pub boost: Option<f32>,
}


/// Fuzzy query implementation
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FuzzyQuery {
    pub field: String,
    pub term: String,
    pub max_edits: Option<u8>,      // Default: 2 (Levenshtein distance)
    pub prefix_length: Option<u8>,  // Default: 0 (no prefix lock)
    pub boost: Option<f32>,
}

impl BoolQuery {
    pub fn new() -> Self {
        BoolQuery {
            must: Vec::new(),
            should: Vec::new(),
            must_not: Vec::new(),
            filter: Vec::new(),
            minimum_should_match: None,
            boost: None,
        }
    }

    pub fn with_must(mut self, query: Query) -> Self {
        self.must.push(query);
        self
    }

    pub fn with_should(mut self, query: Query) -> Self {
        self.should.push(query);
        self
    }

    pub fn with_must_not(mut self, query: Query) -> Self {
        self.must_not.push(query);
        self
    }

    pub fn with_filter(mut self, query: Query) -> Self {
        self.filter.push(query);
        self
    }
}

impl Default for BoolQuery {
    fn default() -> Self {
        Self::new()
    }
}