use fst::{IntoStreamer, Map, MapBuilder, Streamer};
use std::collections::BTreeMap;
use crate::core::error::Result;

/// FST-based index for prefix and wildcard queries
pub struct PrefixIndex {
    /// Finite state transducer for prefix matching
    fst: Map<Vec<u8>>,

    /// Minimum prefix length to prevent abuse
    min_prefix_len: usize,

    /// Term → document frequency mapping
    term_frequencies: BTreeMap<String, u32>,
}

impl PrefixIndex {
    pub fn new(min_prefix_len: usize) -> Self {
        Self {
            fst: Map::default(),
            min_prefix_len,
            term_frequencies: BTreeMap::new(),
        }
    }

    /// Build FST from terms
    pub fn build<I>(&mut self, terms: I) -> Result<()>
    where
        I: Iterator<Item = (String, u32)>,
    {
        let mut builder = MapBuilder::memory();
        let mut sorted_terms = Vec::new();

        for (term, freq) in terms {
            sorted_terms.push((term.clone(), freq));
            self.term_frequencies.insert(term, freq);
        }

        // FST requires sorted input
        sorted_terms.sort_by(|a, b| a.0.cmp(&b.0));

        for (term, freq) in sorted_terms {
            builder.insert(term.as_bytes(), freq as u64)?;
        }

        self.fst = builder.into_map();
        Ok(())
    }

    /// Find all terms with given prefix
    pub fn search_prefix(&self, prefix: &str) -> Vec<String> {
        if prefix.len() < self.min_prefix_len {
            return vec![];
        }

        let mut results = Vec::new();
        let prefix_bytes = prefix.as_bytes();

        // Use FST range query for efficient prefix search
        let mut stream = self.fst.range().ge(prefix_bytes).into_stream();

        while let Some((term_bytes, _freq)) = stream.next() {
            if !term_bytes.starts_with(prefix_bytes) {
                break;
            }

            if let Ok(term) = String::from_utf8(term_bytes.to_vec()) {
                results.push(term);
            }
        }

        results
    }

    /// Handle wildcard patterns (e.g., "prog*", "get*User")
    pub fn search_wildcard(&self, pattern: &str) -> Vec<String> {
        let parts: Vec<&str> = pattern.split('*').collect();

        if parts.is_empty() {
            return vec![];
        }

        // Simple case: prefix wildcard "prog*"
        if parts.len() == 2 && parts[1].is_empty() {
            return self.search_prefix(parts[0]);
        }

        // Complex wildcards: use FST iteration with pattern matching
        let mut results = Vec::new();
        let mut stream = self.fst.stream().into_stream();

        while let Some((term_bytes, _)) = stream.next() {
            if let Ok(term) = String::from_utf8(term_bytes.to_vec()) {
                if self.matches_wildcard(&term, pattern) {
                    results.push(term);
                }
            }
        }

        results
    }

    fn matches_wildcard(&self, text: &str, pattern: &str) -> bool {
        let parts: Vec<&str> = pattern.split('*').collect();
        let mut pos = 0;

        for (i, part) in parts.iter().enumerate() {
            if part.is_empty() {
                continue;
            }

            if i == 0 {
                // Pattern must start with this part
                if !text[pos..].starts_with(part) {
                    return false;
                }
                pos += part.len();
            } else if i == parts.len() - 1 {
                // Pattern must end with this part
                if !text[pos..].ends_with(part) {
                    return false;
                }
            } else {
                // Find part somewhere in the middle
                if let Some(idx) = text[pos..].find(part) {
                    pos += idx + part.len();
                } else {
                    return false;
                }
            }
        }

        true
    }
}