use crate::core::error::{Error, ErrorKind, Result};
use crate::core::types::DocId;
use crate::index::inverted::{InvertedIndex, Term};
use crate::query::ast::{BoolQuery, Query, TermQuery};
use crate::query::matcher::{DocumentMatcher, SegmentSearch};
use crate::query::optimizer::QueryOptimizer;
use crate::query::planner::{LogicalPlan, QueryPlanner};
use crate::query::types::{IndexStatistics, QueryValidator, ValidationConfig};
use crate::reader::reader_pool::IndexReader;
use crate::scoring::scorer::{BM25Scorer, DocStats, Scorer, TfIdfScorer};
use crate::search::results::{ScoreExplanation, ScoredDocument, SearchResults, TopKCollector};

/// Scoring algorithm selection
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScoringAlgorithm {
    BM25,
    TfIdf,
    None, // Simple scoring (1.0 for all matches)
}

/// Query execution configuration
#[derive(Debug, Clone)]
pub struct ExecutionConfig {
    pub scoring: ScoringAlgorithm,
    pub enable_optimization: bool,
    pub enable_validation: bool,
    pub collect_explanations: bool,
    pub timeout_ms: Option<u64>,
}

impl Default for ExecutionConfig {
    fn default() -> Self {
        ExecutionConfig {
            scoring: ScoringAlgorithm::BM25, // BM25 by default
            enable_optimization: true,
            enable_validation: true,
            collect_explanations: false,
            timeout_ms: Some(30000), // 30 seconds default
        }
    }
}

impl ExecutionConfig {
    /// Create a simple config for fast execution without optimization
    pub fn simple() -> Self {
        ExecutionConfig {
            scoring: ScoringAlgorithm::None, // No scoring for speed
            enable_optimization: false,
            enable_validation: false,
            collect_explanations: false,
            timeout_ms: Some(10000),
        }
    }

    /// Create a debug config with explanations
    pub fn debug() -> Self {
        ExecutionConfig {
            scoring: ScoringAlgorithm::BM25,
            enable_optimization: true,
            enable_validation: true,
            collect_explanations: true,
            timeout_ms: None,
        }
    }

    /// Create config with specific scoring algorithm
    pub fn with_scoring(algorithm: ScoringAlgorithm) -> Self {
        ExecutionConfig {
            scoring: algorithm,
            ..Default::default()
        }
    }

    /// Create BM25 config
    pub fn bm25() -> Self {
        Self::with_scoring(ScoringAlgorithm::BM25)
    }

    /// Create TF-IDF config
    pub fn tfidf() -> Self {
        Self::with_scoring(ScoringAlgorithm::TfIdf)
    }
}

// No need for SimpleScorer - when scoring is disabled, we use the score from DocumentMatcher

/// Query executor service (stateless)
///
/// This executor does NOT own any data or cache. It operates on provided IndexReader instances.
///
/// # Example
/// ```rust,no_run
/// # use Drusdenx::search::executor::{QueryExecutor, ExecutionConfig};
/// let executor = QueryExecutor::new();
/// // let reader = reader_pool.get_reader()?;
/// // let query = parser.parse("rust programming")?;
/// // let results = executor.execute(&reader, &query, 10, ExecutionConfig::default())?;
/// ```
pub struct QueryExecutor {
    pub optimizer: QueryOptimizer,
    pub validator_config: ValidationConfig,
}

impl QueryExecutor {
    /// Create a new query executor
    pub fn new() -> Self {
        QueryExecutor {
            optimizer: QueryOptimizer::new(),
            validator_config: ValidationConfig::default(),
        }
    }

    /// Create with custom configuration
    pub fn with_config(validator_config: ValidationConfig) -> Self {
        QueryExecutor {
            optimizer: QueryOptimizer::new(),
            validator_config,
        }
    }

    /// Execute a query on the provided IndexReader
    ///
    /// # Arguments
    /// * `reader` - The IndexReader containing segments and index
    /// * `query` - The parsed query to execute
    /// * `limit` - Maximum number of results to return
    /// * `config` - Execution configuration
    ///
    /// # Returns
    /// * `SearchResults` containing matched documents with scores
    pub fn execute(
        &self,
        reader: &IndexReader,
        query: &Query,
        limit: usize,
        config: ExecutionConfig,
    ) -> Result<SearchResults> {
        let start = std::time::Instant::now();

        // 1. Validate query if enabled
        if config.enable_validation {
            let stats = IndexStatistics::from_index(&reader.index);
            let validator = QueryValidator::new(self.validator_config.clone(), stats);
            validator.validate(query)?;
        }

        // 2. Optimize query if enabled
        let optimized_query = if config.enable_optimization {
            self.optimize_query(query, &reader.index)?
        } else {
            query.clone()
        };

        // 3. Create collector for top-K results
        let mut collector = TopKCollector::new(limit);

        // 4. Execute on reader's segments
        self.execute_on_segments(reader, &optimized_query, &mut collector, &config)?;

        // 5. Build final results
        let total_hits = collector.total_collected;
        let max_score = collector.max_score();
        let hits = collector.get_results(); // This consumes collector, must be last

        Ok(SearchResults {
            hits,
            total_hits,
            max_score,
            took_ms: start.elapsed().as_millis() as u64,
        })
    }

