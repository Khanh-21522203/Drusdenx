use std::sync::Arc;
use crate::core::config::Config;
use crate::core::engine::SearchEngine;
use crate::core::error::Result;
use crate::core::stats::{DatabaseStats, HealthCheckResult};
use crate::core::transaction::Transaction;
use crate::core::types::{Document, DocId};
use crate::memory::low_memory::LowMemoryConfig;
use crate::mvcc::controller::IsolationLevel;
use crate::schema::schema::SchemaWithAnalyzer;
use crate::search::executor::ExecutionConfig;
use crate::search::results::{ScoredDocument, SearchResults};

/// Public facade over `SearchEngine`.
/// All user-facing methods delegate to `Arc<SearchEngine>`.
/// `Clone` is cheap — just clones the `Arc`.
#[derive(Clone)]
pub struct SearchIndex(pub(crate) Arc<SearchEngine>);

impl SearchIndex {
    pub fn open(schema: SchemaWithAnalyzer, config: Config) -> Result<Self> {
        let engine = SearchEngine::new(schema, config)?;
        Ok(SearchIndex(Arc::new(engine)))
    }

    /// Backward-compatible alias for [`SearchIndex::open`].
    pub fn open_with_schema(schema: SchemaWithAnalyzer, config: Config) -> Result<Self> {
        Self::open(schema, config)
    }

    pub fn add_document(&self, doc: Document) -> Result<()> {
        self.0.write_document(doc)
    }

    pub fn delete_document(&self, id: DocId) -> Result<()> {
        self.0.delete_document_by_id(id)
    }

    pub fn delete_by_query(&self, query_str: &str) -> Result<usize> {
        self.0.delete_by_query(query_str)
    }

    pub fn compact(&self) -> Result<()> {
        self.0.compact()
    }

    pub fn flush(&self) -> Result<()> {
        self.0.flush_segments()
    }

    pub fn commit(&self) -> Result<()> {
        self.0.commit_wal()
    }

    pub fn recover(&self) -> Result<()> {
        self.0.recover()
    }

    pub fn search(&self, query: &str) -> Result<Vec<ScoredDocument>> {
        self.search_n(query, 10)
    }

    pub fn search_n(&self, query: &str, limit: usize) -> Result<Vec<ScoredDocument>> {
        let results = self.0.run_search(query, limit, ExecutionConfig::default())?;
        Ok(results.hits)
    }

    pub fn search_with_limit(&self, query: &str, limit: usize) -> Result<Vec<ScoredDocument>> {
        self.search_n(query, limit)
    }

    pub fn search_debug(&self, query_str: &str, limit: usize) -> Result<SearchResults> {
        self.0.run_search(query_str, limit, ExecutionConfig::debug())
    }

    pub fn stats(&self) -> Result<DatabaseStats> {
        self.0.collect_stats()
    }

    pub fn health_check(&self) -> Result<HealthCheckResult> {
        self.0.run_health_check()
    }

    pub fn begin_transaction(&self, isolation: IsolationLevel) -> Arc<Transaction> {
        self.0.begin_transaction(isolation)
    }

    pub fn transaction<F, R>(&self, isolation: IsolationLevel, f: F) -> Result<R>
    where
        F: FnOnce(&Transaction) -> Result<R>,
    {
        self.0.with_transaction(isolation, f)
    }

    /// Legacy compat: `with_transaction`
    pub fn with_transaction<F, R>(&self, isolation: IsolationLevel, f: F) -> Result<R>
    where
        F: FnOnce(&Transaction) -> Result<R>,
    {
        self.0.with_transaction(isolation, f)
    }

    /// Enable low-memory mode for memory-constrained environments.
    pub fn enable_low_memory_mode(&self, config: LowMemoryConfig) {
        self.0.enable_low_memory_mode(config);
    }

    /// Returns `true` if low-memory mode has been enabled.
    pub fn is_low_memory_mode_enabled(&self) -> bool {
        self.0.components.low_memory.lock().is_some()
    }

    /// Returns current memory pressure in the range `[0.0, 1.0]`, or `None`
    /// if low-memory mode is not active.
    pub fn get_memory_pressure(&self) -> Option<f32> {
        self.0.get_memory_pressure()
    }

    /// Trigger memory reclamation if memory pressure is high.
    pub fn maybe_reclaim_memory(&self) -> Result<()> {
        self.0.maybe_reclaim_memory()
    }

    /// Return a clone of the schema this index was opened with.
    pub fn schema(&self) -> crate::schema::schema::SchemaWithAnalyzer {
        self.0.components.schema.clone()
    }
}
