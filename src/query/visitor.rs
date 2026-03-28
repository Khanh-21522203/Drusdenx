use crate::query::ast::{
    TermQuery, PhraseQuery, BoolQuery, RangeQuery,
    PrefixQuery, WildcardQuery, FuzzyQuery,
};
use crate::core::error::Result;

/// One method per Query variant. Implementing this trait is the
/// complete checklist for handling a new query type.
pub trait QueryVisitor {
    type Output;

    fn visit_term(&self, query: &TermQuery)         -> Result<Self::Output>;
    fn visit_phrase(&self, query: &PhraseQuery)     -> Result<Self::Output>;
    fn visit_bool(&self, query: &BoolQuery)         -> Result<Self::Output>;
    fn visit_range(&self, query: &RangeQuery)       -> Result<Self::Output>;
    fn visit_prefix(&self, query: &PrefixQuery)     -> Result<Self::Output>;
    fn visit_wildcard(&self, query: &WildcardQuery) -> Result<Self::Output>;
    fn visit_fuzzy(&self, query: &FuzzyQuery)       -> Result<Self::Output>;
    fn visit_match_all(&self)                       -> Result<Self::Output>;
}
