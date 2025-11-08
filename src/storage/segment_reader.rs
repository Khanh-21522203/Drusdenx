use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use crate::core::error::{Error, ErrorKind, Result};
use crate::core::types::{DocId, Document, FieldValue};
use crate::query::ast::Query;
use crate::search::results::ScoredDocument;
use crate::storage::layout::StorageLayout;
use crate::storage::segment::{SegmentHeader, SegmentId};

pub struct SegmentReader {
    pub segment_id: SegmentId,
    pub header: SegmentHeader,
    pub file: File,
}

impl SegmentReader {
    pub fn open(storage: &StorageLayout, segment_id: SegmentId) -> Result<Self> {
        let path = storage.segment_path(&segment_id);
        let mut file = File::open(path)?;

        // Read header
        let mut header_buf = vec![0u8; SegmentHeader::SIZE];
        file.read_exact(&mut header_buf)?;
        let header: SegmentHeader = bincode::deserialize(&header_buf)?;

        // Verify version
        if header.version != SegmentHeader::VERSION {
            return Err(Error {
                kind: ErrorKind::InvalidArgument,
                context: "Incompatible segment version".to_string(),
            });
        }

        Ok(SegmentReader {
            segment_id,
            header,
            file,
        })
    }

    pub fn read_all_documents(&mut self) -> Result<Vec<Document>> {
        let mut documents = Vec::with_capacity(self.header.doc_count as usize);

        // Skip header
        self.file.seek(SeekFrom::Start(SegmentHeader::SIZE as u64))?;

        for _ in 0..self.header.doc_count {
            // Read length
            let mut len_buf = [0u8; 4];
            self.file.read_exact(&mut len_buf)?;
            let len = u32::from_le_bytes(len_buf) as usize;

            // Read document
            let mut doc_buf = vec![0u8; len];
            self.file.read_exact(&mut doc_buf)?;
            let doc: Document = bincode::deserialize(&doc_buf)?;

            documents.push(doc);
        }

        Ok(documents)
    }

    /// Get specific document by ID
    /// Scans through segment to find document
    pub fn get_document(&mut self, doc_id: DocId) -> Result<Option<Document>> {
        // Skip header
        self.file.seek(SeekFrom::Start(SegmentHeader::SIZE as u64))?;

        for _ in 0..self.header.doc_count {
            // Read length
            let mut len_buf = [0u8; 4];
            self.file.read_exact(&mut len_buf)?;
            let len = u32::from_le_bytes(len_buf) as usize;

            // Read document
            let mut doc_buf = vec![0u8; len];
            self.file.read_exact(&mut doc_buf)?;
            let doc: Document = bincode::deserialize(&doc_buf)?;

            if doc.id == doc_id {
                return Ok(Some(doc));
            }
        }

        Ok(None)
    }
}