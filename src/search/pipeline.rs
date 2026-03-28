use crate::core::error::{Error, ErrorKind, Result};
use crate::index::inverted::{InvertedIndex, Term};
use crate::query::ast::{Query, TermQuery};
use crate::query::matcher::{DocumentMatcher, SegmentSearch};
use crate::query::optimizer::QueryOptimizer;
use crate::query::planner::{LogicalPlan, QueryPlanner};
use crate::query::types::{IndexStatistics, QueryValidator, ValidationConfig};
use crate::reader::reader_pool::IndexReader;
use crate::scoring::scorer::{BM25Scorer, DocStats, Scorer, ScoringContext};
use crate::search::collector::{CollectDecision, Collector, IntoResults, MatchedDocument};
use crate::search::results::{ScoreExplanation, TopKCollector};
use std::sync::Arc;

/// Pipeline configuration (mirrors `ExecutionConfig` for the pipeline API).
pub struct PipelineConfig {
    pub enable_optimization: bool,
    pub enable_validation: bool,
    pub collect_explanations: bool,
    pub timeout_ms: Option<u64>,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        PipelineConfig {
            enable_optimization: true,
            enable_validation: true,
            collect_explanations: false,
            timeout_ms: Some(30000),
        }
    }
}

/// Generic search pipeline parameterized over scorer and collector.
///
/// Zero-overhead monomorphized path: no dynamic dispatch for the hot loop.
pub struct SearchPipeline<S, C>
where
    S: Scorer,
    C: Collector + IntoResults,
{
    pub index: Arc<InvertedIndex>,
    pub optimizer: QueryOptimizer,
    pub validator_config: ValidationConfig,
    pub config: PipelineConfig,
    pub scorer: S,
    pub collector: C,
}

impl<S: Scorer, C: Collector + IntoResults> SearchPipeline<S, C> {
    pub fn execute(&mut self, reader: &IndexReader, query: &Query) -> Result<C::Output> {
        let _ = (reader, query);
        Err(Error::new(
            ErrorKind::InvalidState,
            "SearchPipeline::execute(&mut self, ...) is unavailable; use SearchPipeline::run(self, ...)".to_string(),
        ))
    }
}

impl<S: Scorer, C: Collector + IntoResults> SearchPipeline<S, C> {
    /// Execute and consume the pipeline, returning final results.
    pub fn run(mut self, reader: &IndexReader, query: &Query) -> Result<C::Output> {
        // 1. Validate
        if self.config.enable_validation {
            let stats = IndexStatistics::from_index(&reader.index);
            let validator = QueryValidator::new(self.validator_config.clone(), stats);
            validator.validate(query)?;
        }

        // 2. Optimize
        let optimized_query = if self.config.enable_optimization {
            optimize_query(query, &reader.index, &self.optimizer)?
        } else {
            query.clone()
        };

        // 3. Execute on segments
        let matcher = DocumentMatcher::new(reader.index.clone());
        let stats = IndexStatistics::from_index(&reader.index);

        'segments: for segment_reader in &reader.segments {
            let seg = segment_reader.read();
            let matches = seg.search(&optimized_query, &matcher)?;

            for doc in matches {
                if reader.deleted_docs.contains(doc.doc_id.0 as u32) {
                    continue;
                }

                let score = calculate_score_with(
                    doc.doc_id,
                    &optimized_query,
                    &reader.index,
                    &self.scorer,
                    &stats,
                )?;

                let explanation = if self.config.collect_explanations {
                    Some(ScoreExplanation {
                        value: score,
                        description: format!("score for doc {}", doc.doc_id.0),
                        details: Vec::new(),
                    })
                } else {
                    None
                };

                let matched = MatchedDocument {
                    doc_id: doc.doc_id,
                    score,
                    document: doc.document,
                    explanation,
                };

                match self.collector.collect(matched) {
                    CollectDecision::Continue => {}
                    CollectDecision::Terminate => break 'segments,
                }
            }
        }

        self.collector.finish();

        Ok(self.collector.into_results())
    }
}

/// Builder for `SearchPipeline`.
pub struct PipelineBuilder {
    pub index: Arc<InvertedIndex>,
    pub config: PipelineConfig,
}

impl PipelineBuilder {
    pub fn new(index: Arc<InvertedIndex>) -> Self {
        PipelineBuilder {
            index,
            config: PipelineConfig::default(),
        }
    }

    pub fn config(mut self, config: PipelineConfig) -> Self {
        self.config = config;
        self
    }

    /// Zero-overhead monomorphized path for common case.
    pub fn build_default(self, limit: usize) -> SearchPipeline<BM25Scorer, TopKCollector> {
        SearchPipeline {
            index: self.index,
            optimizer: QueryOptimizer::new(),
            validator_config: ValidationConfig::default(),
            config: self.config,
            scorer: BM25Scorer::default(),
            collector: TopKCollector::new(limit),
        }
    }

    /// Generic path for custom scorers and collectors.
    pub fn build<S, C>(self, scorer: S, collector: C) -> SearchPipeline<S, C>
    where
        S: Scorer,
        C: Collector + IntoResults,
    {
        SearchPipeline {
            index: self.index,
            optimizer: QueryOptimizer::new(),
            validator_config: ValidationConfig::default(),
            config: self.config,
            scorer,
            collector,
        }
    }
}

