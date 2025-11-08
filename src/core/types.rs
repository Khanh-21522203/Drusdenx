use serde::{Serialize, Deserialize};
use std::collections::HashMap;
use chrono::{DateTime, Utc};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct DocId(pub u64);

impl DocId {
    pub fn new(id: u64) -> Self {
        DocId(id)
    }

    pub fn value(&self) -> u64 {
        self.0
    }
}

impl From<u64> for DocId {
    fn from(id: u64) -> Self {
        DocId(id)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum FieldValue {
    Text(String),
    Number(f64),
    Date(DateTime<Utc>),
    Boolean(bool),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Document {
    pub id: DocId,
    pub fields: HashMap<String, FieldValue>,
}

impl Document {
    pub fn new(id: DocId) -> Self {
        Document {
            id,
            fields: HashMap::new(),
        }
    }

    pub fn add_field(&mut self, name: String, value: FieldValue) {
        self.fields.insert(name, value);
    }

    pub fn get_field(&self, name: &str) -> Option<&FieldValue> {
        self.fields.get(name)
    }
}