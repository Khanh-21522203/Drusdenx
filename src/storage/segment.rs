use chrono::{DateTime, Utc};
use uuid::Uuid;
use serde::{Deserialize, Serialize};
use crate::core::types::DocId;

/// Unique segment identifier
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SegmentId(pub Uuid);

impl SegmentId {
    pub fn new() -> Self {
        SegmentId(Uuid::new_v4())
    }
}

/// Index segment
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Segment {
    pub id: SegmentId,
    pub doc_count: u32,
    pub metadata: SegmentMetadata,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SegmentMetadata {
    pub created_at: DateTime<Utc>,
    pub size_bytes: usize,
    pub min_doc_id: DocId,
    pub max_doc_id: DocId,
}

/// Segment file header
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SegmentHeader {
    pub version: u32,     // Format version
    pub doc_count: u32,   // Number of documents
    pub checksum: u32,    // CRC32 checksum
    pub compression: CompressionType,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum CompressionType {
    None,
    Lz4,
    Zstd,
}

impl SegmentHeader {
    pub const VERSION: u32 = 1;
    pub const SIZE: usize = 24; // Fixed header size

    pub fn new(doc_count: u32) -> Self {
        SegmentHeader {
            version: Self::VERSION,
            doc_count,
            checksum: 0,
            compression: CompressionType::None,
        }
    }
}