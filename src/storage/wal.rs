use std::fs::{File, OpenOptions};
use std::io::Write;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use crate::core::types::{DocId, Document};
use crate::storage::layout::StorageLayout;
use crate::core::error::Result;

/// Write-ahead log for durability
pub struct WAL {
    pub file: File,
    pub position: u64,
    pub sync_mode: SyncMode,
    pub sequence: u64,
}

#[derive(Debug, Clone, Copy)]
pub enum SyncMode {
    Immediate,  // fsync after every write
    Batch,      // fsync periodically
    None,       // Let OS handle it
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WALEntry {
    pub sequence: u64,
    pub operation: Operation,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Operation {
    AddDocument(Document),
    UpdateDocument(Document),
    DeleteDocument(DocId),
    Commit,
}

impl WAL {
    pub fn open(storage: &StorageLayout, sequence: u64) -> Result<Self> {
        let path = storage.wal_path(sequence);
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;

        Ok(WAL {
            file,
            position: 0,
            sync_mode: SyncMode::Batch,
            sequence,
        })
    }

    pub fn append(&mut self, operation: Operation) -> Result<()> {
        let entry = WALEntry {
            sequence: self.sequence,
            operation,
            timestamp: Utc::now(),
        };

        let data = bincode::serialize(&entry)?;
        let len = data.len() as u32;

        // Write length + data
        self.file.write_all(&len.to_le_bytes())?;
        self.file.write_all(&data)?;

        self.sequence += 1;
        self.position += 4 + data.len() as u64;

        // Sync based on mode
        match self.sync_mode {
            SyncMode::Immediate => self.file.sync_all()?,
            SyncMode::Batch if self.position % (1024 * 1024) == 0 => {
                self.file.sync_all()?
            },
            _ => {}
        }

        Ok(())
    }

    pub fn sync(&mut self) -> Result<()> {
        self.file.sync_all()?;
        Ok(())
    }

    pub fn rotate(&mut self, storage: &StorageLayout) -> Result<()> {
        self.sync()?;

        // Create new WAL file
        let new_wal = WAL::open(storage, self.sequence)?;
        *self = new_wal;

        Ok(())
    }
}