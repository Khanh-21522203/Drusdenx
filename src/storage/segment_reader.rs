use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::sync::Mutex;
use crate::core::error::{Error, ErrorKind, Result};
use crate::core::types::{DocId, Document};
use crate::storage::layout::StorageLayout;
use crate::storage::segment::{SegmentHeader, SegmentId};
use crate::compression::compress::CompressedBlock;

pub struct SegmentReader {
    pub segment_id: SegmentId,
    pub header: SegmentHeader,
    pub file: Mutex<File>,  // Wrapped in Mutex for interior mutability
}

/// Iterator for lazy loading documents
pub struct DocumentIterator<'a> {
    reader: &'a mut SegmentReader,
    current_index: u32,
    total_docs: u32,
}

impl SegmentReader {
    pub fn open(storage: &StorageLayout, segment_id: SegmentId) -> Result<Self> {
        let path = storage.segment_path(&segment_id);
        let mut file = File::open(&path)?;

        // Read header using bincode directly (variable length)
        let header: SegmentHeader = bincode::deserialize_from(&mut file)
            .map_err(|e| Error::new(ErrorKind::Parse, format!("Failed to read header: {}", e)))?;

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
            file: Mutex::new(file),
        })
    }

    /// NEW: Lazy iterator - doesn't load everything into RAM
    /// Use this instead of read_all_documents()
    pub fn iter_documents(&mut self) -> Result<DocumentIterator<'_>> {
        // Seek to start of documents (after header)
        self.file.lock().unwrap().seek(SeekFrom::Start(SegmentHeader::SIZE as u64))?;
        
        // Extract doc_count before borrowing self
        let total_docs = self.header.doc_count;
        
        Ok(DocumentIterator {
            reader: self,
            current_index: 0,
            total_docs,
        })
    }

    /// Read next single document from file
    /// Only loads 1 document into memory at a time
    fn read_next_document(&mut self) -> Result<Option<Document>> {
        let mut file = self.file.lock().unwrap();
        
        // Read length (serialized CompressedBlock size)
        let mut len_buf = [0u8; 4];
        if file.read_exact(&mut len_buf).is_err() {
            return Ok(None); // EOF
        }
        let len = u32::from_le_bytes(len_buf) as usize;

        // Read serialized CompressedBlock
        let mut block_buf = vec![0u8; len];
        file.read_exact(&mut block_buf)?;
        
        // Deserialize CompressedBlock (includes original_size metadata)
        let compressed_block: CompressedBlock = bincode::deserialize(&block_buf)?;
        let decompressed = compressed_block.decompress()?;
        
        // Deserialize document
        let doc: Document = bincode::deserialize(&decompressed)?;

        Ok(Some(doc))
    }

    /// Get specific document by ID
    /// Scans through segment to find document
    pub fn get_document(&self, doc_id: DocId) -> Result<Option<Document>> {
        let mut file = self.file.lock().unwrap();
        
        // Skip header
        file.seek(SeekFrom::Start(SegmentHeader::SIZE as u64))?;

        for _ in 0..self.header.doc_count {
            // Read length (serialized CompressedBlock size)
            let mut len_buf = [0u8; 4];
            file.read_exact(&mut len_buf)?;
            let len = u32::from_le_bytes(len_buf) as usize;

            // Read serialized CompressedBlock
            let mut block_buf = vec![0u8; len];
            file.read_exact(&mut block_buf)?;
            
            // Deserialize CompressedBlock (includes original_size metadata)
            let compressed_block: CompressedBlock = bincode::deserialize(&block_buf)?;
            let decompressed = compressed_block.decompress()?;
            
            // Deserialize document
            let doc: Document = bincode::deserialize(&decompressed)?;

            if doc.id == doc_id {
                return Ok(Some(doc));
            }
        }

        Ok(None)
    }
}

/// Implement Iterator trait for lazy loading
impl<'a> Iterator for DocumentIterator<'a> {
    type Item = Result<Document>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.current_index >= self.total_docs {
            return None;
        }

        self.current_index += 1;
        
        match self.reader.read_next_document() {
            Ok(Some(doc)) => Some(Ok(doc)),
            Ok(None) => None,
            Err(e) => Some(Err(e)),
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = (self.total_docs - self.current_index) as usize;
        (remaining, Some(remaining))
    }
}

impl<'a> ExactSizeIterator for DocumentIterator<'a> {
    fn len(&self) -> usize {
        (self.total_docs - self.current_index) as usize
    }
}