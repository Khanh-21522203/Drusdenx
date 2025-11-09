use std::collections::HashMap;
use serde::{Deserialize, Serialize};
use crate::core::error::{Error, ErrorKind, Result};
use crate::index::inverted::InvertedIndex;
use crate::query::ast::Query;
use crate::query::planner::LogicalPlan;

/// Sort order for query results
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SortOrder {
    Asc,   // Ascending: 0 → 9, A → Z
    Desc,  // Descending: 9 → 0, Z → A
}

/// Global index statistics for query planning
#[derive(Debug, Clone)]
pub struct IndexStatistics {
    pub total_docs: usize,
    pub total_terms: usize,
    pub avg_doc_length: f32,
    pub field_stats: HashMap<String, FieldStatistics>,
}

/// Per-field statistics
#[derive(Debug, Clone)]
pub struct FieldStatistics {
    pub total_docs: usize,
    pub avg_field_length: f32,
    pub unique_terms: usize,
}

impl IndexStatistics {
    /// Create statistics from an inverted index
    pub fn from_index(index: &InvertedIndex) -> Self {
        let total_docs = index.doc_count;
        let total_terms = index.dictionary.len();

        // Calculate average document length (sum of all term frequencies)
        let mut total_length = 0;
        for posting_list in index.postings.values() {
            if let Ok(postings) = posting_list.iter() {
                for posting in postings {
                    total_length += posting.term_freq;
                }
            }
        }
        let avg_doc_length = if total_docs > 0 {
            total_length as f32 / total_docs as f32
        } else {
            0.0
        };

        IndexStatistics {
            total_docs,
            total_terms,
            avg_doc_length,
            field_stats: HashMap::new(),
        }
    }
}

/// Cost model for query planning
#[derive(Debug, Clone)]
pub struct CostModel {
    pub scan_cost_per_doc: f32,
    pub seek_cost_per_term: f32,
    pub filter_cost_per_doc: f32,
    pub sort_cost_multiplier: f32,
}

impl Default for CostModel {
    fn default() -> Self {
        CostModel {
            scan_cost_per_doc: 1.0,
            seek_cost_per_term: 0.1,
            filter_cost_per_doc: 0.5,
            sort_cost_multiplier: 2.0,
        }
    }
}

impl CostModel {
    /// Estimate cost of a logical plan
    pub fn estimate_cost(&self, plan: &LogicalPlan, stats: &IndexStatistics) -> f32 {
        match plan {
            LogicalPlan::Scan { .. } => {
                self.scan_cost_per_doc * stats.total_docs as f32
            }
            LogicalPlan::IndexSeek { .. } => {
                self.seek_cost_per_term
            }
            LogicalPlan::Filter { input, .. } => {
                let input_cost = self.estimate_cost(input, stats);
                input_cost + (self.filter_cost_per_doc * stats.total_docs as f32)
            }
            LogicalPlan::Sort { input, .. } => {
                let input_cost = self.estimate_cost(input, stats);
                input_cost * self.sort_cost_multiplier
            }
            LogicalPlan::Limit { n, input } => {
                let input_cost = self.estimate_cost(input, stats);
                input_cost * (*n as f32 / stats.total_docs as f32)
            }
            _ => 1.0,
        }
    }
}

/// Query validation configuration
#[derive(Debug, Clone)]
pub struct ValidationConfig {
    pub max_bool_clauses: usize,
    pub max_query_depth: usize,
    pub max_wildcard_terms: usize,
    pub allow_leading_wildcard: bool,
}

impl Default for ValidationConfig {
    fn default() -> Self {
        ValidationConfig {
            max_bool_clauses: 1024,
            max_query_depth: 10,
            max_wildcard_terms: 1000,
            allow_leading_wildcard: false,
        }
    }
}

/// Query validator
pub struct QueryValidator {
    config: ValidationConfig,
    statistics: IndexStatistics,
}

impl QueryValidator {
    pub fn new(config: ValidationConfig, statistics: IndexStatistics) -> Self {
        QueryValidator { config, statistics }
    }

    /// Validate query structure and constraints
    pub fn validate(&self, query: &Query) -> Result<()> {
        self.validate_depth(query, 0)?;
        self.validate_bool_clauses(query)?;
        Ok(())
    }

    fn validate_depth(&self, query: &Query, depth: usize) -> Result<()> {
        if depth > self.config.max_query_depth {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                format!("Query depth {} exceeds maximum {}",
                        depth, self.config.max_query_depth)
            ));
        }

        match query {
            Query::Bool(bool_query) => {
                for q in &bool_query.must {
                    self.validate_depth(q, depth + 1)?;
                }
                for q in &bool_query.should {
                    self.validate_depth(q, depth + 1)?;
                }
                for q in &bool_query.must_not {
                    self.validate_depth(q, depth + 1)?;
                }
            }
            _ => {}
        }

        Ok(())
    }

    fn validate_bool_clauses(&self, query: &Query) -> Result<()> {
        if let Query::Bool(bool_query) = query {
            let total_clauses = bool_query.must.len()
                + bool_query.should.len()
                + bool_query.must_not.len();

            if total_clauses > self.config.max_bool_clauses {
                return Err(Error::new(
                    ErrorKind::InvalidInput,
                    format!("Boolean query has {} clauses, max is {}",
                            total_clauses, self.config.max_bool_clauses)
                ));
            }
        }
        Ok(())
    }
}