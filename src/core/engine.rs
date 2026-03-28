use crate::core::components::EngineComponents;
use crate::core::config::Config;
use crate::core::error::Result;
use crate::core::stats::{
    BufferStats, DatabaseStats, HealthCheck, HealthCheckResult, HealthStatus, MemoryStats,
};
use crate::core::transaction::Transaction;
use crate::core::types::{DocId, Document};
use crate::memory::low_memory::LowMemoryConfig;
use crate::mvcc::controller::IsolationLevel;
use crate::schema::schema::SchemaWithAnalyzer;
use crate::search::executor::ExecutionConfig;
use crate::search::results::SearchResults;
use crate::storage::wal::{Operation, WAL, WALEntry};
use parking_lot::RwLock;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime};

/// Internal coordinator.
/// All method bodies live here; `SearchIndex` (the public facade) delegates to this.
pub(crate) struct SearchEngine {
    pub(crate) components: EngineComponents,
    start_time: Instant,
    query_count: AtomicU64,
    write_count: AtomicU64,
    last_flush_time: Arc<RwLock<Option<SystemTime>>>,
    last_commit_time: Arc<RwLock<Option<SystemTime>>>,
}

impl SearchEngine {
    pub(crate) fn new(schema: SchemaWithAnalyzer, config: Config) -> Result<Self> {
        let components = EngineComponents::assemble(schema, config)?;
        Ok(SearchEngine {
            components,
            start_time: Instant::now(),
            query_count: AtomicU64::new(0),
            write_count: AtomicU64::new(0),
            last_flush_time: Arc::new(RwLock::new(None)),
            last_commit_time: Arc::new(RwLock::new(None)),
        })
    }

    pub(crate) fn write_document(&self, doc: Document) -> Result<()> {
        self.write_count.fetch_add(1, Ordering::Relaxed);

        // Estimate document size
        let doc_size = doc
            .fields
            .iter()
            .map(|(k, v)| {
                k.len()
                    + match v {
                        crate::core::types::FieldValue::Text(s) => s.len(),
                        crate::core::types::FieldValue::Number(_) => 8,
                        crate::core::types::FieldValue::Date(_) => 8,
                        crate::core::types::FieldValue::Boolean(_) => 1,
                    }
            })
            .sum::<usize>()
            + 100;

        if let Some(low_mem) = self.components.low_memory.lock().as_ref().cloned() {
            let lm = low_mem.read();
            lm.memory_tracker.allocate(doc_size)?;
        }

        if let Some(pressure) = self.get_memory_pressure() {
            if pressure > 0.8 {
                self.maybe_reclaim_memory()?;
            }
        }

        self.components.writer.write().add_document(doc)
    }

    pub(crate) fn delete_document_by_id(&self, doc_id: DocId) -> Result<()> {
        self.write_count.fetch_add(1, Ordering::Relaxed);

        if let Some(low_mem) = self.components.low_memory.lock().as_ref().cloned() {
            let lm = low_mem.read();
            lm.memory_tracker.deallocate(500);
        }

        self.components.writer.write().delete_document(doc_id)
    }

    pub(crate) fn delete_by_query(&self, query_str: &str) -> Result<usize> {
        let query = self.components.parser.parse(query_str)?;
        let reader = self.components.reader_pool.get_reader()?;
        let results = reader.search(&query)?;
        let mut deleted_count = 0;

        for doc in results.hits {
            self.delete_document_by_id(doc.doc_id)?;
            deleted_count += 1;
        }

        Ok(deleted_count)
    }

    pub(crate) fn compact(&self) -> Result<()> {
        self.components.writer.write().compact()
    }

    pub(crate) fn run_search(
        &self,
        query_str: &str,
        limit: usize,
        config: ExecutionConfig,
    ) -> Result<SearchResults> {
        self.query_count.fetch_add(1, Ordering::Relaxed);

        if let Some(cached_results) = self.components.cache.get_by_str(query_str, limit, 0) {
            return Ok(cached_results);
        }

        let query = self.components.parser.parse(query_str)?;
        let reader = self.components.reader_pool.get_reader()?;
        let results = self
            .components
            .executor
            .execute(&reader, &query, limit, config)?;

        self.components
            .cache
            .put_by_str(query_str, limit, 0, results.clone());

        Ok(results)
    }

