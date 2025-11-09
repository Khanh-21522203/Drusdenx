use std::collections::{HashMap, VecDeque};
use std::sync::atomic::AtomicUsize;
use parking_lot::Mutex;  // Thread-safe interior mutability

/// Buffer pool for memory reuse
/// Wrapped in Arc<Mutex<>> for shared mutable access across threads
pub struct BufferPool {
    pools: Mutex<HashMap<usize, BufferQueue>>,
    total_memory: AtomicUsize,
    memory_limit: usize,
}

struct BufferQueue {
    buffers: VecDeque<Vec<u8>>,
    size_class: usize,
}

impl BufferQueue {
    fn new(size_class: usize) -> Self {
        BufferQueue {
            buffers: VecDeque::new(),
            size_class,
        }
    }
}

impl BufferPool {
    pub fn new(memory_limit: usize) -> Self {
        let mut pools = HashMap::new();

        // Pre-allocate common buffer sizes
        for size in [256, 1024, 4096, 16384, 65536] {
            pools.insert(size, BufferQueue::new(size));
        }

        BufferPool {
            pools: Mutex::new(pools),
            total_memory: AtomicUsize::new(0),
            memory_limit,
        }
    }

    /// Get buffer from pool (thread-safe)
    pub fn get(&self, size: usize) -> Vec<u8> {
        let size_class = size.next_power_of_two();

        let mut pools = self.pools.lock();
        if let Some(queue) = pools.get_mut(&size_class) {
            if let Some(buf) = queue.buffers.pop_front() {
                return buf;
            }
        }

        // Allocate new buffer if pool is empty
        vec![0u8; size_class]
    }

    /// Return buffer to pool (thread-safe)
    pub fn return_buffer(&self, mut buf: Vec<u8>) {
        let size_class = buf.capacity().next_power_of_two();
        buf.clear();

        let mut pools = self.pools.lock();
        if let Some(queue) = pools.get_mut(&size_class) {
            if queue.buffers.len() < 100 {  // Max 100 buffers per size class
                queue.buffers.push_back(buf);
            }
        }
    }
}