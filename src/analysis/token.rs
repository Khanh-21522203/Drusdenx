use serde::{Serialize, Deserialize};

/// Token representation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Token {
    pub text: String,      // The token text
    pub position: u32,     // Position in document (for phrase queries)
    pub offset: usize,     // Byte offset in original text
    pub length: usize,     // Token length in bytes
    pub token_type: TokenType,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum TokenType {
    Word,
    Number,
    Symbol,
    Punctuation,
    Whitespace,
}

impl Token {
    pub fn new(text: String, position: u32, offset: usize) -> Self {
        let length = text.len();
        Token {
            text,
            position,
            offset,
            length,
            token_type: TokenType::Word,
        }
    }
}