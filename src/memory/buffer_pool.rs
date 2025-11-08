use std::collections::{HashMap, VecDeque};
use std::sync::atomic::AtomicUsize;
use parking_lot::RwLock;

/// Buffer pool for memory reuse
pub struct BufferPool {
    pub pools: HashMap<usize, BufferQueue>,
    pub total_memory: AtomicUsize,
    pub memory_limit: usize,
}

pub struct BufferQueue {
    buffers: VecDeque<Vec<u8>>,
    size_class: usize,
}

impl BufferQueue {
    pub fn new(size_class: usize) -> BufferQueue {
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
            pools,
            total_memory: AtomicUsize::new(0),
            memory_limit,
        }
    }

    pub fn get(&mut self, size: usize) -> Vec<u8> {
        let size_class = size.next_power_of_two();

        if let Some(queue) = self.pools.get_mut(&size_class) {
            if let Some(buf) = queue.buffers.pop_front() {
                return buf;
            }
        }

        vec![0u8; size_class]
    }

    pub fn return_buffer(&mut self, mut buf: Vec<u8>) {
        let size_class = buf.capacity().next_power_of_two();
        buf.clear();

        if let Some(queue) = self.pools.get_mut(&size_class) {
            if queue.buffers.len() < 100 {
                queue.buffers.push_back(buf);
            }
        }
    }
}