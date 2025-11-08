use std::sync::{Arc, Mutex};
use std::time::Duration;
use parking_lot::RwLock;
use crate::core::config::Config;
use crate::core::types::{Document};
use crate::core::error::Result;
use crate::index::inverted::{InvertedIndex};
use crate::memory::pool::MemoryPool;
use crate::mvcc::controller::MVCCController;
use crate::query::parser::QueryParser;
use crate::reader::reader_pool::ReaderPool;
use crate::schema::schema::SchemaWithAnalyzer;
use crate::search::results::{ScoredDocument};
use crate::storage::layout::StorageLayout;
use crate::storage::segment::SegmentId;
use crate::storage::segment_writer::SegmentWriter;
use crate::storage::wal::{WAL};
use crate::writer::index_writer::{IndexWriter, WriterConfig};

pub struct Database {
    config: Config,

    storage: Arc<StorageLayout>,

    schema: SchemaWithAnalyzer,

    query_parser: QueryParser,

    mvcc: Arc<MVCCController>,
    writer: Arc<RwLock<IndexWriter>>, // index + documents + wal
    reader_pool: Arc<ReaderPool>,  // scorer + query_executor
}

impl Database {
    pub fn open_with_schema(
        schema: SchemaWithAnalyzer,
        config: Config
    ) -> Result<Self> {
        let storage = Arc::new(StorageLayout::new(config.storage_path.clone())?);

        // ✅ NEW M06: Initialize MVCC
        let mvcc = Arc::new(MVCCController::new());

        // ✅ Initialize InvertedIndex (needed by ReaderPool for query matching)
        let index = Arc::new(InvertedIndex::new());

        // ✅ Create IndexWriter (handles segment-based writes)
        let wal = WAL::open(&storage, 0)?;
        let segment_writer = SegmentWriter::new(&storage, SegmentId::new())?;

        // Calculate memory pool blocks from config.memory_limit
        let block_size = 4 * 1024 * 1024;  // 4MB per block
        let num_blocks = config.memory_limit / block_size;
        let memory_pool = MemoryPool::new(num_blocks, block_size);

        let writer = Arc::new(RwLock::new(IndexWriter {
            segment_writer,
            wal,
            memory_pool,
            config: WriterConfig {
                batch_size: config.writer_batch_size,
                commit_interval: Duration::from_secs(config.writer_commit_interval_secs),
                max_segment_size: config.writer_max_segment_size,
            },
            mvcc: mvcc.clone(),
            lock: Arc::new(Mutex::new(())),
            storage: storage.clone(),
        }));

        // ✅ Create reader pool (provides lock-free snapshot-based reads)
        let reader_pool = Arc::new(ReaderPool::new(
            mvcc.clone(),
            storage.clone(),
            index,
            config.max_readers,
        ));

        let query_parser = QueryParser::new();

        Ok(Self {
            writer,
            mvcc,
            reader_pool,
            query_parser,
            storage,
            schema,  // No Arc, SchemaWithAnalyzer is Clone
            config,
        })
    }

    pub fn add_document(&self, doc: Document) -> Result<()> {
        self.writer.write().add_document(doc)
    }

    pub fn search(&self, query_str: &str) -> Result<Vec<ScoredDocument>> {
        // Get reader with snapshot - doesn't block on writes
        let reader = self.reader_pool.get_reader()?;
        let query = self.query_parser.parse(query_str)?;

        // Execute query on snapshot (lock-free!)
        let results = reader.search(&query)?;

        Ok(results.hits)
    }

    // 1. User calls add_document()
    // 2. WAL.append(Operation::AddDocument) ← Write happens HERE (may buffer)
    // 3. Update in-memory index & documents
    // 4. (later) User calls flush()
    // 5. Read from memory → Write to segment file
    // 6. Segment.finish() → Segment safely on disk
    // 7. Checkpoint.save() → Record segment metadata (4 fields)
    // 8. WAL.sync() ← Flush WAL buffer to disk
    // 9. WAL.rotate() ← Create new WAL file (effectively "truncates" old entries)
    pub fn flush(&self) -> Result<()> {
        self.writer.write().flush()
    }

    pub fn commit(&mut self) -> Result<()> {
        self.writer.write().commit()
    }
}






























