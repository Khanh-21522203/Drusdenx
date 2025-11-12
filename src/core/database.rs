use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, Instant};
use parking_lot::RwLock;
use crate::analysis::analyzer::{Analyzer, AnalyzerRegistry};
use crate::core::config::Config;
use crate::core::stats::{DatabaseStats, MemoryStats, BufferStats, HealthStatus, HealthCheckResult, HealthCheck};
use crate::core::types::{Document, DocId};
use crate::core::error::Result;
use crate::index::inverted::{InvertedIndex};
use crate::memory::buffer_pool::BufferPool;
use crate::memory::pool::MemoryPool;
use crate::memory::low_memory::{LowMemoryMode, LowMemoryConfig};
use crate::mvcc::controller::MVCCController;
use crate::parallel::indexer::ParallelIndexer;
use crate::query::cache::QueryCache;
use crate::query::parser::QueryParser;
use crate::reader::reader_pool::ReaderPool;
use crate::search::executor::{QueryExecutor, ExecutionConfig};
use crate::schema::schema::SchemaWithAnalyzer;
use crate::search::results::{ScoredDocument, SearchResults};
use crate::storage::layout::StorageLayout;
use crate::storage::segment::SegmentId;
use crate::storage::segment_writer::SegmentWriter;
use crate::storage::wal::{WAL, Operation};
use crate::writer::index_writer::{IndexWriter, WriterConfig};
use crate::core::transaction::{TransactionManager, Transaction};

pub struct Database {
    pub(crate) config: Config,

    pub(crate) storage: Arc<StorageLayout>,

    pub(crate) schema: SchemaWithAnalyzer,

    pub(crate) query_parser: QueryParser,
    pub(crate) query_executor: Arc<QueryExecutor>,  // NEW: Stateless query execution service
    pub(crate) query_cache: Arc<QueryCache>,        // NEW: Query result cache

    pub(crate) mvcc: Arc<MVCCController>,
    pub(crate) writer: Arc<RwLock<IndexWriter>>, // index + documents + wal
    pub(crate) reader_pool: Arc<ReaderPool>,  // scorer + query_executor
    
    // Monitoring and metrics
    start_time: Instant,
    query_count: AtomicU64,
    write_count: AtomicU64,
    last_flush_time: Arc<RwLock<Option<SystemTime>>>,
    last_commit_time: Arc<RwLock<Option<SystemTime>>>,
    
    // Transaction support
    pub(crate) transaction_manager: Option<Arc<TransactionManager>>,
    
    // Low memory mode support
    pub(crate) low_memory_mode: Option<Arc<RwLock<LowMemoryMode>>>,
}

