use crate::query::ast::{Query, TermQuery, PhraseQuery, BoolQuery, RangeQuery, PrefixQuery, WildcardQuery, FuzzyQuery};
use crate::query::types::{IndexStatistics, SortOrder};
use crate::query::visitor::QueryVisitor;
use crate::core::error::Result;

/// Query planner creates execution plans
pub struct QueryPlanner {
    pub statistics: IndexStatistics,
}

impl QueryPlanner {
    pub fn new(statistics: IndexStatistics) -> Self {
        QueryPlanner { statistics }
    }

    /// Create execution plan from query
    pub fn plan(&self, query: &Query) -> LogicalPlan {
        // Use accept() for dispatch; fall back to scan on error
        query.accept(self).unwrap_or_else(|_| LogicalPlan::Scan {
            field: "content".to_string(),
        })
    }
}

impl QueryVisitor for QueryPlanner {
    type Output = LogicalPlan;

    fn visit_term(&self, q: &TermQuery) -> Result<LogicalPlan> {
        Ok(LogicalPlan::IndexSeek {
            field: q.field.clone(),
            term: q.value.clone(),
        })
    }

    fn visit_phrase(&self, _q: &PhraseQuery) -> Result<LogicalPlan> {
        Ok(LogicalPlan::Scan {
            field: "content".to_string(),
        })
    }

    fn visit_bool(&self, q: &BoolQuery) -> Result<LogicalPlan> {
        if !q.must.is_empty() {
            let inputs = q.must
                .iter()
                .map(|query| query.accept(self))
                .collect::<Result<Vec<_>>>()?;
            Ok(LogicalPlan::Intersection { inputs })
        } else if !q.should.is_empty() {
            let inputs = q.should
                .iter()
                .map(|query| query.accept(self))
                .collect::<Result<Vec<_>>>()?;
            Ok(LogicalPlan::Union { inputs })
        } else {
            Ok(LogicalPlan::Scan {
                field: "content".to_string(),
            })
        }
    }

    fn visit_range(&self, _q: &RangeQuery) -> Result<LogicalPlan> {
        Ok(LogicalPlan::Scan {
            field: "content".to_string(),
        })
    }

    fn visit_prefix(&self, _q: &PrefixQuery) -> Result<LogicalPlan> {
        Ok(LogicalPlan::Scan {
            field: "content".to_string(),
        })
    }

    fn visit_wildcard(&self, _q: &WildcardQuery) -> Result<LogicalPlan> {
        Ok(LogicalPlan::Scan {
            field: "content".to_string(),
        })
    }

    fn visit_fuzzy(&self, _q: &FuzzyQuery) -> Result<LogicalPlan> {
        Ok(LogicalPlan::Scan {
            field: "content".to_string(),
        })
    }

    fn visit_match_all(&self) -> Result<LogicalPlan> {
        Ok(LogicalPlan::Scan {
            field: "content".to_string(),
        })
    }
}

/// Logical execution plan
#[derive(Debug, Clone)]
pub enum LogicalPlan {
    Scan { field: String },
    IndexSeek { field: String, term: String },
    Filter { predicate: Query, input: Box<LogicalPlan> },
    Sort { field: String, order: SortOrder, input: Box<LogicalPlan> },
    Limit { n: usize, input: Box<LogicalPlan> },
    Union { inputs: Vec<LogicalPlan> },
    Intersection { inputs: Vec<LogicalPlan> },
    Difference { left: Box<LogicalPlan>, right: Box<LogicalPlan> },
}
