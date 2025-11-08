use std::fs::{File, OpenOptions};
use crate::core::error::{Error, ErrorKind, Result};
use crate::storage::layout::StorageLayout;

/// Single writer guarantee
pub struct FileLock {
    pub file: File,
    pub exclusive: bool,
}

impl FileLock {
    pub fn acquire(storage: &StorageLayout, exclusive: bool) -> Result<Self> {
        let lock_path = storage.base_dir.join(".lock");

        let file = if exclusive {
            // Exclusive write lock
            OpenOptions::new()
                .create(true)
                .write(true)
                .open(&lock_path)?
        } else {
            // Shared read lock
            OpenOptions::new()
                .read(true)
                .open(&lock_path)?
        };

        // Platform-specific locking
        #[cfg(unix)]
        {
            use std::os::unix::io::AsRawFd;
            use libc::{flock, LOCK_EX, LOCK_SH, LOCK_NB};

            let fd = file.as_raw_fd();
            let operation = if exclusive { LOCK_EX } else { LOCK_SH } | LOCK_NB;

            unsafe {
                if flock(fd, operation) != 0 {
                    return Err(Error {
                        kind: ErrorKind::Io,
                        context: "Failed to acquire lock".to_string(),
                    })
                }
            }
        }

        Ok(FileLock { file, exclusive })
    }
}

impl Drop for FileLock {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            use std::os::unix::io::AsRawFd;
            use libc::{flock, LOCK_UN};

            let fd = self.file.as_raw_fd();
            unsafe {
                flock(fd, LOCK_UN);
            }
        }
    }
}