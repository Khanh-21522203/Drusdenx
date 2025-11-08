use rust_stemmers::Algorithm;
use crate::analysis::filter::TokenFilter;
use crate::analysis::filters::lowercase::LowercaseFilter;
use crate::analysis::filters::stemmer::StemmerFilter;
use crate::analysis::filters::stopword::StopWordFilter;
use crate::analysis::language::vietnamese::VietnameseTokenizer;
use crate::analysis::token::Token;
use crate::analysis::tokenizer::{StandardTokenizer, Tokenizer};
use crate::core::error::Result;
/// Text analysis pipeline
pub struct Analyzer {
    pub tokenizer: Box<dyn Tokenizer>,
    pub filters: Vec<Box<dyn TokenFilter>>,
    pub name: String,
}

impl Analyzer {
    pub fn new(name: String, tokenizer: Box<dyn Tokenizer>) -> Self {
        Analyzer {
            tokenizer,
            filters: Vec::new(),
            name,
        }
    }

    pub fn add_filter(mut self, filter: Box<dyn TokenFilter>) -> Self {
        self.filters.push(filter);
        self
    }

    pub fn analyze(&self, text: &str) -> Vec<Token> {
        let mut tokens = self.tokenizer.tokenize(text);

        for filter in &self.filters {
            tokens = filter.filter(tokens);
        }

        tokens
    }

    /// Create standard analyzer for English
    pub fn standard_english() -> Self {
        Analyzer::new("standard_english".to_string(),
                      Box::new(StandardTokenizer::default()))
            .add_filter(Box::new(LowercaseFilter))
            .add_filter(Box::new(StopWordFilter::english()))
            .add_filter(Box::new(StemmerFilter::new(Algorithm::English)))
    }

    /// Create search analyzer for Vietnamese
    pub fn vietnamese_search() -> Self {
        Analyzer::new("vietnamese_search".to_string(),
                      Box::new(VietnameseTokenizer::new()))
            .add_filter(Box::new(LowercaseFilter))
    }
}

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use crate::core::error::{Error, ErrorKind};

/// Registry for managing analyzers
pub struct AnalyzerRegistry {
    analyzers: Arc<RwLock<HashMap<String, Arc<Analyzer>>>>,
}

impl AnalyzerRegistry {
    pub fn new() -> Self {
        let mut registry = AnalyzerRegistry {
            analyzers: Arc::new(RwLock::new(HashMap::new())),
        };

        // Register default analyzers
        registry.register_defaults();
        registry
    }

    fn register_defaults(&mut self) {
        self.register("standard", Analyzer::standard_english());
        self.register("vietnamese", Analyzer::vietnamese_search());
    }

    pub fn register(&mut self, name: &str, analyzer: Analyzer) {
        let mut analyzers = self.analyzers.write().unwrap();
        analyzers.insert(name.to_string(), Arc::new(analyzer));
    }

    pub fn get(&self, name: &str) -> Option<Arc<Analyzer>> {
        let analyzers = self.analyzers.read().unwrap();
        analyzers.get(name).cloned()
    }

    pub fn analyze(&self, analyzer_name: &str, text: &str) -> Result<Vec<Token>> {
        self.get(analyzer_name)
            .map(|analyzer| analyzer.analyze(text))
            .ok_or_else(||
                Error{
                    kind: ErrorKind::NotFound,
                    context: format!("Analyzer '{}' not found", analyzer_name),
            })

    }
}