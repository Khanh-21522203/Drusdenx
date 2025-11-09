use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use crate::core::error::{Error, ErrorKind, Result};

/// Block identifier
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BlockId(pub usize);

/// Memory pool for reducing allocations
pub struct MemoryPool {
    pub blocks: Vec<Block>,
    pub free_list: VecDeque<BlockId>,
    pub total_size: AtomicUsize,
    pub used_size: AtomicUsize,
}

pub struct Block {
    pub ptr: *mut u8,
    pub size: usize,
    pub in_use: AtomicBool,
}

// Mark MemoryPool as Send + Sync since we're using atomic operations for thread safety
unsafe impl Send for MemoryPool {}
unsafe impl Sync for MemoryPool {}

impl MemoryPool {
    pub fn allocate(&mut self, size: usize) -> Option<*mut u8> {
        // Find free block
        for block in &self.blocks {
            if block.size >= size && !block.in_use.load(Ordering::Acquire) {
                block.in_use.store(true, Ordering::Release);
                return Some(block.ptr);
            }
        }
        None
    }

    pub fn deallocate(&mut self, ptr: *mut u8) {
        for block in &self.blocks {
            if block.ptr == ptr {
                block.in_use.store(false, Ordering::Release);
                break;
            }
        }
    }

    pub fn new(num_blocks: usize, block_size: usize) -> Self {
        let mut blocks = Vec::new();
        let mut free_list = VecDeque::new();

        for i in 0..num_blocks {
            let ptr = unsafe {
                std::alloc::alloc(std::alloc::Layout::from_size_align_unchecked(block_size, 8))
            };
            blocks.push(Block {
                ptr,
                size: block_size,
                in_use: AtomicBool::new(false),
            });
            free_list.push_back(BlockId(i));
        }

        MemoryPool {
            blocks,
            free_list,
            total_size: AtomicUsize::new(num_blocks * block_size),
            used_size: AtomicUsize::new(0),
        }
    }
}

impl Drop for MemoryPool {
    fn drop(&mut self) {
        for block in &self.blocks {
            unsafe {
                std::alloc::dealloc(
                    block.ptr,
                    std::alloc::Layout::from_size_align_unchecked(block.size, 8)
                );
            }
        }
    }
}

/// Memory usage tracker
pub struct MemoryTracker {
    pub usage: AtomicUsize,
    pub limit: usize,
}

impl MemoryTracker {
    pub fn new(limit: usize) -> Self {
        MemoryTracker {
            usage: AtomicUsize::new(0),
            limit,
        }
    }

    pub fn allocate(&self, size: usize) -> Result<()> {
        let new_usage = self.usage.fetch_add(size, Ordering::SeqCst) + size;
        if new_usage > self.limit {
            self.usage.fetch_sub(size, Ordering::SeqCst);
            return Err(Error::new(ErrorKind::OutOfMemory, "Memory limit exceeded".to_string()));
        }
        Ok(())
    }

    pub fn deallocate(&self, size: usize) {
        self.usage.fetch_sub(size, Ordering::SeqCst);
    }

    pub fn current_usage(&self) -> usize {
        self.usage.load(Ordering::Acquire)
    }
}