    /// Execute query with simple configuration (convenience method)
    pub fn execute_simple(
        &self,
        reader: &IndexReader,
        query: &Query,
        limit: usize,
    ) -> Result<SearchResults> {
        self.execute(reader, query, limit, ExecutionConfig::simple())
    }

    /// Optimize a query based on index statistics
    fn optimize_query(&self, query: &Query, index: &InvertedIndex) -> Result<Query> {
        if !Self::is_safe_to_optimize(query) {
            return Ok(query.clone());
        }

        // Create planner with current index statistics
        let stats = IndexStatistics::from_index(index);
        let planner = QueryPlanner::new(stats);

        // Generate logical plan
        let plan = planner.plan(query);

        // Optimize the plan
        let optimized_plan = self.optimizer.optimize(plan);

        // Convert plan back to query. If we cannot preserve semantics,
        // keep the original query unchanged.
        match self.plan_to_query(optimized_plan) {
            Ok(optimized_query) => Ok(optimized_query),
            Err(_) => Ok(query.clone()),
        }
    }

    /// Convert logical plan back to Query AST for execution
    fn plan_to_query(&self, plan: LogicalPlan) -> Result<Query> {
        match plan {
            LogicalPlan::IndexSeek { field, term } => {
                // Convert index seek to term query
                Ok(Query::Term(TermQuery {
                    field,
                    value: term,
                    boost: None,
                }))
            }

            LogicalPlan::Filter { predicate, .. } => {
                // Filter predicates are already queries
                Ok(predicate)
            }

            LogicalPlan::Union { inputs } => {
                // Convert union to boolean should query
                let mut should_clauses = Vec::new();
                for input in inputs {
                    should_clauses.push(self.plan_to_query(input)?);
                }

                Ok(Query::Bool(BoolQuery {
                    must: vec![],
                    should: should_clauses,
                    must_not: vec![],
                    filter: vec![],
                    boost: None,
                    minimum_should_match: Some(1),
                }))
            }

            LogicalPlan::Intersection { inputs } => {
                // Convert intersection to boolean must query
                let mut must_clauses = Vec::new();
                for input in inputs {
                    must_clauses.push(self.plan_to_query(input)?);
                }

                Ok(Query::Bool(BoolQuery {
                    must: must_clauses,
                    should: vec![],
                    must_not: vec![],
                    filter: vec![],
                    boost: None,
                    minimum_should_match: None,
                }))
            }

            LogicalPlan::Difference { left, right } => {
                // Convert difference to must with must_not
                let left_query = self.plan_to_query(*left)?;
                let right_query = self.plan_to_query(*right)?;

                Ok(Query::Bool(BoolQuery {
                    must: vec![left_query],
                    should: vec![],
                    must_not: vec![right_query],
                    filter: vec![],
                    boost: None,
                    minimum_should_match: None,
                }))
            }

            LogicalPlan::Limit { input, .. } => {
                // Limit doesn't affect query structure, just execution
                self.plan_to_query(*input)
            }

            LogicalPlan::Sort { input, .. } => {
                // Sort doesn't affect query structure, just result ordering
                self.plan_to_query(*input)
            }

            LogicalPlan::Scan { field: _ } => Err(Error::new(
                ErrorKind::InvalidState,
                "Cannot safely convert LogicalPlan::Scan back into a specific query".to_string(),
            )),
        }
    }

    fn is_safe_to_optimize(query: &Query) -> bool {
        match query {
            Query::Term(_) => true,
            Query::Bool(bool_query) => {
                bool_query.must_not.is_empty()
                    && bool_query.filter.is_empty()
                    && bool_query.must.iter().all(Self::is_safe_to_optimize)
                    && bool_query.should.iter().all(Self::is_safe_to_optimize)
            }
            Query::MatchAll => true,
            Query::Phrase(_)
            | Query::Range(_)
            | Query::Prefix(_)
            | Query::Wildcard(_)
            | Query::Fuzzy(_) => false,
        }
    }

