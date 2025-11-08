// use std::collections::{HashMap, HashSet};
// use crate::analysis::analyzer::Analyzer;
// use crate::analysis::token::Token;
// use crate::analysis::tokenizer::Tokenizer;
// use crate::core::types::{DocId, Document, FieldValue};
// use crate::core::error::{Error, ErrorKind, Result};
// 
// pub struct InMemoryIndex {
//     /// Term → list of document IDs containing the term
//     pub terms: HashMap<String, Vec<DocId>>,
//     /// Document storage
//     pub documents: HashMap<DocId, Document>,
//     /// Total document count for statistics
//     pub total_docs: usize,
//     /// Tokenizer for text processing (injected via constructor)
//     pub tokenizer: Box<dyn Tokenizer>,
// }
// 
// impl InMemoryIndex {
//     pub fn new(tokenizer: Box<dyn Tokenizer>) -> Self {
//         InMemoryIndex {
//             terms: HashMap::new(),
//             documents: HashMap::new(),
//             total_docs: 0,
//             tokenizer
//         }
//     }
// 
//     pub fn add_document(&mut self, doc: Document) -> Result<DocId> {
//         let doc_id = doc.id;
// 
//         // Index text fields
//         for (field_name, field_value) in &doc.fields {
//             if let FieldValue::Text(text) = field_value {
//                 let tokens = self.tokenizer.tokenize(text);
//                 for token in tokens {
//                     self.terms
//                         .entry(token)
//                         .or_insert_with(Vec::new)
//                         .push(doc_id);
//                 }
//             }
//         }
// 
//         // Store document
//         self.documents.insert(doc_id, doc);
//         self.total_docs += 1;
// 
//         Ok(doc_id)
//     }
// 
//     pub fn search(&self, query: &str) -> Result<Vec<Document>> {
//         let tokens = self.tokenizer.tokenize(query);
//         let mut results = Vec::new();
//         let mut seen = HashSet::new();
// 
//         for token in tokens {
//             if let Some(doc_ids) = self.terms.get(&token) {
//                 for doc_id in doc_ids {
//                     if seen.insert(*doc_id) {
//                         if let Some(doc) = self.documents.get(doc_id) {
//                             results.push(doc.clone());
//                         }
//                     }
//                 }
//             }
//         }
// 
//         Ok(results)
//     }
// 
//     pub fn delete_document(&mut self, id: DocId) -> Result<()> {
//         // Remove document from storage
//         if self.documents.remove(&id).is_none() {
//             return Err(Error {
//                 kind: ErrorKind::NotFound,
//                 context: format!("Document {:?} not found", id),
//             });
//         }
// 
//         // Remove document from inverted index
//         for (_term, doc_ids) in self.terms.iter_mut() {
//             doc_ids.retain(|&doc_id| doc_id != id);
//         }
// 
//         // Clean up empty term entries
//         self.terms.retain(|_term, doc_ids| !doc_ids.is_empty());
// 
//         self.total_docs -= 1;
//         Ok(())
//     }
// 
//     pub fn all_documents(&self) -> Vec<Document> {
//         self.documents.values().cloned().collect()
//     }
// }