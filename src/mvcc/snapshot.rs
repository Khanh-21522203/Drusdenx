/// Newtype wrapper for snapshot version numbers.
/// Provides type safety over raw u64 version numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Version(pub u64);

/// Internal RAII pin that prevents a snapshot version from being GC'd.
///
/// `MVCCController` holds a `Weak<SnapshotLease>` per version.
/// When all `Snapshot` clones (which hold `Arc<SnapshotLease>`) are dropped,
/// `Weak::strong_count() == 0` and the version is eligible for GC.
#[derive(Debug)]
pub(crate) struct SnapshotLease {
    /// Pinned version — prevents GC of this snapshot in `MVCCController`.
    #[allow(dead_code)]
    pub(crate) version: Version,
}