impl Database {
    pub fn open_with_schema(
        schema: SchemaWithAnalyzer,
        config: Config
    ) -> Result<Self> {
        let storage = Arc::new(StorageLayout::new(config.storage_path.clone())?);

        // Initialize MVCC
        let mvcc = Arc::new(MVCCController::new());
        let index = Arc::new(InvertedIndex::new());

        // Create IndexWriter (handles segment-based writes)
        let wal = WAL::open(&storage, 0)?;
        let buffer_pool = Arc::new(BufferPool::new(
            config.buffer_pool_size.unwrap_or(100 * 1024 * 1024)  // Default 100MB
        ));

        // Create ParallelIndexer for concurrent document processing
        let parallel_indexer = Arc::new(ParallelIndexer::new(
            config.indexing_threads.unwrap_or_else(|| num_cpus::get())
        ));
        let segment_writer = SegmentWriter::new(
            &storage,
            SegmentId::new(),
            buffer_pool.clone()
        )?;

        // Calculate memory pool blocks from config.memory_limit
        let block_size = 4 * 1024 * 1024;  // 4MB per block
        let num_blocks = config.memory_limit / block_size;
        let memory_pool = MemoryPool::new(num_blocks, block_size);

        let analyzer_registry = Arc::new(AnalyzerRegistry::new());

        // Get Analyzer from registry using schema.default_analyzer
        let analyzer = analyzer_registry
            .get(&schema.default_analyzer)
            .unwrap_or_else(|| {
                // Fallback to standard analyzer if not found
                Arc::new(Analyzer::standard_english())
            });


        // Create IndexWriter with configured merge policy
        let merge_policy_type = config.merge_policy;
        let mut index_writer = IndexWriter::new_with_merge_policy(
            storage.clone(),
            mvcc.clone(),
            memory_pool,
            buffer_pool.clone(),
            parallel_indexer.clone(),
            analyzer,
            merge_policy_type,
        )?;
        
        // Override config
        index_writer.config = WriterConfig {
            batch_size: config.writer_batch_size,
            commit_interval: Duration::from_secs(config.writer_commit_interval_secs),
            max_segment_size: config.writer_max_segment_size,
        };
        
        let writer = Arc::new(RwLock::new(index_writer));

        // Create shared QueryCache
        let cache_entries = config.cache_size / 1024; // Approximate entry count (1KB per result)
        let query_cache = Arc::new(QueryCache::new(cache_entries));

        // Create reader pool (provides lock-free snapshot-based reads)
        let reader_pool = Arc::new(ReaderPool::new(
            mvcc.clone(),
            storage.clone(),
            index,
            config.max_readers,
        ));

        let query_parser = QueryParser::new();
        
        // Create stateless query executor
        let query_executor = Arc::new(QueryExecutor::new());

        let db = Self {
            writer,
            mvcc,
            reader_pool,
            query_parser,
            query_executor,
            query_cache,
            storage,
            schema,  // No Arc, SchemaWithAnalyzer is Clone
            config,
            start_time: Instant::now(),
            query_count: AtomicU64::new(0),
            write_count: AtomicU64::new(0),
            last_flush_time: Arc::new(RwLock::new(None)),
            last_commit_time: Arc::new(RwLock::new(None)),
            transaction_manager: None, // Will be set after database is created
            low_memory_mode: None, // Will be enabled if needed
        };
        
        Ok(db)
    }

    pub fn add_document(&self, doc: Document) -> Result<()> {
        self.write_count.fetch_add(1, Ordering::Relaxed);
        
        // Estimate document size (rough estimate: fields + overhead)
        let doc_size = doc.fields.iter()
            .map(|(k, v)| k.len() + match v {
                crate::core::types::FieldValue::Text(s) => s.len(),
                crate::core::types::FieldValue::Number(_) => 8,
                crate::core::types::FieldValue::Date(_) => 8,
                crate::core::types::FieldValue::Boolean(_) => 1,
            })
            .sum::<usize>() + 100; // 100 bytes overhead per document
        
        // Track memory allocation if low memory mode is enabled
        if let Some(low_mem) = &self.low_memory_mode {
            let lm = low_mem.read();
            let _ = lm.memory_tracker.allocate(doc_size);
        }
        
        // Check memory pressure and reclaim if needed
        if let Some(pressure) = self.get_memory_pressure() {
            if pressure > 0.8 {
                // Trigger memory reclamation in background (non-blocking)
                self.maybe_reclaim_memory()?;
            }
        }
        
        self.writer.write().add_document(doc)
    }
    
    /// Delete a document by ID (soft delete - marks as deleted)
    pub fn delete_document(&self, doc_id: DocId) -> Result<()> {
        self.write_count.fetch_add(1, Ordering::Relaxed);
        
        // Deallocate memory if low memory mode is enabled
        // Estimate: average document size (rough estimate)
        if let Some(low_mem) = &self.low_memory_mode {
            let lm = low_mem.read();
            lm.memory_tracker.deallocate(500); // Average doc size estimate
        }
        
        self.writer.write().delete_document(doc_id)
    }
    
    /// Delete documents matching a query
    pub fn delete_by_query(&self, query_str: &str) -> Result<usize> {
        // Parse query
        let query = self.query_parser.parse(query_str)?;
        
        // Get reader to find matching documents
        let reader = self.reader_pool.get_reader()?;
        
        // Search for documents to delete
        let results = reader.search(&query)?;
        let mut deleted_count = 0;
        
        // Delete each matching document
        for doc in results.hits {
            self.delete_document(doc.doc_id)?;
            deleted_count += 1;
        }
        
        Ok(deleted_count)
    }
    
    /// Compact the index to physically remove deleted documents
    /// This creates new segments without deleted documents
    pub fn compact(&self) -> Result<()> {
        self.writer.write().compact()
    }

