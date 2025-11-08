use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use parking_lot::RwLock;
use crate::core::types::{DocId, Document, FieldValue};
use crate::index::inverted::{InvertedIndex, Term};
use crate::query::cache::{QueryCache, QueryKey};
use crate::query::optimizer::QueryOptimizer;
use crate::query::parser::QueryParser;
use crate::query::planner::{LogicalPlan, QueryPlanner};
use crate::query::types::{IndexStatistics, QueryValidator, ValidationConfig};
use crate::scoring::scorer::{BM25Scorer, Scorer};
use crate::search::results::{ScoredDocument, SearchResults, TopKCollector};
use crate::core::error::{Error, Result};
use crate::core::error::ErrorKind::UnsupportedQuery;
use crate::query::ast::{BoolQuery, FuzzyQuery, PhraseQuery, PrefixQuery, Query, RangeQuery, TermQuery, WildcardQuery};

/// Execute queries with optimization and caching
pub struct QueryExecutor {
    pub index: Arc<InvertedIndex>,
    pub documents: Arc<RwLock<HashMap<DocId, Document>>>,
    pub planner: QueryPlanner,
    pub optimizer: QueryOptimizer,
    pub cache: QueryCache,
    pub scorer: Box<dyn Scorer>,
}

impl QueryExecutor {
    pub fn new(index: Arc<InvertedIndex>, documents: Arc<RwLock<HashMap<DocId, Document>>>,) -> Self {
        let statistics = IndexStatistics::from_index(&index);

        QueryExecutor {
            index: index.clone(),
            documents,
            planner: QueryPlanner::new(statistics.clone()),
            optimizer: QueryOptimizer::new(),
            cache: QueryCache::new(1000),
            scorer: Box::new(BM25Scorer::default()),
        }
    }

    pub fn execute(&self, query_str: &str, limit: usize) -> Result<SearchResults> {
        // Check cache first
        let key = QueryKey {
            query: query_str.to_string(),
            limit,
            offset: 0,
        };

        if let Some(results) = self.cache.get(&key) {
            return Ok(results);
        }

        // Parse query
        let parser = QueryParser::new();
        let query = parser.parse(query_str)?;

        // Validate query
        let validator = QueryValidator::new(ValidationConfig::default(),
                                            self.planner.statistics.clone());
        validator.validate(&query)?;

        // Plan and optimize
        let plan = self.planner.plan(&query);
        let optimized_plan = self.optimizer.optimize(plan);

        // Execute plan
        let results = self.execute_plan(&optimized_plan, limit)?;

        // Cache results
        self.cache.put(key, results.clone());

        Ok(results)
    }

    pub fn execute_query(&self, query: &Query, limit: usize) -> Result<SearchResults> {
        let start = std::time::Instant::now();
        let mut collector = TopKCollector::new(limit);

        self.execute_query_internal(query, &mut collector)?;

        let total_collected = collector.total_collected;
        let max_score = collector.max_score();
        let hits = collector.get_results();

        Ok(SearchResults {
            hits,
            total_hits: total_collected,
            max_score,
            took_ms: start.elapsed().as_millis() as u64,
        })
    }


    fn execute_query_internal(&self, query: &Query, collector: &mut TopKCollector) -> Result<()> {
        match query {
            Query::MatchAll => self.execute_match_all(collector)?,
            Query::Term(tq) => self.execute_term_internal(tq, collector)?,
            Query::Phrase(pq) => self.execute_phrase_internal(pq, collector)?,
            Query::Bool(bq) => self.execute_bool_internal(bq, collector)?,
            Query::Range(rq) => self.execute_range_internal(rq, collector)?,
            Query::Prefix(pq) => self.execute_prefix(pq, collector)?,
            Query::Wildcard(wq) => self.execute_wildcard(wq, collector)?,
            Query::Fuzzy(fq) => self.execute_fuzzy(fq, collector)?,
        }
        Ok(())
    }

