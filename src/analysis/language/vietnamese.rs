use unicode_segmentation::UnicodeSegmentation;
use crate::analysis::token::{Token, TokenType};
use crate::analysis::tokenizer::Tokenizer;

/// Vietnamese tokenizer
/// Note: For production, use specialized Vietnamese NLP libraries like:
/// - `vi-nlp` or `underthesea` (Python bindings)
/// - For now, we use simple word-based tokenization
pub struct VietnameseTokenizer {
    // Vietnamese is syllable-based, simple word splitting works reasonably
}

impl VietnameseTokenizer {
    pub fn new() -> Self {
        VietnameseTokenizer {}
    }
}

impl Default for VietnameseTokenizer {
    fn default() -> Self {
        Self::new()
    }
}

impl Tokenizer for VietnameseTokenizer {
    fn tokenize(&self, text: &str) -> Vec<Token> {
        let mut tokens = Vec::new();
        let mut position = 0u32;
        let mut offset = 0;

        // Vietnamese words are separated by spaces (syllable-based)
        // More sophisticated tokenization would use dictionary-based approach
        for word in text.unicode_words() {
            let word_str = word.to_string();
            let word_len = word_str.len();

            tokens.push(Token {
                text: word_str,
                position,
                offset,
                length: word_len,
                token_type: TokenType::Word,
            });
            position += 1;
            offset += word_len + 1; // +1 for space
        }

        tokens
    }

    fn name(&self) -> &str {
        "vietnamese"
    }

    fn clone_box(&self) -> Box<dyn Tokenizer> {
        Box::new(VietnameseTokenizer::new())
    }
}