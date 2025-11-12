use std::path::PathBuf;
use std::fs;
use crate::core::error::Result;
use crate::storage::segment::SegmentId;

/// Directory structure for data files
#[derive(Debug, Clone)]
pub struct StorageLayout {
    pub base_dir: PathBuf,      // Root directory
    pub segments_dir: PathBuf,  // Document data segments (.seg files)
    pub idx_dir: PathBuf,       // Inverted index files (.idx files)
    pub wal_dir: PathBuf,       // Write-ahead log location
    pub meta_dir: PathBuf,      // Metadata files location
}

impl StorageLayout {
    pub fn new(base_dir: PathBuf) -> Result<Self> {
        let segments_dir = base_dir.join("segments");
        let idx_dir = base_dir.join("idx");
        let wal_dir = base_dir.join("wal");
        let meta_dir = base_dir.join("meta");

        // Create directories
        fs::create_dir_all(&segments_dir)?;
        fs::create_dir_all(&idx_dir)?;
        fs::create_dir_all(&wal_dir)?;
        fs::create_dir_all(&meta_dir)?;

        Ok(StorageLayout {
            base_dir,
            segments_dir,
            idx_dir,
            wal_dir,
            meta_dir,
        })
    }

    pub fn segment_path(&self, id: &SegmentId) -> PathBuf {
        self.segments_dir.join(format!("{}.seg", id.0))
    }

    pub fn index_path(&self, id: &SegmentId) -> PathBuf {
        self.idx_dir.join(format!("{}.idx", id.0))
    }

    pub fn wal_path(&self, sequence: u64) -> PathBuf {
        self.wal_dir.join(format!("wal_{:08}.log", sequence))
    }

    pub fn checkpoint_path(&self) -> PathBuf {
        self.meta_dir.join("checkpoint.bin")
    }
    
    pub fn wal_dir(&self) -> &PathBuf {
        &self.wal_dir
    }
}