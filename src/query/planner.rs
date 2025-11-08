use std::collections::HashMap;
use crate::query::ast::Query;
use crate::query::types::{IndexStatistics, SortOrder};

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
        match query {
            Query::Term(term_query) => {
                LogicalPlan::IndexSeek {
                    field: term_query.field.clone(),
                    term: term_query.value.clone(),
                }
            }
            Query::Bool(bool_query) => {
                if !bool_query.must.is_empty() {
                    // Must clause: intersection
                    let inputs = bool_query.must
                        .iter()
                        .map(|q| self.plan(q))
                        .collect();
                    LogicalPlan::Intersection { inputs }
                } else if !bool_query.should.is_empty() {
                    // Should clause: union
                    let inputs = bool_query.should
                        .iter()
                        .map(|q| self.plan(q))
                        .collect();
                    LogicalPlan::Union { inputs }
                } else {
                    // Default: scan all
                    LogicalPlan::Scan {
                        field: "content".to_string(),
                    }
                }
            }
            Query::MatchAll => {
                LogicalPlan::Scan {
                    field: "content".to_string(),
                }
            }
            _ => {
                // Default plan
                LogicalPlan::Scan {
                    field: "content".to_string(),
                }
            }
        }
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