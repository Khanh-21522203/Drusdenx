use crate::analysis::token::Token;
use unicode_segmentation::UnicodeSegmentation;

pub trait Tokenizer: Send + Sync {
    fn tokenize(&self, text: &str) -> Vec<Token>;

    fn name(&self) -> &str;

    fn clone_box(&self) -> Box<dyn Tokenizer>;
}

/// Standard Unicode tokenizer
#[derive(Clone)]
pub struct StandardTokenizer {
    pub lowercase: bool,
    pub max_token_length: usize,
}

impl Default for StandardTokenizer {
    fn default() -> Self {
        StandardTokenizer {
            lowercase: true,
            max_token_length: 255,
        }
    }
}

impl Tokenizer for StandardTokenizer {
    fn tokenize(&self, text: &str) -> Vec<Token> {
        let mut tokens = Vec::new();
        let mut position = 0u32;
        let mut offset = 0;

        let text_to_process = if self.lowercase {
            text.to_lowercase()
        } else {
            text.to_string()
        };

        // Use unicode_segmentation to split into words
        for word in text_to_process.unicode_words() {
            if word.len() <= self.max_token_length {
                let token_text = if self.lowercase {
                    word.to_lowercase()
                } else {
                    word.to_string()
                };

                tokens.push(Token::new(
                    token_text,
                    position,
                    offset,
                ));
                position += 1;
            }
            offset += word.len();
        }

        tokens
    }

    fn name(&self) -> &str {
        "standard"
    }

    fn clone_box(&self) -> Box<dyn Tokenizer> {
        Box::new(Self {
            lowercase: self.lowercase,
            max_token_length: self.max_token_length,
        })
    }
}