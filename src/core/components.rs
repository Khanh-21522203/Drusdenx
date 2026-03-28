use crate::analysis::analyzer::{Analyzer, AnalyzerRegistry};
use crate::core::config::Config;
use crate::core::error::Result;
use crate::index::inverted::InvertedIndex;
use crate::memory::buffer_pool::BufferPool;
use crate::memory::low_memory::LowMemoryMode;
use crate::memory::pool::MemoryPool;
use crate::mvcc::controller::MVCCController;
use crate::parallel::indexer::ParallelIndexer;
use crate::query::cache::QueryCache;
use crate::query::parser::QueryParser;
use crate::reader::reader_pool::ReaderPool;
use crate::schema::schema::SchemaWithAnalyzer;
use crate::search::executor::QueryExecutor;
use crate::storage::layout::StorageLayout;
use crate::writer::index_writer::{IndexWriter, WriterConfig};
use parking_lot::{Mutex, RwLock};
use std::sync::Arc;
use std::time::Duration;

/// All assembled engine components.
/// This is the single authoritative place that knows the assembly ordering.
pub(crate) struct EngineComponents {
    pub(crate) writer: Arc<RwLock<IndexWriter>>,
    pub(crate) reader_pool: Arc<ReaderPool>,
    pub(crate) mvcc: Arc<MVCCController>,
    pub(crate) executor: Arc<QueryExecutor>,
    pub(crate) parser: QueryParser,
    pub(crate) cache: Arc<QueryCache>,
    pub(crate) storage: Arc<StorageLayout>,
    /// Schema used for field-type-aware operations.
    /// Exposed through `SearchIndex::schema()`.
    pub(crate) schema: SchemaWithAnalyzer,
    /// Interior-mutable so `enable_low_memory_mode` can be called through `&self` / `Arc`.
    pub(crate) low_memory: Mutex<Option<Arc<RwLock<LowMemoryMode>>>>,
    pub(crate) config: Config,
}

impl EngineComponents {
    /// Single factory: the only function that knows the assembly DAG.
    pub(crate) fn assemble(schema: SchemaWithAnalyzer, config: Config) -> Result<Self> {
        let storage = Arc::new(StorageLayout::new(config.storage_path.clone())?);

        // Initialize MVCC
        let mvcc = Arc::new(MVCCController::new());
        let index = Arc::new(InvertedIndex::new());

        // Memory subsystem
        let buffer_pool = Arc::new(BufferPool::new(
            config.buffer_pool_size.unwrap_or(100 * 1024 * 1024),
        ));

        let block_size = 4 * 1024 * 1024;
        let num_blocks = config.memory_limit / block_size;
        let memory_pool = MemoryPool::new(num_blocks, block_size);

        // Parallel indexer
        let parallel_indexer = Arc::new(ParallelIndexer::new(
            config.indexing_threads.unwrap_or_else(|| num_cpus::get()),
        ));

        // Analyzer
        let analyzer_registry = Arc::new(AnalyzerRegistry::new());
        let analyzer = analyzer_registry
            .get(&schema.default_analyzer)
            .unwrap_or_else(|| Arc::new(Analyzer::standard_english()));

        // IndexWriter with merge policy
        let merge_policy_type = config.merge_policy;
        let mut index_writer = IndexWriter::new_with_merge_policy(
            storage.clone(),
            mvcc.clone(),
            memory_pool,
            buffer_pool.clone(),
            parallel_indexer.clone(),
            analyzer,
            merge_policy_type,
            config.compression,
        )?;

        index_writer.config = WriterConfig {
            batch_size: config.writer_batch_size,
            commit_interval: Duration::from_secs(config.writer_commit_interval_secs),
            max_segment_size: config.writer_max_segment_size,
            compression: config.compression,
        };

        let writer = Arc::new(RwLock::new(index_writer));

        // Query cache
        let cache_entries = config.cache_size / 1024;
        let cache = Arc::new(QueryCache::new(cache_entries));

        // Reader pool
        let reader_pool = Arc::new(ReaderPool::new(
            mvcc.clone(),
            storage.clone(),
            index,
            config.max_readers,
        ));

        let parser = QueryParser::new();
        let executor = Arc::new(QueryExecutor::new());

        Ok(EngineComponents {
            writer,
            reader_pool,
            mvcc,
            executor,
            parser,
            cache,
            storage,
            schema,
            low_memory: Mutex::new(None),
            config,
        })
    }
}
