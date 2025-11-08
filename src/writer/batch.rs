use crate::core::types::Document;
use crate::writer::index_writer::IndexWriter;
use crate::core::error::Result;

/// Batch writer for bulk operations
pub struct BatchWriter {
    pub writer: IndexWriter,
    pub buffer: Vec<Document>,
    pub batch_size: usize,
}

impl BatchWriter {
    pub fn new(writer: IndexWriter, batch_size: usize) -> Self {
        BatchWriter {
            writer,
            buffer: Vec::with_capacity(batch_size),
            batch_size,
        }
    }

    pub fn add(&mut self, doc: Document) -> Result<()> {
        self.buffer.push(doc);

        if self.buffer.len() >= self.batch_size {
            self.flush()?;
        }

        Ok(())
    }

    pub fn flush(&mut self) -> Result<()> {
        for doc in self.buffer.drain(..) {
            self.writer.add_document(doc)?;
        }
        self.writer.flush()?;
        Ok(())
    }

    pub fn finish(mut self) -> Result<()> {
        self.flush()?;
        self.writer.commit()?;
        Ok(())
    }
}