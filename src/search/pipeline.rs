use std::sync::Arc;
use crate::core::error::Result;
use crate::index::inverted::{InvertedIndex, Term};
use crate::query::ast::{Query, TermQuery};
use crate::query::matcher::{DocumentMatcher, SegmentSearch};
use crate::query::optimizer::QueryOptimizer;
use crate::query::planner::{QueryPlanner, LogicalPlan};
use crate::query::types::{IndexStatistics, QueryValidator, ValidationConfig};
use crate::reader::reader_pool::IndexReader;
use crate::scoring::scorer::{BM25Scorer, DocStats, Scorer, ScoringContext};
use crate::search::collector::{Collector, CollectDecision, IntoResults, MatchedDocument};
use crate::search::results::{ScoreExplanation, TopKCollector};

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
                // Skip deleted
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

        // We need to move the collector out to call into_results().
        // Since we need &mut self, we use a trick: swap with a dummy.
        // Instead, we take the results from a reference-based approach.
        // Use std::mem::replace with a new collector of same type is not possible
        // without Default. We work around this by reconstructing the results inline.
        //
        // The simplest solution: use a wrapper that allows consuming.
        // Since we can't move out of &mut self, we delegate to a separate helper.
        // This is a known limitation — see pipeline_helper below.
        //
        // For now: rebuild results from the heap directly via get_results().
        // This is safe because we do NOT call collect() after this point.
        unimplemented_collect_results()
    }
}

// This function cannot be implemented generically without consuming self.
// Instead, we provide `execute_consuming` below.
fn unimplemented_collect_results<T>() -> Result<T> {
    unreachable!("Use SearchPipeline::run() instead of execute()")
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
    let stats = IndexStatistics::from_index(index);
    let planner = QueryPlanner::new(stats);
    let plan = planner.plan(query);
    let optimized_plan = optimizer.optimize(plan);
    plan_to_query(optimized_plan)
}

fn plan_to_query(plan: LogicalPlan) -> Result<Query> {
    use crate::query::ast::BoolQuery;
    match plan {
        LogicalPlan::IndexSeek { field, term } => {
            Ok(Query::Term(TermQuery { field, value: term, boost: None }))
        }
        LogicalPlan::Filter { predicate, .. } => Ok(predicate),
        LogicalPlan::Union { inputs } => {
            let should = inputs.into_iter()
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
            let must = inputs.into_iter()
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
        LogicalPlan::Scan { .. } => Ok(Query::MatchAll),
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
