use std::fs::{File, OpenOptions};
use std::io::{Write, Read, Seek, SeekFrom};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use crate::core::types::{DocId, Document};
use crate::storage::layout::StorageLayout;
use crate::core::error::{Result, Error, ErrorKind};

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
    
    /// Read all entries from WAL for recovery
    pub fn read_entries(&mut self) -> Result<Vec<WALEntry>> {
        let mut entries = Vec::new();
        
        // Seek to beginning of file
        self.file.seek(SeekFrom::Start(0))?;
        
        loop {
            // Try to read length
            let mut len_buf = [0u8; 4];
            match self.file.read_exact(&mut len_buf) {
                Ok(_) => {},
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                    // Reached end of file
                    break;
                },
                Err(e) => return Err(Error::new(ErrorKind::Io, format!("Failed to read WAL: {}", e))),
            }
            
            let len = u32::from_le_bytes(len_buf) as usize;
            
            // Sanity check - entry shouldn't be too large
            if len > 10_000_000 {  // 10MB max per entry
                return Err(Error::new(ErrorKind::InvalidInput, "WAL entry too large, possibly corrupted".to_string()));
            }
            
            // Read entry data
            let mut data = vec![0u8; len];
            self.file.read_exact(&mut data)?;
            
            // Deserialize entry
            match bincode::deserialize::<WALEntry>(&data) {
                Ok(entry) => entries.push(entry),
                Err(e) => {
                    // Log warning but continue - partial recovery is better than none
                    eprintln!("Warning: Failed to deserialize WAL entry: {}", e);
                    // Try to continue reading from next position
                }
            }
        }
        
        // Reset file position for future appends
        self.position = self.file.seek(SeekFrom::End(0))?;
        
        Ok(entries)
    }
    
    /// Find all WAL files for recovery
    pub fn find_wal_files(storage: &StorageLayout) -> Result<Vec<u64>> {
        let mut sequences = Vec::new();
        let wal_dir = storage.wal_dir();
        
        if wal_dir.exists() {
            for entry in std::fs::read_dir(wal_dir)? {
                let entry = entry?;
                let path = entry.path();
                
                if path.extension().and_then(|s| s.to_str()) == Some("log") {
                    // Extract sequence number from filename (format: wal_00000000.log)
                    if let Some(stem) = path.file_stem() {
                        if let Some(stem_str) = stem.to_str() {
                            if stem_str.starts_with("wal_") {
                                let seq_str = &stem_str[4..]; // Skip "wal_" prefix
                                if let Ok(seq) = seq_str.parse::<u64>() {
                                    sequences.push(seq);
                                }
                            }
                        }
                    }
                }
            }
        }
        
        // Sort sequences to process in order
        sequences.sort();
        Ok(sequences)
    }
}