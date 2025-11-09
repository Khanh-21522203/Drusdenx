use std::path::PathBuf;
use crate::compression::compress::CompressionType;

#[derive(Debug, Clone)]
pub struct Config {
    pub storage_path: PathBuf,
    pub memory_limit: usize,
    pub cache_size: usize,

    pub writer_batch_size: usize,               // WriterConfig.batch_size
    pub writer_commit_interval_secs: u64,       // WriterConfig.commit_interval
    pub writer_max_segment_size: usize,         // WriterConfig.max_segment_size

    pub max_readers: usize,                     // Max concurrent readers
    pub buffer_pool_size: Option<usize>,     // Default: 100MB
    pub indexing_threads: Option<usize>,     // Default: num_cpus
    pub compression: CompressionType,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            storage_path: PathBuf::from("./data"),
            cache_size: 10 * 1024 * 1024,              // 10MB query cache
            memory_limit: 100 * 1024 * 1024,           // 100MB (M01: general, M06: MemoryPool)

            writer_batch_size: 1000,                   // Flush every 1000 docs
            writer_commit_interval_secs: 60,           // Commit every 60 seconds
            writer_max_segment_size: 50 * 1024 * 1024, // 50MB max per segment
            max_readers: 10,                           // Max 10 concurrent readers
            buffer_pool_size: Some(100 * 1024 * 1024),
            indexing_threads: None,  // Will use num_cpus
            compression: CompressionType::LZ4,
        }
    }
}