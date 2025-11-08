use crate::query::planner::LogicalPlan;
use crate::query::types::CostModel;

/// Trait for query optimization rules
pub trait OptimizationRule: Send + Sync {
    fn name(&self) -> &str;
    fn optimize(&self, plan: LogicalPlan) -> Option<LogicalPlan>;
}

/// Rule: Push down filters closer to data source
pub struct FilterPushdownRule;

impl OptimizationRule for FilterPushdownRule {
    fn name(&self) -> &str {
        "filter_pushdown"
    }

    fn optimize(&self, plan: LogicalPlan) -> Option<LogicalPlan> {
        match plan {
            LogicalPlan::Sort { field, order, input } => {
                if let LogicalPlan::Filter { predicate, input: inner } = *input {
                    // Move Filter before Sort
                    Some(LogicalPlan::Filter {
                        predicate,
                        input: Box::new(LogicalPlan::Sort {
                            field,
                            order,
                            input: inner,
                        }),
                    })
                } else {
                    None
                }
            }
            _ => None,
        }
    }
}

/// Rule: Merge adjacent Limit operators
pub struct LimitMergeRule;

impl OptimizationRule for LimitMergeRule {
    fn name(&self) -> &str {
        "limit_merge"
    }

    fn optimize(&self, plan: LogicalPlan) -> Option<LogicalPlan> {
        match plan {
            LogicalPlan::Limit { n, input } => {
                if let LogicalPlan::Limit { n: inner_n, input: inner_input } = *input {
                    Some(LogicalPlan::Limit {
                        n: n.min(inner_n),
                        input: inner_input,
                    })
                } else {
                    None
                }
            }
            _ => None,
        }
    }
}

/// Query optimizer
pub struct QueryOptimizer {
    pub rules: Vec<Box<dyn OptimizationRule>>,
    pub cost_model: CostModel,
}

impl QueryOptimizer {
    pub fn new() -> Self {
        QueryOptimizer {
            rules: vec![
                Box::new(FilterPushdownRule),
                Box::new(LimitMergeRule),
            ],
            cost_model: CostModel::default(),
        }
    }

    pub fn optimize(&self, plan: LogicalPlan) -> LogicalPlan {
        let mut optimized = plan;
        for rule in &self.rules {
            if let Some(new_plan) = rule.optimize(optimized.clone()) {
                optimized = new_plan;
            }
        }
        optimized
    }
}