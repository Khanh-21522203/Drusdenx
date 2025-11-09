use std::sync::Arc;
use parking_lot::RwLock;
use crate::core::config::Config;
use crate::core::database::Database;
use crate::core::error::Result;
use crate::core::types::{Document, DocId};
use crate::search::results::ScoredDocument;
use crate::search::executor::ExecutionConfig;
use crate::query::cache::QueryCache;
use crate::reader::reader_pool::ReaderPool;
use crate::writer::index_writer::IndexWriter;
use crate::schema::schema::SchemaWithAnalyzer;

/// Read-only database handle for scaling read operations
pub struct ReadDatabase {
    reader_pool: Arc<ReaderPool>,
    query_cache: Arc<QueryCache>,
    query_executor: Arc<crate::search::executor::QueryExecutor>,
    query_parser: crate::query::parser::QueryParser,
}

impl ReadDatabase {
    /// Create read-only database from main database
    pub fn from_database(db: &Database) -> Self {
        ReadDatabase {
            reader_pool: db.reader_pool.clone(),
            query_cache: db.query_cache.clone(),
            query_executor: db.query_executor.clone(),
            query_parser: db.query_parser.clone(),
        }
    }
    
    /// Create multiple read replicas for load balancing
    pub fn create_replicas(db: &Database, count: usize) -> Vec<Self> {
        (0..count)
            .map(|_| Self::from_database(db))
            .collect()
    }
    
    /// Search with caching
    pub fn search(&self, query_str: &str) -> Result<Vec<ScoredDocument>> {
        self.search_with_limit(query_str, 10)
    }
    
    pub fn search_with_limit(&self, query_str: &str, limit: usize) -> Result<Vec<ScoredDocument>> {
        // Check cache first
        if let Some(cached_results) = self.query_cache.get_by_str(query_str, limit, 0) {
            return Ok(cached_results.hits);
        }
        
        // Parse and execute query
        let query = self.query_parser.parse(query_str)?;
        let reader = self.reader_pool.get_reader()?;
        let config = ExecutionConfig::default();
        let results = self.query_executor.execute(&reader, &query, limit, config)?;
        
        // Cache results
        self.query_cache.put_by_str(query_str, limit, 0, results.clone());
        
        Ok(results.hits)
    }
    
    /// Get reader pool stats
    pub fn reader_stats(&self) -> (usize, usize) {
        // (active_readers, max_readers)
        (0, self.reader_pool.max_readers) // TODO: Track active readers
    }
}

/// Write-only database handle for scaling write operations
pub struct WriteDatabase {
    writer: Arc<RwLock<IndexWriter>>,
}

impl WriteDatabase {
    /// Create write-only database from main database
    pub fn from_database(db: &Database) -> Self {
        WriteDatabase {
            writer: db.writer.clone(),
        }
    }
    
    /// Add document
    pub fn add_document(&self, doc: Document) -> Result<()> {
        self.writer.write().add_document(doc)
    }
    
    /// Batch add documents with parallel processing
    pub fn add_documents_batch(&self, docs: Vec<Document>) -> Result<()> {
        self.writer.write().add_documents_batch(docs)
    }
    
    /// Delete document
    pub fn delete_document(&self, doc_id: DocId) -> Result<()> {
        self.writer.write().delete_document(doc_id)
    }
    
    /// Flush to disk
    pub fn flush(&self) -> Result<()> {
        self.writer.write().flush()
    }
    
    /// Commit changes
    pub fn commit(&self) -> Result<()> {
        self.writer.write().commit()
    }
    
    /// Compact segments
    pub fn compact(&self) -> Result<()> {
        self.writer.write().compact()
    }
}

/// Load balancer for read replicas
pub struct ReadLoadBalancer {
    replicas: Vec<ReadDatabase>,
    current: std::sync::atomic::AtomicUsize,
}

impl ReadLoadBalancer {
    pub fn new(replicas: Vec<ReadDatabase>) -> Self {
        ReadLoadBalancer {
            replicas,
            current: std::sync::atomic::AtomicUsize::new(0),
        }
    }
    
    /// Round-robin load balancing
    pub fn get_replica(&self) -> &ReadDatabase {
        let index = self.current.fetch_add(1, std::sync::atomic::Ordering::Relaxed) % self.replicas.len();
        &self.replicas[index]
    }
    
    /// Execute search on least loaded replica
    pub fn search(&self, query_str: &str) -> Result<Vec<ScoredDocument>> {
        self.get_replica().search(query_str)
    }
}

/// Master-slave architecture for read/write separation
pub struct MasterSlaveDatabase {
    master: Arc<Database>,      // Handles writes and coordinates
    read_balancer: ReadLoadBalancer,  // Handles reads with load balancing
    write_db: WriteDatabase,    // Write handle
}

impl MasterSlaveDatabase {
    pub fn new(config: Config, schema: SchemaWithAnalyzer, read_replicas: usize) -> Result<Self> {
        // Create master database
        let master = Arc::new(Database::open_with_schema(schema, config)?);
        
        // Create read replicas
        let replicas = ReadDatabase::create_replicas(&master, read_replicas);
        let read_balancer = ReadLoadBalancer::new(replicas);
        
        // Create write handle
        let write_db = WriteDatabase::from_database(&master);
        
        Ok(MasterSlaveDatabase {
            master,
            read_balancer,
            write_db,
        })
    }
    
    /// Read operations go through load balancer
    pub fn search(&self, query_str: &str) -> Result<Vec<ScoredDocument>> {
        self.read_balancer.search(query_str)
    }
    
    /// Write operations go to master
    pub fn add_document(&self, doc: Document) -> Result<()> {
        self.write_db.add_document(doc)
    }
    
    pub fn delete_document(&self, doc_id: DocId) -> Result<()> {
        self.write_db.delete_document(doc_id)
    }
    
    /// Admin operations
    pub fn flush(&self) -> Result<()> {
        self.write_db.flush()
    }
    
    pub fn commit(&self) -> Result<()> {
        self.write_db.commit()
    }
    
    /// Get statistics from master
    pub fn stats(&self) -> Result<crate::core::stats::DatabaseStats> {
        self.master.stats()
    }
    
    pub fn health_check(&self) -> Result<crate::core::stats::HealthCheckResult> {
        self.master.health_check()
    }
}
