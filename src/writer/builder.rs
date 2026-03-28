use std::sync::Arc;
use crate::analysis::analyzer::Analyzer;
use crate::core::error::{Error, ErrorKind, Result};
use crate::memory::buffer_pool::BufferPool;
use crate::mvcc::controller::MVCCController;
use crate::parallel::indexer::ParallelIndexer;
use crate::storage::layout::StorageLayout;
use crate::storage::merge_policy::{MergePolicy, TieredMergePolicy};
use crate::writer::index_writer::WriterConfig;
use crate::writer::segment_store::SegmentStore;
use crate::writer::session::WriterContext;

/// Builder for `WriterContext` with injectable storage backends.
pub struct WritePipelineBuilder {
    pub storage: Option<Arc<StorageLayout>>,
    pub mvcc: Option<Arc<MVCCController>>,
    pub buffer_pool: Option<Arc<BufferPool>>,
    pub parallel_indexer: Option<Arc<ParallelIndexer>>,
    pub analyzer: Option<Arc<Analyzer>>,
    pub merge_policy: Option<Arc<dyn MergePolicy>>,
    pub config: Option<WriterConfig>,
    pub segment_store: Option<Arc<dyn SegmentStore>>,
}

impl WritePipelineBuilder {
    pub fn new() -> Self {
        WritePipelineBuilder {
            storage: None,
            mvcc: None,
            buffer_pool: None,
            parallel_indexer: None,
            analyzer: None,
            merge_policy: None,
            config: None,
            segment_store: None,
        }
    }

    pub fn storage(mut self, storage: Arc<StorageLayout>) -> Self {
        self.storage = Some(storage);
        self
    }

    pub fn mvcc(mut self, mvcc: Arc<MVCCController>) -> Self {
        self.mvcc = Some(mvcc);
        self
    }

    pub fn buffer_pool(mut self, pool: Arc<BufferPool>) -> Self {
        self.buffer_pool = Some(pool);
        self
    }

    pub fn parallel_indexer(mut self, indexer: Arc<ParallelIndexer>) -> Self {
        self.parallel_indexer = Some(indexer);
        self
    }

    pub fn analyzer(mut self, analyzer: Arc<Analyzer>) -> Self {
        self.analyzer = Some(analyzer);
        self
    }

    pub fn merge_policy(mut self, policy: impl MergePolicy + 'static) -> Self {
        self.merge_policy = Some(Arc::new(policy));
        self
    }

    pub fn config(mut self, config: WriterConfig) -> Self {
        self.config = Some(config);
        self
    }

    pub fn segment_store(mut self, store: impl SegmentStore + 'static) -> Self {
        self.segment_store = Some(Arc::new(store));
        self
    }

    pub fn build(self) -> Result<WriterContext> {
        let storage = self.storage.ok_or_else(|| {
            Error::new(ErrorKind::InvalidArgument, "storage is required".to_string())
        })?;
        let mvcc = self.mvcc.ok_or_else(|| {
            Error::new(ErrorKind::InvalidArgument, "mvcc is required".to_string())
        })?;
        let buffer_pool = self.buffer_pool.ok_or_else(|| {
            Error::new(ErrorKind::InvalidArgument, "buffer_pool is required".to_string())
        })?;
        let parallel_indexer = self.parallel_indexer.ok_or_else(|| {
            Error::new(ErrorKind::InvalidArgument, "parallel_indexer is required".to_string())
        })?;
        let analyzer = self.analyzer.ok_or_else(|| {
            Error::new(ErrorKind::InvalidArgument, "analyzer is required".to_string())
        })?;
        let segment_store = self.segment_store.ok_or_else(|| {
            Error::new(ErrorKind::InvalidArgument, "segment_store is required".to_string())
        })?;

        Ok(WriterContext {
            storage,
            mvcc,
            buffer_pool,
            parallel_indexer,
            analyzer,
            merge_policy: self.merge_policy
                .unwrap_or_else(|| Arc::new(TieredMergePolicy::default())),
            config: self.config.unwrap_or_default(),
            segment_store,
        })
    }
}

impl Default for WritePipelineBuilder {
    fn default() -> Self {
        Self::new()
    }
}