    fn execute_match_all(&self, collector: &mut TopKCollector) -> Result<()> {
        // Match all documents
        for doc_id in 0..self.index.doc_count {
            collector.collect(ScoredDocument {
                doc_id: DocId(doc_id as u64),
                score: 1.0,
                document: None,
                explanation: None,
            });
        }
        Ok(())
    }

    fn execute_term_internal(&self, query: &TermQuery, collector: &mut TopKCollector) -> Result<()> {
        // Single term search
        let term = Term::new(&query.value);
        if let Some(posting_list) = self.index.search_term(&term) {
            for posting in &posting_list.postings {
                let score = (posting.term_freq as f32) * query.boost.unwrap_or(1.0);
                collector.collect(ScoredDocument {
                    doc_id: posting.doc_id,
                    score,
                    document: None,
                    explanation: None,
                });
            }
        }
        Ok(())
    }

    fn execute_phrase_internal(&self, query: &PhraseQuery, collector: &mut TopKCollector) -> Result<()> {
        // Phrase search - check positions are adjacent
        // (Simplified - see M05 DocumentMatcher for full implementation)
        let terms: Vec<Term> = query.phrase.iter().map(|s| Term::new(s)).collect();

        // Find documents where all terms appear
        let mut candidates: Option<HashSet<DocId>> = None;

        for term in &terms {
            if let Some(posting_list) = self.index.search_term(term) {
                let term_docs: HashSet<DocId> = posting_list.postings.iter()
                    .map(|p| p.doc_id)
                    .collect();

                candidates = match candidates {
                    None => Some(term_docs),
                    Some(docs) => Some(docs.intersection(&term_docs).copied().collect()),
                };
            } else {
                // If any term not found, no results
                return Ok(());
            }
        }

        // For each candidate, verify positions are adjacent (with slop tolerance)
        if let Some(docs) = candidates {
            for doc_id in docs {
                // Check if positions match with slop tolerance
                // Simplified: Just collect with fixed score
                // Full implementation would verify position constraints
                collector.collect(ScoredDocument {
                    doc_id,
                    score: query.boost.unwrap_or(1.0),
                    document: None,
                    explanation: None,
                });
            }
        }

        Ok(())
    }

    fn execute_bool_internal(&self, query: &BoolQuery, collector: &mut TopKCollector) -> Result<()> {
        // Boolean query - combine must/should/must_not
        let mut must_docs = HashSet::new();
        let mut should_docs = HashSet::new();
        let mut must_not_docs = HashSet::new();

        // Execute must clauses (AND)
        for clause in &query.must {
            let results = self.execute_query(clause, usize::MAX)?;
            let docs: HashSet<DocId> = results.hits.iter().map(|h| h.doc_id).collect();
            if must_docs.is_empty() {
                must_docs = docs;
            } else {
                must_docs.retain(|id| docs.contains(id));
            }
        }

        // Execute should clauses (OR)
        for clause in &query.should {
            let results = self.execute_query(clause, usize::MAX)?;
            should_docs.extend(results.hits.iter().map(|h| h.doc_id));
        }

        // Execute must_not clauses (NOT)
        for clause in &query.must_not {
            let results = self.execute_query(clause, usize::MAX)?;
            must_not_docs.extend(results.hits.iter().map(|h| h.doc_id));
        }

        // Combine: (must AND (should OR empty)) NOT must_not
        let final_docs: HashSet<DocId> = if !must_docs.is_empty() {
            must_docs.iter().copied().filter(|id| !must_not_docs.contains(id)).collect()
        } else if !should_docs.is_empty() {
            should_docs.iter().copied().filter(|id| !must_not_docs.contains(id)).collect()
        } else {
            HashSet::new()
        };

        for doc_id in final_docs {
            collector.collect(ScoredDocument {
                doc_id,
                score: query.boost.unwrap_or(1.0),
                document: None,
                explanation: None,
            });
        }

        Ok(())
    }