    pub(crate) fn flush_segments(&self) -> Result<()> {
        let result = self.components.writer.write().flush();
        if result.is_ok() {
            *self.last_flush_time.write() = Some(SystemTime::now());
        }
        result
    }

    pub(crate) fn commit_wal(&self) -> Result<()> {
        let result = self.components.writer.write().commit();
        if result.is_ok() {
            *self.last_commit_time.write() = Some(SystemTime::now());
        }
        result
    }

    pub(crate) fn recover(&self) -> Result<()> {
        let storage = self.components.storage.clone();
        let wal_sequences = WAL::find_wal_files(&storage)?;

        if wal_sequences.is_empty() {
            return Ok(());
        }

        let mut recovered_count: usize = 0;

        for sequence in wal_sequences {
            let mut wal = WAL::open(&storage, sequence)?;
            let entries = wal.read_entries()?;
            let pending_operations = operations_after_last_commit(entries);

            for operation in pending_operations {
                let mut writer = self.components.writer.write();
                match writer.apply_recovered_operation(operation) {
                    Ok(_) => recovered_count += 1,
                    Err(e) => eprintln!("Warning: Failed to recover operation: {}", e),
                }
            }
        }

        if recovered_count > 0 {
            self.commit_wal()?;
        }
        eprintln!(
            "WAL recovery completed: {} operations recovered",
            recovered_count
        );
        Ok(())
    }

    pub(crate) fn begin_transaction(&self, isolation: IsolationLevel) -> Arc<Transaction> {
        Arc::new(Transaction::begin(
            self.components.mvcc.clone(),
            self.components.storage.clone(),
            isolation,
        ))
    }

    pub(crate) fn with_transaction<F, R>(&self, isolation: IsolationLevel, f: F) -> Result<R>
    where
        F: FnOnce(&Transaction) -> Result<R>,
    {
        let tx = self.begin_transaction(isolation);

        match f(&tx) {
            Ok(result) => {
                let ops = tx.commit()?;
                for op in ops {
                    match op {
                        crate::core::transaction::TransactionOp::Insert(doc) => {
                            self.write_document(doc)?;
                        }
                        crate::core::transaction::TransactionOp::Update(doc_id, doc) => {
                            self.delete_document_by_id(doc_id)?;
                            self.write_document(doc)?;
                        }
                        crate::core::transaction::TransactionOp::Delete(doc_id) => {
                            self.delete_document_by_id(doc_id)?;
                        }
                    }
                }
                self.flush_segments()?;
                Ok(result)
            }
            Err(e) => {
                tx.rollback()?;
                Err(e)
            }
        }
    }

    pub(crate) fn collect_stats(&self) -> Result<DatabaseStats> {
        let snapshot = self.components.mvcc.current_snapshot();
        let cache_stats = self.components.cache.stats();

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

        let wal_size = self.components.writer.read().wal.position;
        let reader_segment_open_failures = self.components.reader_pool.segment_open_failure_count();

        let index_size_bytes: u64 = snapshot
            .segments
            .iter()
            .map(|seg| seg.metadata.size_bytes as u64)
            .sum();

        Ok(DatabaseStats {
            uptime_secs,
            start_time: SystemTime::now() - Duration::from_secs(uptime_secs),
            segment_count: snapshot.segments.len(),
            total_documents: snapshot.doc_count,
            deleted_documents: snapshot.deleted_docs.len() as usize,
            index_size_bytes,
            wal_size_bytes: wal_size,
            memory_pool_usage: MemoryStats {
                allocated_bytes: 0,
                used_bytes: 0,
                capacity_bytes: self.components.config.memory_limit,
                utilization_percent: 0.0,
            },
            buffer_pool_usage: BufferStats {
                page_count: 0,
                page_size: 4096,
                hit_rate: 0.0,
                dirty_pages: 0,
            },
            reader_pool_size: self.components.reader_pool.max_readers,
            reader_segment_open_failures,
            cache_stats,
            queries_per_second,
            avg_query_latency_ms: 0.0,
            writes_per_second,
            pending_writes: 0,
            last_flush_time: self.last_flush_time.read().clone(),
            last_commit_time: self.last_commit_time.read().clone(),
        })
    }

