use chrono::{DateTime, Utc};
use nom::{IResult, bytes::complete::*, character::complete::*, combinator::*, multi::*, sequence::*};
use crate::core::error::{Error, ErrorKind, Result};
use crate::core::types::FieldValue;
use crate::query::ast::{BoolQuery, PhraseQuery, Query, RangeQuery, TermQuery};

/// Query parser for converting string queries to AST
pub struct QueryParser {
    pub default_field: String,
    pub default_operator: BooleanOperator,
    pub allow_wildcards: bool,
    pub fuzzy_enabled: bool,
}

#[derive(Debug, Clone, Copy)]
pub enum BooleanOperator {
    And,
    Or,
}

impl QueryParser {
    pub fn new() -> Self {
        QueryParser {
            default_field: "content".to_string(),
            default_operator: BooleanOperator::Or,
            allow_wildcards: true,
            fuzzy_enabled: true,
        }
    }

    /// Parse a query string into Query AST
    /// Examples:
    /// - "rust programming" -> OR query
    /// - "rust AND programming" -> AND query
    /// - "title:rust" -> Field query
    /// - "\"exact phrase\"" -> Phrase query
    /// - "price:[10 TO 100]" -> Range query
    /// - "rust~2" -> Fuzzy query
    /// - "rus*" -> Wildcard query
    pub fn parse(&self, input: &str) -> Result<Query> {
        // Simplified parser implementation
        let tokens: Vec<&str> = input.split_whitespace().collect();

        if tokens.is_empty() {
            return Ok(Query::MatchAll);
        }

        // Check for phrase query
        if input.starts_with('"') && input.ends_with('"') {
            let phrase = input.trim_matches('"');
            let terms: Vec<String> = phrase.split_whitespace()
                .map(String::from)
                .collect();
            return Ok(Query::Phrase(PhraseQuery {
                field: self.default_field.clone(),
                phrase: terms,
                slop: 0,
                boost: None,
            }));
        }

        // Check for boolean operators
        if tokens.contains(&"AND") || tokens.contains(&"OR") || tokens.contains(&"NOT") {
            return self.parse_boolean_query(&tokens);
        }

        // Check for field:value syntax
        if let Some(pos) = input.find(':') {
            let field = &input[..pos];
            let value = &input[pos + 1..];

            // Check for range query
            if value.starts_with('[') || value.starts_with('{') {
                return self.parse_range_query(field, value);
            }

            return Ok(Query::Term(TermQuery {
                field: field.to_string(),
                value: value.to_string(),
                boost: None,
            }));
        }

        // // Check for fuzzy query
        // if let Some(pos) = input.find('~') {
        //     let term = &input[..pos];
        //     let distance = input[pos + 1..].parse::<u8>().unwrap_or(2);
        //     return Ok(Query::Fuzzy(FuzzyQuery {
        //         field: self.default_field.clone(),
        //         value: term.to_string(),
        //         max_edits: distance.min(2),
        //         prefix_length: 0,
        //         transpositions: true,
        //         boost: None,
        //     }));
        // }
        //
        // // Check for wildcard query
        // if input.contains('*') || input.contains('?') {
        //     return Ok(Query::Wildcard(WildcardQuery {
        //         field: self.default_field.clone(),
        //         pattern: input.to_string(),
        //         boost: None,
        //     }));
        // }

        // Default to term query
        Ok(Query::Term(TermQuery {
            field: self.default_field.clone(),
            value: input.to_string(),
            boost: None,
        }))
    }

    fn parse_boolean_query(&self, tokens: &[&str]) -> Result<Query> {
        let mut bool_query = BoolQuery::new();
        let mut current_op = self.default_operator;
        let mut current_term = String::new();

        for token in tokens {
            match *token {
                "AND" => current_op = BooleanOperator::And,
                "OR" => current_op = BooleanOperator::Or,
                "NOT" => {
                    // Next term should be must_not
                    current_op = BooleanOperator::And;
                    continue;
                }
                _ => {
                    let term_query = Query::Term(TermQuery {
                        field: self.default_field.clone(),
                        value: token.to_string(),
                        boost: None,
                    });

                    match current_op {
                        BooleanOperator::And => bool_query.must.push(term_query),
                        BooleanOperator::Or => bool_query.should.push(term_query),
                    }
                }
            }
        }

        Ok(Query::Bool(bool_query))
    }

    fn parse_range_query(&self, field: &str, value: &str) -> Result<Query> {
        // Parse [10 TO 100] or {10 TO 100}
        let inclusive_start = value.starts_with('[');
        let inclusive_end = value.ends_with(']');

        let inner = value.trim_start_matches(|c| c == '[' || c == '{')
            .trim_end_matches(|c| c == ']' || c == '}');

        let parts: Vec<&str> = inner.split(" TO ").collect();
        if parts.len() != 2 {
            return Err(Error::new(ErrorKind::Parse, "Invalid range query".parse().unwrap()));
        }

        let mut range = RangeQuery {
            field: field.to_string(),
            gt: None,
            gte: None,
            lt: None,
            lte: None,
            boost: None,
        };

        let start_val = self.parse_field_value(parts[0]);
        let end_val = self.parse_field_value(parts[1]);

        if inclusive_start {
            range.gte = Some(start_val);
        } else {
            range.gt = Some(start_val);
        }

        if inclusive_end {
            range.lte = Some(end_val);
        } else {
            range.lt = Some(end_val);
        }

        Ok(Query::Range(range))
    }

    fn parse_field_value(&self, s: &str) -> FieldValue {
        if let Ok(num) = s.parse::<f64>() {
            FieldValue::Number(num)
        } else if let Ok(date) = DateTime::parse_from_rfc3339(s) {
            FieldValue::Date(date.with_timezone(&Utc))
        } else {
            FieldValue::Text(s.to_string())
        }
    }
}