    fn execute_range_internal(&self, query: &RangeQuery, collector: &mut TopKCollector) -> Result<()> {
        // Range query on numeric/date fields
        // Scan all documents and check if field values match range bounds

        let docs = self.documents.read();

        for doc_id in 0..self.index.doc_count {
            let doc_id_obj = DocId(doc_id as u64);

            // Get document from storage
            if let Some(doc) = docs.get(&doc_id_obj) {
                // Get field value
                if let Some(field_value) = doc.fields.get(&query.field) {
                    // Check if value is within range
                    if self.matches_range_bounds(field_value, query) {
                        collector.collect(ScoredDocument {
                            doc_id: doc_id_obj,
                            score: query.boost.unwrap_or(1.0),
                            document: None,
                            explanation: None,
                        });
                    }
                }
            }
        }

        Ok(())
    }

    fn matches_range_bounds(&self, value: &FieldValue, query: &RangeQuery) -> bool {
        match value {
            FieldValue::Number(num) => {
                // Check gt (greater than)
                if let Some(FieldValue::Number(gt)) = &query.gt {
                    if num <= gt {
                        return false;
                    }
                }

                // Check gte (greater than or equal)
                if let Some(FieldValue::Number(gte)) = &query.gte {
                    if num < gte {
                        return false;
                    }
                }

                // Check lt (less than)
                if let Some(FieldValue::Number(lt)) = &query.lt {
                    if num >= lt {
                        return false;
                    }
                }

                // Check lte (less than or equal)
                if let Some(FieldValue::Number(lte)) = &query.lte {
                    if num > lte {
                        return false;
                    }
                }

                true
            }
            FieldValue::Date(date) => {
                // Check gt (greater than)
                if let Some(FieldValue::Date(gt)) = &query.gt {
                    if date <= gt {
                        return false;
                    }
                }

                // Check gte (greater than or equal)
                if let Some(FieldValue::Date(gte)) = &query.gte {
                    if date < gte {
                        return false;
                    }
                }

                // Check lt (less than)
                if let Some(FieldValue::Date(lt)) = &query.lt {
                    if date >= lt {
                        return false;
                    }
                }

                // Check lte (less than or equal)
                if let Some(FieldValue::Date(lte)) = &query.lte {
                    if date > lte {
                        return false;
                    }
                }

                true
            }
            FieldValue::Text(_) => {
                // Text fields don't support range queries
                false
            }
            _ => false,
        }
    }
    fn execute_prefix(&self, query: &PrefixQuery, collector: &mut TopKCollector) -> Result<()> {
        // Prefix search using FST (fast autocomplete)
        // Example: "hel" matches "hello", "help", "helicopter"

        // Use InvertedIndex.prefix_search() to find matching terms
        let matching_terms = self.index.prefix_search(&query.prefix)?;

        let mut seen_docs = HashSet::new();
        for term_str in matching_terms {
            let term = Term::new(&term_str);
            if let Some(posting_list) = self.index.search_term(&term) {
                for posting in &posting_list.postings {
                    if seen_docs.insert(posting.doc_id) {
                        let score = (posting.term_freq as f32) * query.boost.unwrap_or(1.0);
                        collector.collect(ScoredDocument {
                            doc_id: posting.doc_id,
                            score,
                            document: None,
                            explanation: None,
                        });
                    }
                }
            }
        }
        Ok(())
    }