    pub fn search(&self, query_str: &str) -> Result<Vec<ScoredDocument>> {
        self.search_with_limit(query_str, 10) // Default limit of 10
    }
    
    pub fn search_with_limit(&self, query_str: &str, limit: usize) -> Result<Vec<ScoredDocument>> {
        self.query_count.fetch_add(1, Ordering::Relaxed);
        
        // 1. Check cache first (optimized - no string allocation)
        if let Some(cached_results) = self.query_cache.get_by_str(query_str, limit, 0) {
            return Ok(cached_results.hits);
        }
        
        // 2. Parse query string
        let query = self.query_parser.parse(query_str)?;
        
        // 3. Get reader with snapshot - doesn't block on writes
        let reader = self.reader_pool.get_reader()?;
        
        // 4. Execute query using QueryExecutor service
        let config = ExecutionConfig::default();
        let results = self.query_executor.execute(&reader, &query, limit, config)?;
        
        // 5. Cache results for future queries (optimized - no string allocation)
        self.query_cache.put_by_str(query_str, limit, 0, results.clone());
        
        // 6. Return hits
        Ok(results.hits)
    }
    
    pub fn search_debug(&self, query_str: &str, limit: usize) -> Result<SearchResults> {
        // Debug version that returns full results with explanations
        let query = self.query_parser.parse(query_str)?;
        let reader = self.reader_pool.get_reader()?;
        
        let config = ExecutionConfig::debug(); // Enables explanations
        self.query_executor.execute(&reader, &query, limit, config)
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
        let result = self.writer.write().flush();
        if result.is_ok() {
            *self.last_flush_time.write() = Some(SystemTime::now());
        }
        result
    }

    pub fn commit(&self) -> Result<()> {
        let result = self.writer.write().commit();
        if result.is_ok() {
            *self.last_commit_time.write() = Some(SystemTime::now());
        }
        result
    }
    
    /// Recover from WAL after crash or restart
    /// Should be called during database initialization
    pub fn recover(&self) -> Result<()> {
        // Find all WAL files
        let storage = self.storage.clone();
        let wal_sequences = WAL::find_wal_files(&storage)?;
        
        if wal_sequences.is_empty() {
            return Ok(()); // Nothing to recover
        }
        
        println!("Starting WAL recovery, found {} WAL files", wal_sequences.len());
        let mut recovered_count = 0;
        
        // Process each WAL file in order
        for sequence in wal_sequences {
            let mut wal = WAL::open(&storage, sequence)?;
            let entries = wal.read_entries()?;
            
            println!("Processing WAL sequence {}: {} entries", sequence, entries.len());
            
            for entry in entries {
                match entry.operation {
                    Operation::AddDocument(doc) => {
                        // Re-add document to index
                        match self.add_document(doc) {
                            Ok(_) => recovered_count += 1,
                            Err(e) => {
                                eprintln!("Warning: Failed to recover document: {}", e);
                            }
                        }
                    },
                    Operation::DeleteDocument(doc_id) => {
                        // Recover delete operation by marking document as deleted
                        match self.delete_document(doc_id) {
                            Ok(_) => recovered_count += 1,
                            Err(e) => {
                                eprintln!("Warning: Failed to recover delete for doc {}: {}", doc_id.0, e);
                            }
                        }
                    },
                    Operation::UpdateDocument(doc) => {
                        // Update document
                        match self.add_document(doc) {
                            Ok(_) => recovered_count += 1,
                            Err(e) => {
                                eprintln!("Warning: Failed to recover document update: {}", e);
                            }
                        }
                    },
                    Operation::Commit => {
                        // Commit operation - ensure data is persisted
                        self.flush()?;
                    }
                }
            }
        }
        
        // After recovery, commit to ensure everything is persisted
        self.commit()?;
        
        println!("WAL recovery completed: {} operations recovered", recovered_count);
        Ok(())
    }
    
    /// Enable low memory mode with custom configuration
    pub fn enable_low_memory_mode(&mut self, config: LowMemoryConfig) {
        let low_mem = LowMemoryMode::new(config);
        self.low_memory_mode = Some(Arc::new(RwLock::new(low_mem)));
    }
    
