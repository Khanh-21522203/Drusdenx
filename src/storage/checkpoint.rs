use std::fs;
use std::fs::File;
use std::io::Read;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use crate::storage::layout::StorageLayout;
use crate::storage::segment::SegmentId;
use crate::storage::wal::{Operation, WALEntry, WAL};
use crate::core::error::Result;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Checkpoint {
    pub wal_position: u64,
    pub segments: Vec<SegmentId>,
    pub timestamp: DateTime<Utc>,
    pub doc_count: usize,
}

impl Checkpoint {
    /// Load checkpoint from disk
    pub fn load(storage: &StorageLayout) -> Result<Option<Self>> {
        let path = storage.checkpoint_path();
        if !path.exists() {
            return Ok(None);
        }

        let data = fs::read(path)?;
        let checkpoint = bincode::deserialize(&data)?;
        Ok(Some(checkpoint))
    }

    /// Save checkpoint to disk
    pub fn save(&self, storage: &StorageLayout) -> Result<()> {
        let data = bincode::serialize(self)?;
        fs::write(storage.checkpoint_path(), data)?;
        Ok(())
    }
}


pub struct RecoveryManager {
    pub wal: WAL,
    pub checkpoint: Option<Checkpoint>,
    pub storage: StorageLayout,
}

impl RecoveryManager {
    pub fn new(storage: StorageLayout) -> Result<Self> {
        let checkpoint = Self::load_checkpoint(&storage)?;
        let wal_sequence = checkpoint
            .as_ref()
            .map(|c| c.wal_position)
            .unwrap_or(0);

        let wal = WAL::open(&storage, wal_sequence)?;

        Ok(RecoveryManager {
            wal,
            checkpoint,
            storage,
        })
    }

    pub fn recover(&mut self) -> Result<Vec<Operation>> {
        if let Some(checkpoint) = &self.checkpoint {
            println!("Recovering from checkpoint at {}", checkpoint.timestamp);

            // Replay WAL from checkpoint
            let operations = self.replay_wal_from(checkpoint.wal_position)?;
            println!("Replayed {} operations", operations.len());

            return Ok(operations);
        }

        Ok(Vec::new())
    }

    fn replay_wal_from(&self, position: u64) -> Result<Vec<Operation>> {
        let mut operations = Vec::new();
        let mut file = File::open(self.storage.wal_path(position))?;

        loop {
            // Try to read entry
            let mut len_buf = [0u8; 4];
            if file.read_exact(&mut len_buf).is_err() {
                break; // End of file
            }

            let len = u32::from_le_bytes(len_buf) as usize;
            let mut entry_buf = vec![0u8; len];
            file.read_exact(&mut entry_buf)?;

            let entry: WALEntry = bincode::deserialize(&entry_buf)?;
            operations.push(entry.operation);
        }

        Ok(operations)
    }

    pub fn create_checkpoint(&mut self, segments: Vec<SegmentId>) -> Result<()> {
        let checkpoint = Checkpoint {
            wal_position: self.wal.sequence,
            segments,
            timestamp: Utc::now(),
            doc_count: 0, // Will be updated
        };

        let data = bincode::serialize(&checkpoint)?;
        fs::write(self.storage.checkpoint_path(), data)?;

        self.checkpoint = Some(checkpoint);
        Ok(())
    }

    fn load_checkpoint(storage: &StorageLayout) -> Result<Option<Checkpoint>> {
        let path = storage.checkpoint_path();
        if !path.exists() {
            return Ok(None);
        }

        let data = fs::read(path)?;
        let checkpoint = bincode::deserialize(&data)?;
        Ok(Some(checkpoint))
    }
}