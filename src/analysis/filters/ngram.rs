use crate::analysis::filter::TokenFilter;
use crate::analysis::token::Token;

pub struct NGramFilter {
    pub min_gram: usize,
    pub max_gram: usize,
}

impl NGramFilter {
    pub fn new(min_gram: usize, max_gram: usize) -> Self {
        NGramFilter { min_gram, max_gram }
    }
}

impl TokenFilter for NGramFilter {
    fn filter(&self, tokens: Vec<Token>) -> Vec<Token> {
        let mut result = Vec::new();

        for token in tokens {
            let chars: Vec<char> = token.text.chars().collect();

            for n in self.min_gram..=self.max_gram.min(chars.len()) {
                for i in 0..=chars.len().saturating_sub(n) {
                    let ngram: String = chars[i..i + n].iter().collect();

                    result.push(Token {
                        text: ngram,
                        position: token.position,
                        offset: token.offset + i,
                        length: n,
                        token_type: token.token_type,
                    });
                }
            }
        }

        result
    }

    fn name(&self) -> &str {
        "ngram"
    }

    fn clone_box(&self) -> Box<dyn TokenFilter> {
        Box::new(NGramFilter {
            min_gram: self.min_gram,
            max_gram: self.max_gram,
        })
    }
}