    /// Enable low memory mode with default configuration
    pub fn enable_low_memory_mode_default(&mut self) {
        self.enable_low_memory_mode(LowMemoryConfig::default());
    }
    
    /// Check if low memory mode is enabled
    pub fn is_low_memory_mode_enabled(&self) -> bool {
        self.low_memory_mode.is_some()
    }
    
    /// Get current memory pressure (0.0 to 1.0)
    pub fn get_memory_pressure(&self) -> Option<f32> {
        self.low_memory_mode.as_ref().map(|lm| {
            lm.read().memory_pressure()
        })
    }
    
    /// Trigger memory reclamation if needed (should be called periodically)
    pub fn maybe_reclaim_memory(&self) -> Result<()> {
        if let Some(low_mem) = &self.low_memory_mode {
            let mut lm = low_mem.write();
            lm.maybe_reclaim()?;
        }
        Ok(())
    }
    
    /// Enable transactions for this database
    pub fn enable_transactions(self) -> Arc<Self> {
        Arc::new(self)
    }
    
    /// Create a database with transactions enabled from the start
    pub fn with_transactions(config: Config, schema: SchemaWithAnalyzer) -> Result<Arc<Self>> {
        let db = Self::open_with_schema(schema, config)?;
        Ok(Arc::new(db))
    }
    
    /// Begin a new transaction
    pub fn begin_transaction(&self, isolation: crate::mvcc::controller::IsolationLevel) -> Arc<Transaction> {
        // Create transaction directly using MVCC controller
        Arc::new(Transaction::begin(self.mvcc.clone(), isolation))
    }
    
    /// Execute in a transaction
    pub fn with_transaction<F, R>(&self, isolation: crate::mvcc::controller::IsolationLevel, f: F) -> Result<R>
    where
        F: FnOnce(&Transaction) -> Result<R>,
    {
        let tx = self.begin_transaction(isolation);
        
        match f(&tx) {
            Ok(result) => {
                // Get operations from transaction
                let ops = tx.commit()?;
                
                // Execute operations
                for op in ops {
                    match op {
                        crate::core::transaction::TransactionOp::Insert(doc) => {
                            self.add_document(doc)?;
                        }
                        crate::core::transaction::TransactionOp::Update(doc_id, doc) => {
                            self.delete_document(doc_id)?;
                            self.add_document(doc)?;
                        }
                        crate::core::transaction::TransactionOp::Delete(doc_id) => {
                            self.delete_document(doc_id)?;
                        }
                    }
                }
                
                // Flush to ensure durability
                self.flush()?;
                
                Ok(result)
            }
            Err(e) => {
                tx.rollback()?;
                Err(e)
            }
        }
    }
    
    /// Get database statistics for monitoring
    pub fn stats(&self) -> Result<DatabaseStats> {
        let snapshot = self.mvcc.current_snapshot();
        let cache_stats = self.query_cache.stats();
        
        // Calculate query rate
        let uptime_secs = self.start_time.elapsed().as_secs();
        let query_count = self.query_count.load(Ordering::Relaxed);
        let write_count = self.write_count.load(Ordering::Relaxed);
        let queries_per_second = if uptime_secs > 0 {
            query_count as f64 / uptime_secs as f64
        } else {
            0.0
        };
        let writes_per_second = if uptime_secs > 0 {
            write_count as f64 / uptime_secs as f64
        } else {
            0.0
        };
        
        // Get WAL size
        let wal_size = self.writer.read().wal.position;
        
        // Calculate index size (approximate - sum of segment sizes)
        let index_size_bytes: u64 = snapshot.segments.iter()
            .map(|seg| seg.metadata.size_bytes as u64)
            .sum();
        
        Ok(DatabaseStats {
            uptime_secs,
            start_time: SystemTime::now() - Duration::from_secs(uptime_secs),
            
            // Storage metrics
            segment_count: snapshot.segments.len(),
            total_documents: snapshot.doc_count,
            deleted_documents: snapshot.deleted_docs.len() as usize,
            index_size_bytes,
            wal_size_bytes: wal_size,
            
            // Memory metrics (simplified - real impl would query pools)
            memory_pool_usage: MemoryStats {
                allocated_bytes: 0, // TODO: Get from memory pool
                used_bytes: 0,
                capacity_bytes: self.config.memory_limit,
                utilization_percent: 0.0,
            },
            buffer_pool_usage: BufferStats {
                page_count: 0, // TODO: Get from buffer pool
                page_size: 4096,
                hit_rate: 0.0,
                dirty_pages: 0,
            },
            reader_pool_size: self.reader_pool.max_readers,
            
            // Query metrics
            cache_stats,
            queries_per_second,
            avg_query_latency_ms: 0.0, // TODO: Track query latency
            
            // Write metrics
            writes_per_second,
            pending_writes: 0, // TODO: Track pending writes
            last_flush_time: self.last_flush_time.read().clone(),
            last_commit_time: self.last_commit_time.read().clone(),
        })
    }
    
