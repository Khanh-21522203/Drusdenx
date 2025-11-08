use serde::{Serialize, Deserialize};

/// Field definition with analyzer
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldDefinition {
    pub name: String,
    pub field_type: FieldType,
    pub indexed: bool,
    pub stored: bool,
    pub analyzer: Option<String>,  // Analyzer name for this field
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FieldType {
    Text,
    Number,
    Date,
    Boolean,
}

/// Extended schema with analyzer support (extends Schema from M02)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchemaWithAnalyzer {
    pub fields: Vec<FieldDefinitionWithAnalyzer>,
    pub default_analyzer: String,
}

/// Field definition with analyzer (extends FieldDefinition from M02)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldDefinitionWithAnalyzer {
    pub name: String,
    pub field_type: FieldType,
    pub indexed: bool,
    pub stored: bool,
    pub analyzer: Option<String>,  // Added: per-field analyzer
}

impl SchemaWithAnalyzer {
    pub fn new() -> Self {
        SchemaWithAnalyzer {
            fields: Vec::new(),
            default_analyzer: "standard".to_string(),
        }
    }

    pub fn add_text_field(mut self, name: &str, analyzer: Option<String>) -> Self {
        self.fields.push(FieldDefinitionWithAnalyzer {
            name: name.to_string(),
            field_type: FieldType::Text,
            indexed: true,
            stored: true,
            analyzer,
        });
        self
    }

    pub fn get_analyzer_for_field(&self, field_name: &str) -> Option<&String> {
        self.fields
            .iter()
            .find(|f| f.name == field_name)
            .and_then(|f| f.analyzer.as_ref())
    }
}