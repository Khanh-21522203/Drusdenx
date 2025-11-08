use std::collections::{HashMap, HashSet};
use memmap2::{Mmap, MmapOptions};
use std::fs::File;
use std::path::Path;
use std::sync::{Arc};
use parking_lot::RwLock;
use crate::core::error::Result;

/// Page identifier
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PageId {
    pub segment_id: u32,
    pub page_num: u32,
}

/// Page data
pub struct Page {
    pub id: PageId,
    pub data: Vec<u8>,
}

const PAGE_SIZE: usize = 4096;

/// Memory-mapped file for zero-copy reads
pub struct MmapFile {
    pub mmap: Mmap,
    pub len: usize,
    pub read_only: bool,
}

impl MmapFile {
    pub fn open_read_only<P: AsRef<Path>>(path: P) -> Result<Self> {
        let file = File::open(&path)?;
        let metadata = file.metadata()?;
        let len = metadata.len() as usize;

        let mmap = unsafe { MmapOptions::new().len(len).map(&file)? };

        Ok(MmapFile { mmap, len, read_only: true })
    }

    pub fn data(&self) -> &[u8] {
        &self.mmap[..]
    }
}

/// Page cache for frequently accessed pages
pub struct PageCache {
    pub pages: Arc<RwLock<HashMap<PageId, Arc<Page>>>>,
    pub dirty_pages: Arc<RwLock<HashSet<PageId>>>,
    pub max_pages: usize,
}

impl PageCache {
    pub fn get_page(&self, id: PageId, mmap: &MmapFile) -> Arc<Page> {
        {
            let pages = self.pages.read();
            if let Some(page) = pages.get(&id) {
                return Arc::clone(page);
            }
        }

        // Load from mmap
        let offset = id.page_num as usize * PAGE_SIZE;
        let mut data = vec![0u8; PAGE_SIZE];
        data.copy_from_slice(&mmap.data()[offset..offset + PAGE_SIZE]);

        let page = Arc::new(Page { id, data });

        let mut pages = self.pages.write();
        pages.insert(id, Arc::clone(&page));
        page
    }
}