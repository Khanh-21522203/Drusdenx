use std::collections::HashMap;
use regex::Regex;
use serde::{Deserialize, Serialize};
use crate::analysis::token::Token;
use crate::core::error::{Error, ErrorKind, Result};
use crate::core::types::DocId;
use crate::core::utils::levenshtein_distance;
use crate::index::posting::{Posting, PostingList};
use crate::index::skiplist::SkipList;
use crate::search::prefix::PrefixIndex;
use crate::simd::operation::SimdOps;

/// Index statistics for scoring and monitoring
#[derive(Debug, Clone)]
pub struct IndexStats {
    pub doc_count: usize,
    pub total_tokens: usize,
    pub unique_terms: usize,
    pub avg_doc_length: f32,
}

/// Term representation
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Term(Vec<u8>);

impl Term {
    pub fn new(text: &str) -> Self {
        Term(text.as_bytes().to_vec())
    }

    pub fn as_str(&self) -> Result<&str> {
        std::str::from_utf8(&self.0)
            .map_err(|_| Error::new(ErrorKind::Parse, "Invalid UTF-8 in term".to_string()))
    }
}

/// Inverted index structure
pub struct InvertedIndex {
    pub dictionary: TermDictionary,
    pub postings: HashMap<Term, PostingList>,
    pub skip_lists: HashMap<Term, SkipList>,
    pub doc_count: usize,
    pub total_tokens: usize,
    pub prefix_index: Option<PrefixIndex>,
}

impl InvertedIndex {
    pub fn new() -> Self {
        InvertedIndex {
            dictionary: TermDictionary::new(),
            postings: HashMap::new(),
            skip_lists: HashMap::new(),
            doc_count: 0,
            total_tokens: 0,
            prefix_index: None,
        }
    }

    pub fn build_prefix_index(&mut self) -> Result<()> {
        let terms_with_freq = self.dictionary.term_map.iter()
            .map(|(term, idx)| {
                let term_str = String::from_utf8_lossy(&term.0).to_string();
                let doc_freq = self.dictionary.term_infos[*idx].doc_freq;
                (term_str, doc_freq)
            });

        let mut prefix_index = PrefixIndex::new(1); // min_prefix_len = 1
        prefix_index.build(terms_with_freq)?;
        self.prefix_index = Some(prefix_index);
        Ok(())
    }

    // ADD: Search for terms matching prefix
    pub fn prefix_search(&self, prefix: &str) -> Result<Vec<String>> {
        match &self.prefix_index {
            Some(index) => Ok(index.search_prefix(prefix)),
            None => Err(Error::new(ErrorKind::InvalidState, "Prefix index not built".to_string())),
        }
    }
    
    pub fn add_document(&mut self, doc_id: DocId, tokens: &[Token]) -> Result<()> {
        let mut term_positions: HashMap<Term, Vec<u32>> = HashMap::new();

        // Group tokens by term
        for token in tokens {
            let term = Term::new(&token.text);
            term_positions.entry(term)
                .or_insert_with(Vec::new)
                .push(token.position);
        }

        // Update posting lists
        for (term, positions) in term_positions {
            let posting = Posting {
                doc_id,
                term_freq: positions.len() as u32,
                positions,
                field_norm: 1.0 / (tokens.len() as f32).sqrt(), // Simple normalization
            };

            // Get existing postings or create empty vec
            let mut all_postings = if let Some(existing_list) = self.postings.get(&term) {
                // Decode existing postings
                existing_list.iter()?
            } else {
                Vec::new()
            };

            // Add new posting
            all_postings.push(posting);

            // Sort by doc_id (required for delta encoding)
            all_postings.sort_by_key(|p| p.doc_id);

            // Rebuild PostingList with all postings (including new one)
            let new_posting_list = PostingList::new(all_postings)?;
            self.postings.insert(term.clone(), new_posting_list);

            // Update dictionary with term statistics
            if let Some(posting_list) = self.postings.get(&term) {
                self.dictionary.add_term(&term, posting_list.doc_freq());

                // Build skip list for fast querying
                let skip_list = SkipList::build(posting_list)?;
                self.skip_lists.insert(term.clone(), skip_list);
            }
        }

        self.doc_count += 1;
        self.total_tokens += tokens.len();

        Ok(())
    }
    
    /// Get current index statistics
    pub fn stats(&self) -> IndexStats {
        let unique_terms = self.dictionary.term_count();
        let avg_doc_length = if self.doc_count > 0 {
            self.total_tokens as f32 / self.doc_count as f32
        } else {
            0.0
        };
        
        IndexStats {
            doc_count: self.doc_count,
            total_tokens: self.total_tokens,
            unique_terms,
            avg_doc_length,
        }
    }

    pub fn intersect_terms(&self, terms: &[Term]) -> Result<Vec<DocId>> {
        if terms.is_empty() {
            return Ok(Vec::new());
        }

        // Get posting lists and convert to sorted arrays for SIMD operations
        let mut sorted_arrays: Vec<Vec<u32>> = Vec::new();

        for term in terms {
            if let Some(list) = self.postings.get(term) {
                // Extract doc IDs as sorted u32 array
                let doc_ids: Vec<u32> = list.iter()?
                    .into_iter()
                    .map(|posting| posting.doc_id.0 as u32)
                    .collect();
                sorted_arrays.push(doc_ids);
            } else {
                return Ok(Vec::new());  // Term not found
            }
        }
        
        if sorted_arrays.is_empty() {
            return Ok(Vec::new());
        }

        // Use SIMD operations for fast intersection
        let mut result = sorted_arrays[0].clone();
        for i in 1..sorted_arrays.len() {
            result = SimdOps::intersect_sorted(&result, &sorted_arrays[i]);
            if result.is_empty() {
                break;
            }
        }
        
        // Convert back to DocId
        Ok(result.into_iter().map(|id| DocId(id as u64)).collect())
    }

