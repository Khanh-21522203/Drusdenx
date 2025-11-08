use std::collections::HashSet;
use rust_stemmers::{Algorithm, Stemmer};
use crate::analysis::token::Token;

pub trait TokenFilter: Send + Sync {
    fn filter(&self, tokens: Vec<Token>) -> Vec<Token>;

    fn name(&self) -> &str;

    fn clone_box(&self) -> Box<dyn TokenFilter>;
}
