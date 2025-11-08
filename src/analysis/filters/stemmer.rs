use rust_stemmers::{Algorithm, Stemmer};
use crate::analysis::filter::TokenFilter;
use crate::analysis::token::Token;

pub struct StemmerFilter {
    pub algorithm: Algorithm,
}

impl StemmerFilter {
    pub fn new(algorithm: Algorithm) -> Self {
        StemmerFilter { algorithm }
    }
}

impl TokenFilter for StemmerFilter {
    fn filter(&self, tokens: Vec<Token>) -> Vec<Token> {
        let stemmer = Stemmer::create(self.algorithm);

        tokens.into_iter()
            .map(|mut token| {
                token.text = stemmer.stem(&token.text).to_string();
                token
            })
            .collect()
    }

    fn name(&self) -> &str {
        "stemmer"
    }

    fn clone_box(&self) -> Box<dyn TokenFilter> {
        Box::new(StemmerFilter {
            algorithm: self.algorithm,
        })
    }
}