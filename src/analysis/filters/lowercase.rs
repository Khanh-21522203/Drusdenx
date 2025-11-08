use crate::analysis::filter::TokenFilter;
use crate::analysis::token::Token;

pub struct LowercaseFilter;

impl TokenFilter for LowercaseFilter {
    fn filter(&self, tokens: Vec<Token>) -> Vec<Token> {
        tokens.into_iter()
            .map(|mut token| {
                token.text = token.text.to_lowercase();
                token
            })
            .collect()
    }

    fn name(&self) -> &str {
        "lowercase"
    }

    fn clone_box(&self) -> Box<dyn TokenFilter> {
        Box::new(LowercaseFilter)
    }
}