// use std::path::PathBuf;
// use std::sync::{Arc, RwLock};
// use chrono::Utc;
// use crate::core::config::Config;
// use crate::core::types::{DocId, Document};
// use crate::core::error::Result;
// use crate::core::in_memory_index::{InMemoryIndex, SimpleTokenizer};
// use crate::storage::checkpoint::{Checkpoint, RecoveryManager};
// use crate::storage::file_lock::FileLock;
// use crate::storage::layout::StorageLayout;
// use crate::storage::segment::{Segment, SegmentId};
// use crate::storage::segment_reader::SegmentReader;
// use crate::storage::segment_writer::SegmentWriter;
// use crate::storage::wal::{SyncMode, WAL};
//
// pub struct Database {
//     pub index: Arc<RwLock<InMemoryIndex>>,
//     pub config: Config,
//
//     pub storage: StorageLayout,
//     pub wal: WAL,
//     pub lock: FileLock,
//     pub segments: Vec<SegmentId>,
// }
//
// impl Database {
//     /// Create new database with persistence
//     pub fn new(path: PathBuf) -> Result<Self> {
//         Self::open(path)
//     }
//
//     /// Create in-memory database
//     pub fn new_in_memory() -> Self {
//         let tokenizer = Box::new(SimpleTokenizer);
//         let index = InMemoryIndex::new(tokenizer);
//
//         Database {
//             index: Arc::new(RwLock::new(index)),
//             config: Config::default(),
//             // No persistence components
//             storage: StorageLayout {
//                 base_dir: PathBuf::from("/dev/null"),
//                 segments_dir: PathBuf::from("/dev/null"),
//                 wal_dir: PathBuf::from("/dev/null"),
//                 meta_dir: PathBuf::from("/dev/null"),
//             },
//             wal: WAL {
//                 file: std::fs::File::create("/dev/null").unwrap(),
//                 position: 0,
//                 sequence: 0,
//                 sync_mode: SyncMode::None
//             },
//             lock: FileLock {
//                 file: std::fs::File::create("/dev/null").unwrap(),
//                 exclusive: true,
//             },
//             segments: Vec::new(),
//         }
//     }
//     pub fn add_document(&self, doc: Document) -> Result<()> {
//         let mut index = self.index.write().unwrap();
//         index.add_document(doc)
//     }
//
//     pub fn search(&self, query: &str) -> Result<Vec<Document>> {
//         let index = self.index.read().unwrap();
//         index.search(query)
//     }
//
//     pub fn get_document(&self, id: DocId) -> Result<Option<Document>> {
//         let index = self.index.read().unwrap();
//         Ok(index.documents.get(&id).cloned())
//     }
//
//     pub fn delete_document(&self, id: DocId) -> Result<()> {
//         let mut index = self.index.write().unwrap();
//         index.delete_document(id)
//     }
//
//     pub fn stats(&self) -> IndexStats {
//         let index = self.index.read().unwrap();
//         IndexStats {
//             total_documents: index.total_docs,
//             total_terms: index.terms.len(),
//         }
//     }
//
//     /// Open existing database or create new one
//     pub fn open(path: PathBuf) -> Result<Self> {
//         let storage = StorageLayout::new(path)?;
//         let lock = FileLock::acquire(&storage, true)?;
//         let mut recovery = RecoveryManager::new(storage.clone())?;
//
//         // Recover from crash
//         let operations = recovery.recover()?;
//         // Build index from segments
//         let (index, segments) = Self::load_segments(&storage)?;
//         // Replay operations
//         for op in operations {
//             // Apply operation to index
//         }
//
//         Ok(Database {
//             index: Arc::new(RwLock::new(index)),
//             config: Config::default(),
//             storage,
//             wal: recovery.wal,
//             lock,
//             segments
//         })
//     }
//
//     /// Persist current state
//     pub fn commit(&mut self) -> Result<()> {
//         // Write current batch to segment
//         let segment = self.flush_to_segment()?;
//
//         // Update checkpoint
//         self.create_checkpoint(vec![segment.id])?;
//
//         // Rotate WAL
//         self.wal.rotate(&self.storage)?;
//
//         Ok(())
//     }
//
//     /// Force sync to disk
//     pub fn sync(&mut self) -> Result<()> {
//         self.wal.file.sync_all()?;
//         Ok(())
//     }
//
//     /// Load segments from disk into memory
//     fn load_segments(storage: &StorageLayout) -> Result<(InMemoryIndex, Vec<SegmentId>)> {
//         // Load checkpoint to get active segments
//         let checkpoint = match Checkpoint::load(storage)? {
//             Some(cp) => cp,
//             None => {
//                 // No checkpoint, create empty index
//                 let tokenizer = Box::new(SimpleTokenizer);
//                 return Ok((InMemoryIndex::new(tokenizer), Vec::new()));
//             }
//         };
//
//         // Create empty index
//         let tokenizer = Box::new(SimpleTokenizer);
//         let mut index = InMemoryIndex::new(tokenizer);
//
//         // Load each segment
//         for segment_id in &checkpoint.segments {
//             let mut reader = SegmentReader::open(storage, *segment_id)?;
//             let documents = reader.read_all_documents()?;
//
//             // Add documents to index
//             for doc in documents {
//                 index.add_document(doc)?;
//             }
//         }
//
//         Ok((index, checkpoint.segments))
//     }
//
//     /// Flush current index to new segment
//     fn flush_to_segment(&mut self) -> Result<Segment> {
//         let segment_id = SegmentId::new();
//         let mut writer = SegmentWriter::new(&self.storage, segment_id)?;
//
//         // Write all documents from index
//         let index = self.index.read().unwrap();
//         for (_, doc) in &index.documents {
//             writer.write_document(doc)?;
//         }
//
//         let segment = writer.finish()?;
//         self.segments.push(segment_id);
//
//         Ok(segment)
//     }
//
//     /// Create checkpoint with current segments
//     fn create_checkpoint(&self, segment_ids: Vec<SegmentId>) -> Result<()> {
//         let checkpoint = Checkpoint {
//             wal_position: self.wal.sequence,
//             segments: segment_ids,
//             timestamp: Utc::now(),
//             doc_count: self.index.read().unwrap().documents.len(),
//         };
//
//         checkpoint.save(&self.storage)?;
//         Ok(())
//     }
// }
//
// #[derive(Debug, Clone)]
// pub struct IndexStats {
//     pub total_documents: usize,
//     pub total_terms: usize,
// }