use crate::core::error::Result;
use crate::storage::wal::Operation;

/// Port: Write-Ahead Log abstraction.
/// Implement to provide different WAL backends (disk, in-memory, etc.)
pub trait WriteAheadLog: Send + Sync {
    fn append(&mut self, op: Operation) -> Result<()>;
    fn sync(&mut self) -> Result<()>;
    fn sequence(&self) -> u64;
}
