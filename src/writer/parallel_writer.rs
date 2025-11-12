use std::sync::{Arc, Mutex};
use crossbeam::channel::{bounded, Sender, Receiver};
use std::thread;
use crate::core::types::Document;
use crate::writer::data_writer::DataWriter;
use crate::index::index_writer::IndexWriter;
use crate::storage::segment::Segment;
use crate::core::error::Result;

/// ParallelWriter coordinates DataWriter and IndexWriter in parallel
pub struct ParallelWriter {
    data_writer: Arc<Mutex<DataWriter>>,
    index_writer: Arc<Mutex<IndexWriter>>,
    doc_sender: Sender<Document>,
    segment_receiver: Receiver<Segment>,
}

impl ParallelWriter {
    pub fn new(
        data_writer: DataWriter,
        index_writer: IndexWriter,
    ) -> Self {
        let (doc_sender, doc_receiver) = bounded(1000); // Buffer 1000 docs
        let (segment_sender, segment_receiver) = bounded(10); // Buffer 10 segments
        
        let data_writer = Arc::new(Mutex::new(data_writer));
        let index_writer = Arc::new(Mutex::new(index_writer));
        
        // Spawn parallel writer threads
        let data_writer_clone = data_writer.clone();
        let index_writer_clone = index_writer.clone();
        
        thread::spawn(move || {
            Self::write_worker(doc_receiver, data_writer_clone, index_writer_clone, segment_sender);
        });
        
        ParallelWriter {
            data_writer,
            index_writer,
            doc_sender,
            segment_receiver,
        }
    }
    
    /// Background worker that writes documents in parallel
    fn write_worker(
        doc_receiver: Receiver<Document>,
        data_writer: Arc<Mutex<DataWriter>>,
        index_writer: Arc<Mutex<IndexWriter>>,
        segment_sender: Sender<Segment>,
    ) {
        while let Ok(doc) = doc_receiver.recv() {
            // Write data and index in parallel using rayon
            let doc_clone = doc.clone();
            
            rayon::scope(|s| {
                // Data write thread
                s.spawn(|_| {
                    if let Ok(mut writer) = data_writer.lock() {
                        let _ = writer.write_document(&doc);
                        
                        // Check if flush needed
                        if writer.should_flush() {
                            if let Ok(segment) = writer.flush() {
                                let _ = segment_sender.send(segment);
                            }
                        }
                    }
                });
                
                // Index write thread
                s.spawn(|_| {
                    if let Ok(mut writer) = index_writer.lock() {
                        let _ = writer.index_document(&doc_clone);
                    }
                });
            });
        }
    }
    
    /// Write document (non-blocking)
    pub fn write_document(&self, doc: Document) -> Result<()> {
        self.doc_sender.send(doc).map_err(|_| {
            crate::core::error::Error {
                kind: crate::core::error::ErrorKind::Internal,
                context: "Failed to send document to writer".to_string(),
            }
        })?;
        Ok(())
    }
    
    /// Get flushed segments (non-blocking)
    pub fn try_recv_segment(&self) -> Option<Segment> {
        self.segment_receiver.try_recv().ok()
    }
    
    /// Flush both writers
    pub fn flush(&self) -> Result<Option<Segment>> {
        let mut data_writer = self.data_writer.lock().unwrap();
        let segment = data_writer.flush()?;
        
        // Clear index writer
        let mut index_writer = self.index_writer.lock().unwrap();
        index_writer.clear();
        
        Ok(Some(segment))
    }
}
