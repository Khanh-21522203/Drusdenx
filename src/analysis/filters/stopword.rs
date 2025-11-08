use std::collections::HashSet;
use crate::analysis::filter::TokenFilter;
use crate::analysis::token::Token;

pub struct StopWordFilter {
    pub stop_words: HashSet<String>,
}

impl StopWordFilter {
    pub fn new(stop_words: Vec<String>) -> Self {
        StopWordFilter {
            stop_words: stop_words.into_iter().collect(),
        }
    }

    pub fn english() -> Self {
        let words = vec![
            "a", "an", "and", "are", "as", "at", "be", "by", "for",
            "from", "has", "he", "in", "is", "it", "its", "of", "on",
            "that", "the", "to", "was", "will", "with"
        ].into_iter().map(String::from).collect();

        StopWordFilter::new(words)
    }
}

impl TokenFilter for StopWordFilter {
    fn filter(&self, tokens: Vec<Token>) -> Vec<Token> {
        tokens.into_iter()
            .filter(|token| !self.stop_words.contains(&token.text))
            .collect()
    }

    fn name(&self) -> &str {
        "stop_words"
    }

    fn clone_box(&self) -> Box<dyn TokenFilter> {
        Box::new(StopWordFilter {
            stop_words: self.stop_words.clone(),
        })
    }
}