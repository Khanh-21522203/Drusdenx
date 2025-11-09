use std::time::SystemTime;
use serde::{Serialize, Deserialize};
use crate::query::cache::CacheStats;

/// Database statistics for monitoring
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseStats {
    // General info
    pub uptime_secs: u64,
    pub start_time: SystemTime,
    
    // Storage metrics
    pub segment_count: usize,
    pub total_documents: usize,
    pub deleted_documents: usize,
    pub index_size_bytes: u64,
    pub wal_size_bytes: u64,
    
    // Memory metrics
    pub memory_pool_usage: MemoryStats,
    pub buffer_pool_usage: BufferStats,
    pub reader_pool_size: usize,
    
    // Query metrics
    pub cache_stats: CacheStats,
    pub queries_per_second: f64,
    pub avg_query_latency_ms: f64,
    
    // Write metrics
    pub writes_per_second: f64,
    pub pending_writes: usize,
    pub last_flush_time: Option<SystemTime>,
    pub last_commit_time: Option<SystemTime>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryStats {
    pub allocated_bytes: usize,
    pub used_bytes: usize,
    pub capacity_bytes: usize,
    pub utilization_percent: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BufferStats {
    pub page_count: usize,
    pub page_size: usize,
    pub hit_rate: f32,
    pub dirty_pages: usize,
}

/// Health check status
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum HealthStatus {
    Healthy,
    Degraded(String),
    Unhealthy(String),
}

impl HealthStatus {
    pub fn is_healthy(&self) -> bool {
        matches!(self, HealthStatus::Healthy)
    }
}

/// Health check result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthCheckResult {
    pub status: HealthStatus,
    pub checks: Vec<HealthCheck>,
    pub timestamp: SystemTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthCheck {
    pub name: String,
    pub status: HealthStatus,
    pub message: Option<String>,
    pub latency_ms: u64,
}
