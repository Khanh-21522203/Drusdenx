use crate::core::error::Result;
use crate::storage::wal::{Operation, WAL};
use crate::writer::wal_backend::WriteAheadLog;

/// Production adapter: wraps the existing `WAL` struct.
pub struct DiskWal {
    pub inner: WAL,
}

impl DiskWal {
    pub fn new(wal: WAL) -> Self {
        DiskWal { inner: wal }
    }
}

impl WriteAheadLog for DiskWal {
    fn append(&mut self, op: Operation) -> Result<()> {
        self.inner.append(op)
    }

    fn sync(&mut self) -> Result<()> {
        self.inner.sync()
    }

    fn sequence(&self) -> u64 {
        self.inner.sequence
    }
}