    fn execute_wildcard(&self, query: &WildcardQuery, collector: &mut TopKCollector) -> Result<()> {
        // Wildcard supports: * (any chars), ? (single char)
        // Example: "hel*" matches "hello", "help", "helicopter"
        // Example: "h?llo" matches "hello", "hallo"

        // Convert wildcard pattern to regex or FST automaton
        let matching_terms = self.index.wildcard_search(&query.pattern)?;

        let mut seen_docs = HashSet::new();
        for term_str in matching_terms {
            let term = Term::new(&term_str);
            if let Some(posting_list) = self.index.search_term(&term) {
                for posting in &posting_list.postings {
                    if seen_docs.insert(posting.doc_id) {
                        let score = (posting.term_freq as f32) * query.boost.unwrap_or(1.0);
                        collector.collect(ScoredDocument {
                            doc_id: posting.doc_id,
                            score,
                            document: None,
                            explanation: None,
                        });
                    }
                }
            }
        }
        Ok(())
    }

    fn execute_fuzzy(&self, query: &FuzzyQuery, collector: &mut TopKCollector) -> Result<()> {
        // Fuzzy search using Levenshtein distance
        // Example: "hello" with max_edits=1 matches "hallo", "hullo", "hell"

        // Use Levenshtein automaton to find terms within edit distance
        let matching_terms = self.index.fuzzy_search(
            &query.term,
            query.max_edits.unwrap_or(2),
            query.prefix_length.unwrap_or(0),
        )?;

        let mut seen_docs = HashSet::new();
        for (term_str, edit_distance) in matching_terms {
            let term = Term::new(&term_str);
            if let Some(posting_list) = self.index.search_term(&term) {
                for posting in &posting_list.postings {
                    if seen_docs.insert(posting.doc_id) {
                        // Score decreases with edit distance
                        let distance_penalty = 1.0 - (edit_distance as f32 * 0.2);
                        let score = (posting.term_freq as f32) * distance_penalty * query.boost.unwrap_or(1.0);

                        collector.collect(ScoredDocument {
                            doc_id: posting.doc_id,
                            score,
                            document: None,
                            explanation: None,
                        });
                    }
                }
            }
        }
        Ok(())
    }

    fn execute_plan(&self, plan: &LogicalPlan, limit: usize) -> Result<SearchResults> {
        let start = std::time::Instant::now();
        let mut collector = TopKCollector::new(limit);

        match plan {
            LogicalPlan::IndexSeek { field, term } => {
                // Get posting list from index
                let term_obj = Term::new(term);
                if let Some(posting_list) = self.index.search_term(&term_obj) {
                    for posting in &posting_list.postings {
                        // Score and collect
                        let score = 1.0; // Simplified scoring
                        collector.collect(ScoredDocument {
                            doc_id: posting.doc_id,
                            score,
                            document: None,
                            explanation: None,
                        });
                    }
                }
            }
            LogicalPlan::Scan { .. } => {
                // Full scan implementation
                for doc_id in 0..self.index.doc_count {
                    collector.collect(ScoredDocument {
                        doc_id: DocId(doc_id as u64),
                        score: 0.5,
                        document: None,
                        explanation: None,
                    });
                }
            }
            LogicalPlan::Intersection { inputs } => {
                // Execute each input and intersect results
                let mut result_sets: Vec<HashSet<DocId>> = Vec::new();

                for input in inputs {
                    let results = self.execute_plan(input, usize::MAX)?;
                    let doc_ids: HashSet<DocId> = results.hits
                        .into_iter()
                        .map(|h| h.doc_id)
                        .collect();
                    result_sets.push(doc_ids);
                }

                // Find intersection
                if let Some(first) = result_sets.first() {
                    let mut intersection = first.clone();
                    for set in result_sets.iter().skip(1) {
                        intersection.retain(|id| set.contains(id));
                    }

                    for doc_id in intersection {
                        collector.collect(ScoredDocument {
                            doc_id,
                            score: 1.0,
                            document: None,
                            explanation: None,
                        });
                    }
                }
            }
            _ => {
                // Other plan types...
            }
        }

        let total_collected = collector.total_collected;
        let max_score = collector.max_score();
        let hits = collector.get_results();

        Ok(SearchResults {
            hits,
            total_hits: total_collected,
            max_score,
            took_ms: start.elapsed().as_millis() as u64,
        })
    }
}