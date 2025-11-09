use std::sync::Arc;
use crate::memory::adaptive::AdaptiveManager;
use crate::memory::pool::MemoryTracker;
use crate::memory::swap::SwapManager;
use crate::core::error::Result;

/// Configuration for low memory mode
#[derive(Debug, Clone)]
pub struct LowMemoryConfig {
    pub heap_limit: usize,           // Max heap usage (e.g., 50MB)
    pub buffer_size: usize,          // I/O buffer size
    pub cache_size: usize,           // Page cache size
    pub batch_size: usize,           // Processing batch size
    pub enable_compression: bool,    // Force compression
    pub swap_to_disk: bool,         // Use disk for overflow
    pub gc_threshold: f32,          // GC trigger (0.8 = 80%)
}

impl Default for LowMemoryConfig {
    fn default() -> Self {
        LowMemoryConfig {
            heap_limit: 50 * 1024 * 1024,  // 50MB
            buffer_size: 4 * 1024,          // 4KB buffers
            cache_size: 5 * 1024 * 1024,    // 5MB cache
            batch_size: 100,                // Small batches
            enable_compression: true,
            swap_to_disk: true,
            gc_threshold: 0.8,
        }
    }
}

/// Low memory database mode
pub struct LowMemoryMode {
    pub config: LowMemoryConfig,
    pub memory_tracker: Arc<MemoryTracker>,
    pub adaptive_manager: AdaptiveManager,
    pub swap_manager: SwapManager,
}

impl LowMemoryMode {
    pub fn new(config: LowMemoryConfig) -> Self {
        LowMemoryMode {
            config: config.clone(),
            memory_tracker: Arc::new(MemoryTracker::new(config.heap_limit)),
            adaptive_manager: AdaptiveManager::new(config.clone()),
            swap_manager: SwapManager::new(),
        }
    }

    /// Check if running in low memory mode
    pub fn is_enabled(&self) -> bool {
        self.config.heap_limit < 100 * 1024 * 1024
    }

    /// Get current memory pressure
    pub fn memory_pressure(&self) -> f32 {
        let used = self.memory_tracker.current_usage();
        let limit = self.config.heap_limit;
        used as f32 / limit as f32
    }

    /// Trigger memory reclamation if needed
    pub fn maybe_reclaim(&mut self) -> Result<()> {
        if self.memory_pressure() > self.config.gc_threshold {
            self.reclaim_memory()?;
        }
        Ok(())
    }

    fn reclaim_memory(&mut self) -> Result<()> {
        // 1. Clear caches
        self.adaptive_manager.clear_caches();

        // 2. Flush buffers
        self.adaptive_manager.flush_buffers()?;

        // 3. Swap cold data to disk
        if self.config.swap_to_disk {
            self.swap_manager.swap_cold_data()?;
        }

        // 4. Run garbage collection
        self.force_gc();

        Ok(())
    }

    fn force_gc(&self) {
        // Hint to allocator to release memory
        #[cfg(not(target_env = "msvc"))]
        unsafe {
            libc::malloc_trim(0);
        }
    }
}