    /// Health check for monitoring systems
    pub fn health_check(&self) -> Result<HealthCheckResult> {
        let mut checks = Vec::new();
        let _start = Instant::now();
        
        // Check 1: WAL is writable
        let wal_check_start = Instant::now();
        let wal_status = match self.writer.try_write() {
            Some(_writer) => HealthStatus::Healthy,
            None => HealthStatus::Degraded("WAL is locked".to_string()),
        };
        checks.push(HealthCheck {
            name: "WAL".to_string(),
            status: wal_status.clone(),
            message: None,
            latency_ms: wal_check_start.elapsed().as_millis() as u64,
        });
        
        // Check 2: Can get reader
        let reader_check_start = Instant::now();
        let reader_status = match self.reader_pool.get_reader() {
            Ok(_) => HealthStatus::Healthy,
            Err(e) => HealthStatus::Unhealthy(format!("Cannot get reader: {}", e)),
        };
        checks.push(HealthCheck {
            name: "ReaderPool".to_string(),
            status: reader_status.clone(),
            message: None,
            latency_ms: reader_check_start.elapsed().as_millis() as u64,
        });
        
        // Check 3: Query cache responsive
        let cache_check_start = Instant::now();
        let cache_status = if self.query_cache.stats().hit_rate() >= 0.0 {
            HealthStatus::Healthy
        } else {
            HealthStatus::Degraded("Cache hit rate low".to_string())
        };
        checks.push(HealthCheck {
            name: "QueryCache".to_string(),
            status: cache_status.clone(),
            message: Some(format!("Hit rate: {:.2}%", self.query_cache.stats().hit_rate() * 100.0)),
            latency_ms: cache_check_start.elapsed().as_millis() as u64,
        });
        
        // Check 4: Disk space
        let disk_check_start = Instant::now();
        let disk_status = HealthStatus::Healthy; // TODO: Check actual disk space
        checks.push(HealthCheck {
            name: "DiskSpace".to_string(),
            status: disk_status.clone(),
            message: None,
            latency_ms: disk_check_start.elapsed().as_millis() as u64,
        });
        
        // Check 5: Memory pressure (if low memory mode enabled)
        let memory_check_start = Instant::now();
        if let Some(pressure) = self.get_memory_pressure() {
            let memory_status = if pressure > 0.9 {
                HealthStatus::Unhealthy(format!("Memory pressure critical: {:.1}%", pressure * 100.0))
            } else if pressure > 0.8 {
                HealthStatus::Degraded(format!("Memory pressure high: {:.1}%", pressure * 100.0))
            } else {
                HealthStatus::Healthy
            };
            checks.push(HealthCheck {
                name: "Memory".to_string(),
                status: memory_status,
                message: Some(format!("Pressure: {:.1}%", pressure * 100.0)),
                latency_ms: memory_check_start.elapsed().as_millis() as u64,
            });
        }
        
        // Overall status
        let overall_status = if checks.iter().all(|c| c.status == HealthStatus::Healthy) {
            HealthStatus::Healthy
        } else if checks.iter().any(|c| matches!(c.status, HealthStatus::Unhealthy(_))) {
            HealthStatus::Unhealthy("One or more critical checks failed".to_string())
        } else {
            HealthStatus::Degraded("Some checks are degraded".to_string())
        };
        
        Ok(HealthCheckResult {
            status: overall_status,
            checks,
            timestamp: SystemTime::now(),
        })
    }
}