    /// Execute query on IndexReader's segments with configurable scoring
    fn execute_on_segments(
        &self,
        reader: &IndexReader,
        query: &Query,
        collector: &mut TopKCollector,
        config: &ExecutionConfig,
    ) -> Result<()> {
        // Get index statistics for scoring
        let stats = IndexStatistics::from_index(&reader.index);

        // Create document matcher for query evaluation (filtering)
        let matcher = DocumentMatcher::new(reader.index.clone());

        // Process each segment
        for segment_reader in &reader.segments {
            // Get READ lock on segment reader for concurrent reads
            let seg_reader = segment_reader.read();

            // Get matched documents (for filtering)
            let matches = seg_reader.search(query, &matcher)?;

            // Process matched documents
            for doc in matches {
                // Skip deleted documents
                if reader.deleted_docs.contains(doc.doc_id.0 as u32) {
                    continue;
                }

                // Calculate score based on selected algorithm
                let final_score = match config.scoring {
                    ScoringAlgorithm::BM25 => {
                        let scorer = BM25Scorer::default();
                        self.calculate_score(doc.doc_id, query, &reader.index, &scorer, &stats)?
                    }
                    ScoringAlgorithm::TfIdf => {
                        let scorer = TfIdfScorer::new(true); // normalized TF-IDF
                        self.calculate_score(doc.doc_id, query, &reader.index, &scorer, &stats)?
                    }
                    ScoringAlgorithm::None => {
                        1.0 // Simple scoring
                    }
                };

                let scored_doc = ScoredDocument {
                    doc_id: doc.doc_id,
                    score: final_score,
                    document: doc.document,
                    explanation: if config.collect_explanations {
                        Some(self.generate_score_explanation(
                            doc.doc_id,
                            query,
                            final_score,
                            &reader.index,
                        )?)
                    } else {
                        None
                    },
                };

                // Collect result
                collector.collect(scored_doc);
            }
        }

        Ok(())
    }

    /// Calculate score for a document given a query and scorer
    fn calculate_score<S: Scorer>(
        &self,
        doc_id: DocId,
        query: &Query,
        index: &InvertedIndex,
        scorer: &S,
        stats: &IndexStatistics,
    ) -> Result<f32> {
        match query {
            Query::Term(term_query) => {
                self.score_term_query(doc_id, term_query, index, scorer, stats)
            }
            Query::Bool(bool_query) => {
                self.score_bool_query(doc_id, bool_query, index, scorer, stats)
            }
            Query::Phrase(_phrase_query) => {
                // For phrase queries, use simple scoring for now
                // Proper phrase scoring would require position-aware scoring
                Ok(1.0)
            }
            _ => Ok(1.0), // Other query types use simple scoring
        }
    }

    /// Score a single term query using the provided scorer
    fn score_term_query<S: Scorer>(
        &self,
        doc_id: DocId,
        term_query: &TermQuery,
        index: &InvertedIndex,
        scorer: &S,
        stats: &IndexStatistics,
    ) -> Result<f32> {
        let term = Term::new(&term_query.value);

        // Get posting list for term
        if let Some(posting_list) = index.search_term(&term) {
            // Get term info for IDF
            if let Some(term_info) = index.dictionary.get_term_info(&term) {
                // Find posting for this document
                for posting in &posting_list.iter()? {
                    if posting.doc_id == doc_id {
                        // Calculate doc stats
                        let doc_stats = DocStats {
                            doc_length: posting.positions.len(),
                            avg_doc_length: stats.avg_doc_length,
                            total_docs: stats.total_docs,
                        };

                        // Calculate BM25 score
                        let score = scorer.score(&posting, term_info, &doc_stats);
                        return Ok(score * term_query.boost.unwrap_or(1.0));
                    }
                }
            }
        }

        Ok(0.0) // Term not found in document
    }

    /// Score a boolean query (sum of term scores)
    fn score_bool_query<S: Scorer>(
        &self,
        doc_id: DocId,
        bool_query: &BoolQuery,
        index: &InvertedIndex,
        scorer: &S,
        stats: &IndexStatistics,
    ) -> Result<f32> {
        let mut total_score = 0.0;

        // Score must clauses
        for must_clause in &bool_query.must {
            total_score += self.calculate_score(doc_id, must_clause, index, scorer, stats)?;
        }

        // Score should clauses
        for should_clause in &bool_query.should {
            total_score += self.calculate_score(doc_id, should_clause, index, scorer, stats)?;
        }

        // Apply boost
        Ok(total_score * bool_query.boost.unwrap_or(1.0))
    }