// --- Helpers ---

fn optimize_query(
    query: &Query,
    index: &InvertedIndex,
    optimizer: &QueryOptimizer,
) -> Result<Query> {
    if !is_safe_to_optimize(query) {
        return Ok(query.clone());
    }

    let stats = IndexStatistics::from_index(index);
    let planner = QueryPlanner::new(stats);
    let plan = planner.plan(query);
    let optimized_plan = optimizer.optimize(plan);

    match plan_to_query(optimized_plan) {
        Ok(optimized_query) => Ok(optimized_query),
        Err(_) => Ok(query.clone()),
    }
}

fn plan_to_query(plan: LogicalPlan) -> Result<Query> {
    use crate::query::ast::BoolQuery;
    match plan {
        LogicalPlan::IndexSeek { field, term } => Ok(Query::Term(TermQuery {
            field,
            value: term,
            boost: None,
        })),
        LogicalPlan::Filter { predicate, .. } => Ok(predicate),
        LogicalPlan::Union { inputs } => {
            let should = inputs
                .into_iter()
                .map(plan_to_query)
                .collect::<Result<Vec<_>>>()?;
            Ok(Query::Bool(BoolQuery {
                must: vec![],
                should,
                must_not: vec![],
                filter: vec![],
                boost: None,
                minimum_should_match: Some(1),
            }))
        }
        LogicalPlan::Intersection { inputs } => {
            let must = inputs
                .into_iter()
                .map(plan_to_query)
                .collect::<Result<Vec<_>>>()?;
            Ok(Query::Bool(BoolQuery {
                must,
                should: vec![],
                must_not: vec![],
                filter: vec![],
                boost: None,
                minimum_should_match: None,
            }))
        }
        LogicalPlan::Difference { left, right } => {
            let left_query = plan_to_query(*left)?;
            let right_query = plan_to_query(*right)?;
            Ok(Query::Bool(BoolQuery {
                must: vec![left_query],
                should: vec![],
                must_not: vec![right_query],
                filter: vec![],
                boost: None,
                minimum_should_match: None,
            }))
        }
        LogicalPlan::Limit { input, .. } => plan_to_query(*input),
        LogicalPlan::Sort { input, .. } => plan_to_query(*input),
        LogicalPlan::Scan { .. } => Err(Error::new(
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
                && bool_query.must.iter().all(is_safe_to_optimize)
                && bool_query.should.iter().all(is_safe_to_optimize)
        }
        Query::MatchAll => true,
        Query::Phrase(_)
        | Query::Range(_)
        | Query::Prefix(_)
        | Query::Wildcard(_)
        | Query::Fuzzy(_) => false,
    }
}

fn calculate_score_with<S: Scorer>(
    doc_id: crate::core::types::DocId,
    query: &Query,
    index: &InvertedIndex,
    scorer: &S,
    stats: &IndexStatistics,
) -> Result<f32> {
    match query {
        Query::Term(tq) => {
            let term = Term::new(&tq.value);
            if let Some(posting_list) = index.search_term(&term) {
                if let Some(term_info) = index.dictionary.get_term_info(&term) {
                    for posting in &posting_list.iter()? {
                        if posting.doc_id == doc_id {
                            let doc_stats = DocStats {
                                doc_length: posting.positions.len(),
                                avg_doc_length: stats.avg_doc_length,
                                total_docs: stats.total_docs,
                            };
                            let ctx = ScoringContext {
                                doc_id,
                                posting: &posting,
                                term_info,
                                doc_stats,
                                query_boost: tq.boost.unwrap_or(1.0),
                            };
                            return Ok(scorer.score_ctx(&ctx));
                        }
                    }
                }
            }
            Ok(0.0)
        }
        Query::Bool(bq) => {
            let mut total = 0.0;
            for must in &bq.must {
                total += calculate_score_with(doc_id, must, index, scorer, stats)?;
            }
            for should in &bq.should {
                total += calculate_score_with(doc_id, should, index, scorer, stats)?;
            }
            Ok(total * bq.boost.unwrap_or(1.0))
        }
        _ => Ok(1.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mvcc::controller::Snapshot;
    use roaring::RoaringBitmap;

    #[test]
    fn execute_returns_error_instead_of_panicking() {
        let index = Arc::new(InvertedIndex::new());
        let mut pipeline = PipelineBuilder::new(index.clone()).build_default(10);
        let reader = IndexReader {
            snapshot: Arc::new(Snapshot::default()),
            segments: Vec::new(),
            deleted_docs: Arc::new(RoaringBitmap::new()),
            index,
        };

        let result = pipeline.execute(&reader, &Query::MatchAll);
        assert!(result.is_err());
    }

    #[test]
    fn optimize_query_preserves_non_term_variants() {
        let index = InvertedIndex::new();
        let optimizer = QueryOptimizer::new();

        let wildcard = Query::Wildcard(crate::query::ast::WildcardQuery {
            field: "title".to_string(),
            pattern: "pre*".to_string(),
            boost: None,
        });

        let optimized = optimize_query(&wildcard, &index, &optimizer).unwrap();
        assert!(matches!(optimized, Query::Wildcard(_)));
    }
}