    /// Union multiple terms using SIMD operations
    pub fn union_terms(&self, terms: &[Term]) -> Result<Vec<DocId>> {
        if terms.is_empty() {
            return Ok(Vec::new());
        }
        
        // Collect all doc IDs from all terms
        let mut sorted_arrays: Vec<Vec<u32>> = Vec::new();
        
        for term in terms {
            if let Some(list) = self.postings.get(term) {
                let doc_ids: Vec<u32> = list.iter()?
                    .into_iter()
                    .map(|posting| posting.doc_id.0 as u32)
                    .collect();
                sorted_arrays.push(doc_ids);
            }
            // Note: We don't return empty if a term is not found (union semantics)
        }
        
        if sorted_arrays.is_empty() {
            return Ok(Vec::new());
        }
        
        // Use SIMD operations for fast union
        let mut result = sorted_arrays[0].clone();
        for i in 1..sorted_arrays.len() {
            result = SimdOps::union_sorted(&result, &sorted_arrays[i]);
        }
        
        // Convert back to DocId
        Ok(result.into_iter().map(|id| DocId(id as u64)).collect())
    }
    
    pub fn search_term(&self, term: &Term) -> Option<&PostingList> {
        self.postings.get(term)
    }

    /// Get an iterator over all terms in the index
    pub fn terms(&self) -> impl Iterator<Item = &Term> {
        self.postings.keys()
    }

    pub fn wildcard_search(&self, pattern: &str) -> Result<Vec<String>> {
        // Convert wildcard pattern to regex
        // * -> .*, ? -> .
        let regex_pattern = pattern
            .replace("*", ".*")
            .replace("?", ".");

        let regex = Regex::new(&regex_pattern)
            .map_err(|e| Error::new(ErrorKind::InvalidInput, format!("Invalid wildcard: {}", e)))?;

        // Search through all terms in dictionary
        let mut matching_terms = Vec::new();
        for term in self.dictionary.term_map.keys() {
            let term_str = String::from_utf8_lossy(&term.0);
            if regex.is_match(&term_str) {
                matching_terms.push(term_str.to_string());
            }
        }

        Ok(matching_terms)
    }

    pub fn fuzzy_search(&self, term: &str, max_distance: u8, prefix_length: u8) -> Result<Vec<(String, u8)>> {
        let mut matching_terms = Vec::new();

        // Extract prefix if specified
        let (prefix, suffix) = if prefix_length > 0 && term.len() >= prefix_length as usize {
            term.split_at(prefix_length as usize)
        } else {
            ("", term)
        };

        // Search through all terms in dictionary
        for dict_term in self.dictionary.term_map.keys() {
            let dict_term_str = String::from_utf8_lossy(&dict_term.0);

            // Check prefix match if required
            if !prefix.is_empty() && !dict_term_str.starts_with(prefix) {
                continue;
            }

            // Calculate Levenshtein distance
            let distance = levenshtein_distance(suffix, &dict_term_str[prefix.len()..]);

            if distance <= max_distance as usize {
                matching_terms.push((dict_term_str.to_string(), distance as u8));
            }
        }

        // Sort by distance (closest matches first)
        matching_terms.sort_by_key(|(_, dist)| *dist);

        Ok(matching_terms)
    }
}

/// Term dictionary using FST
pub struct TermDictionary {
    pub term_infos: Vec<TermInfo>,
    pub term_map: HashMap<Term, usize>, // Term -> index in term_infos
}

/// Term statistics
#[derive(Debug, Clone)]
pub struct TermInfo {
    pub doc_freq: u32,        // Number of documents containing term
    pub total_freq: u64,      // Total occurrences across all documents
    pub idf: f32,            // Inverse document frequency
    pub posting_offset: u64,  // Offset in posting file (for persistence)
    pub posting_size: u32,    // Size of posting list
}

impl TermDictionary {
    pub fn new() -> Self {
        TermDictionary {
            term_infos: Vec::new(),
            term_map: HashMap::new(),
        }
    }

    pub fn add_term(&mut self, term: &Term, doc_freq: u32) {
        if let Some(index) = self.term_map.get(term) {
            // Update existing term
            self.term_infos[*index].doc_freq = doc_freq;
        } else {
            // Add new term
            let index = self.term_infos.len();
            self.term_map.insert(term.clone(), index);
            self.term_infos.push(TermInfo {
                doc_freq,
                total_freq: doc_freq as u64,
                idf: 0.0, // Will be calculated later
                posting_offset: 0,
                posting_size: 0,
            });
        }
    }

    pub fn calculate_idf(&mut self, total_docs: usize) {
        for term_info in &mut self.term_infos {
            // IDF = log(N / df) where N is total docs, df is doc frequency
            term_info.idf = ((total_docs as f32 + 1.0) / (term_info.doc_freq as f32 + 1.0)).ln();
        }
    }

    pub fn get_term_info(&self, term: &Term) -> Option<&TermInfo> {
        self.term_map.get(term).map(|index| &self.term_infos[*index])
    }

    pub fn len(&self) -> usize {
        self.term_infos.len()
    }

    pub fn is_empty(&self) -> bool {
        self.term_infos.is_empty()
    }
    
    pub fn term_count(&self) -> usize {
        self.term_map.len()
    }
}