    /// Generate detailed score explanation
    fn generate_score_explanation(
        &self,
        doc_id: DocId,
        query: &Query,
        score: f32,
        index: &InvertedIndex,
    ) -> Result<ScoreExplanation> {
        let mut details = Vec::new();

        match query {
            Query::Term(tq) => {
                let term = Term::new(&tq.value);
                if let Some(term_info) = index.dictionary.get_term_info(&term) {
                    details.push(ScoreExplanation {
                        value: term_info.idf,
                        description: format!("IDF for term '{}'", tq.value),
                        details: Vec::new(),
                    });
                }
            }
            _ => {}
        }

        Ok(ScoreExplanation {
            value: score,
            description: format!("BM25 score for document {}", doc_id.0),
            details,
        })
    }
}

impl Default for QueryExecutor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::types::FieldValue;
    use crate::query::ast::{FuzzyQuery, PhraseQuery, RangeQuery, WildcardQuery};

    #[test]
    fn test_execution_config() {
        // Default uses BM25
        let config = ExecutionConfig::default();
        assert_eq!(config.scoring, ScoringAlgorithm::BM25);
        assert!(config.enable_optimization);
        assert!(config.enable_validation);
        assert!(!config.collect_explanations);

        // Simple uses no scoring
        let simple = ExecutionConfig::simple();
        assert_eq!(simple.scoring, ScoringAlgorithm::None);
        assert!(!simple.enable_optimization);
        assert!(!simple.enable_validation);

        // Debug uses BM25 with explanations
        let debug = ExecutionConfig::debug();
        assert_eq!(debug.scoring, ScoringAlgorithm::BM25);
        assert!(debug.collect_explanations);

        // TF-IDF config
        let tfidf = ExecutionConfig::tfidf();
        assert_eq!(tfidf.scoring, ScoringAlgorithm::TfIdf);

        // BM25 config
        let bm25 = ExecutionConfig::bm25();
        assert_eq!(bm25.scoring, ScoringAlgorithm::BM25);
    }

    #[test]
    fn optimize_query_preserves_non_term_query_semantics() {
        let executor = QueryExecutor::new();
        let index = InvertedIndex::new();

        let phrase = Query::Phrase(PhraseQuery {
            field: "content".to_string(),
            phrase: vec!["rust".to_string(), "book".to_string()],
            slop: 0,
            boost: None,
        });
        let wildcard = Query::Wildcard(WildcardQuery {
            field: "title".to_string(),
            pattern: "ru*".to_string(),
            boost: None,
        });
        let fuzzy = Query::Fuzzy(FuzzyQuery {
            field: "title".to_string(),
            term: "rust".to_string(),
            max_edits: Some(1),
            prefix_length: None,
            boost: None,
        });
        let range = Query::Range(RangeQuery {
            field: "price".to_string(),
            gt: None,
            gte: Some(FieldValue::Number(10.0)),
            lt: None,
            lte: Some(FieldValue::Number(20.0)),
            boost: None,
        });

        let optimized_phrase = executor.optimize_query(&phrase, &index).unwrap();
        let optimized_wildcard = executor.optimize_query(&wildcard, &index).unwrap();
        let optimized_fuzzy = executor.optimize_query(&fuzzy, &index).unwrap();
        let optimized_range = executor.optimize_query(&range, &index).unwrap();

        assert!(matches!(optimized_phrase, Query::Phrase(_)));
        assert!(matches!(optimized_wildcard, Query::Wildcard(_)));
        assert!(matches!(optimized_fuzzy, Query::Fuzzy(_)));
        assert!(matches!(optimized_range, Query::Range(_)));
    }

    #[test]
    fn optimize_query_keeps_bool_with_must_not() {
        let executor = QueryExecutor::new();
        let index = InvertedIndex::new();

        let bool_query = Query::Bool(BoolQuery {
            must: vec![],
            should: vec![Query::Term(TermQuery {
                field: "content".to_string(),
                value: "foo".to_string(),
                boost: None,
            })],
            must_not: vec![Query::Term(TermQuery {
                field: "content".to_string(),
                value: "bar".to_string(),
                boost: None,
            })],
            filter: vec![],
            minimum_should_match: Some(1),
            boost: None,
        });

        let optimized = executor.optimize_query(&bool_query, &index).unwrap();
        let Query::Bool(q) = optimized else {
            panic!("expected bool query");
        };

        assert_eq!(q.must_not.len(), 1);
        match &q.must_not[0] {
            Query::Term(term) => assert_eq!(term.value, "bar"),
            _ => panic!("expected term in must_not"),
        }
    }
}