    pub(crate) fn run_health_check(&self) -> Result<HealthCheckResult> {
        let mut checks = Vec::new();
        let _start = Instant::now();

        let wal_check_start = Instant::now();
        let wal_status = match self.components.writer.try_write() {
            Some(_) => HealthStatus::Healthy,
            None => HealthStatus::Degraded("WAL is locked".to_string()),
        };
        checks.push(HealthCheck {
            name: "WAL".to_string(),
            status: wal_status,
            message: None,
            latency_ms: wal_check_start.elapsed().as_millis() as u64,
        });

        let reader_check_start = Instant::now();
        let reader_failures = self.components.reader_pool.segment_open_failure_count();
        let reader_status = match self.components.reader_pool.get_reader() {
            Ok(_) if reader_failures > 0 => HealthStatus::Degraded(format!(
                "Reader has skipped {} segment open failures",
                reader_failures
            )),
            Ok(_) => HealthStatus::Healthy,
            Err(e) => HealthStatus::Unhealthy(format!("Cannot get reader: {}", e)),
        };
        checks.push(HealthCheck {
            name: "ReaderPool".to_string(),
            status: reader_status,
            message: Some(format!("segment_open_failures={}", reader_failures)),
            latency_ms: reader_check_start.elapsed().as_millis() as u64,
        });

        let cache_check_start = Instant::now();
        let cache_status = HealthStatus::Healthy;
        checks.push(HealthCheck {
            name: "QueryCache".to_string(),
            status: cache_status,
            message: Some(format!(
                "Hit rate: {:.2}%",
                self.components.cache.stats().hit_rate() * 100.0
            )),
            latency_ms: cache_check_start.elapsed().as_millis() as u64,
        });

        let disk_check_start = Instant::now();
        checks.push(HealthCheck {
            name: "DiskSpace".to_string(),
            status: HealthStatus::Healthy,
            message: None,
            latency_ms: disk_check_start.elapsed().as_millis() as u64,
        });

        let memory_check_start = Instant::now();
        if let Some(pressure) = self.get_memory_pressure() {
            let memory_status = if pressure > 0.9 {
                HealthStatus::Unhealthy(format!(
                    "Memory pressure critical: {:.1}%",
                    pressure * 100.0
                ))
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

        let overall_status = if checks.iter().all(|c| c.status == HealthStatus::Healthy) {
            HealthStatus::Healthy
        } else if checks
            .iter()
            .any(|c| matches!(c.status, HealthStatus::Unhealthy(_)))
        {
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

    pub(crate) fn enable_low_memory_mode(&self, config: LowMemoryConfig) {
        use crate::memory::low_memory::LowMemoryMode;
        let low_mem = LowMemoryMode::new(config);
        *self.components.low_memory.lock() = Some(Arc::new(RwLock::new(low_mem)));
    }

    pub(crate) fn get_memory_pressure(&self) -> Option<f32> {
        self.components
            .low_memory
            .lock()
            .as_ref()
            .map(|lm| lm.read().memory_pressure())
    }

    pub(crate) fn maybe_reclaim_memory(&self) -> Result<()> {
        if let Some(low_mem) = self.components.low_memory.lock().as_ref().cloned() {
            let mut lm = low_mem.write();
            lm.maybe_reclaim()?;
        }
        Ok(())
    }
}

fn operations_after_last_commit(entries: Vec<WALEntry>) -> Vec<Operation> {
    let start = entries
        .iter()
        .rposition(|entry| matches!(entry.operation, Operation::Commit))
        .map(|idx| idx + 1)
        .unwrap_or(0);

    entries
        .into_iter()
        .skip(start)
        .map(|entry| entry.operation)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::Config;
    use crate::core::error::ErrorKind;
    use crate::core::types::FieldValue;
    use crate::core::types::{DocId, Document};
    use crate::memory::low_memory::LowMemoryConfig;
    use crate::schema::schema::SchemaWithAnalyzer;
    use crate::storage::segment::{Segment, SegmentId, SegmentMetadata};
    use chrono::Utc;
    use std::collections::HashMap;

    fn doc(id: u64) -> Document {
        Document {
            id: DocId(id),
            fields: HashMap::new(),
        }
    }

    #[test]
    fn operations_after_last_commit_returns_only_uncommitted_tail() {
        let entries = vec![
            WALEntry {
                sequence: 0,
                operation: Operation::AddDocument(doc(1)),
                timestamp: Utc::now(),
            },
            WALEntry {
                sequence: 1,
                operation: Operation::Commit,
                timestamp: Utc::now(),
            },
            WALEntry {
                sequence: 2,
                operation: Operation::DeleteDocument(DocId(2)),
                timestamp: Utc::now(),
            },
        ];

        let operations = operations_after_last_commit(entries);
        assert_eq!(operations.len(), 1);
        assert!(matches!(operations[0], Operation::DeleteDocument(DocId(2))));
    }

    #[test]
    fn operations_after_last_commit_empty_for_clean_commit() {
        let entries = vec![
            WALEntry {
                sequence: 0,
                operation: Operation::AddDocument(doc(1)),
                timestamp: Utc::now(),
            },
            WALEntry {
                sequence: 1,
                operation: Operation::Commit,
                timestamp: Utc::now(),
            },
        ];

        let operations = operations_after_last_commit(entries);
        assert!(operations.is_empty());
    }

    #[test]
    fn write_document_propagates_out_of_memory_error() {
        let temp_dir = tempfile::tempdir().unwrap();
        let mut config = Config::default();
        config.storage_path = temp_dir.path().to_path_buf();
        let engine = SearchEngine::new(SchemaWithAnalyzer::new(), config).unwrap();

        engine.enable_low_memory_mode(LowMemoryConfig {
            heap_limit: 64,
            buffer_size: 1024,
            cache_size: 1024,
            batch_size: 1,
            enable_compression: true,
            swap_to_disk: false,
            gc_threshold: 0.9,
        });

        let large_doc = Document {
            id: DocId(99),
            fields: HashMap::from([(
                "content".to_string(),
                FieldValue::Text("this payload exceeds the tiny heap limit".to_string()),
            )]),
        };

        let err = engine.write_document(large_doc).unwrap_err();
        assert!(matches!(err.kind, ErrorKind::OutOfMemory));
    }

    #[test]
    fn stats_and_health_expose_reader_segment_open_failures() {
        let temp_dir = tempfile::tempdir().unwrap();
        let mut config = Config::default();
        config.storage_path = temp_dir.path().to_path_buf();
        let engine = SearchEngine::new(SchemaWithAnalyzer::new(), config).unwrap();

        let missing_segment = Arc::new(Segment {
            id: SegmentId::new(),
            doc_count: 1,
            metadata: SegmentMetadata {
                created_at: Utc::now(),
                size_bytes: 0,
                min_doc_id: DocId(1),
                max_doc_id: DocId(1),
            },
        });
        engine
            .components
            .mvcc
            .create_snapshot(vec![missing_segment]);

        let _ = engine.components.reader_pool.get_reader();

        let stats = engine.collect_stats().unwrap();
        assert_eq!(stats.reader_segment_open_failures, 1);

        let health = engine.run_health_check().unwrap();
        let reader = health
            .checks
            .iter()
            .find(|check| check.name == "ReaderPool")
            .unwrap();
        assert!(matches!(
            reader.status,
            crate::core::stats::HealthStatus::Degraded(_)
        ));
        assert_eq!(reader.message.as_deref(), Some("segment_open_failures=1"));
    }
}
