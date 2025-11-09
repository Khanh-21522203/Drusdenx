pub mod core;
pub mod storage;
pub mod analysis;
pub mod schema;
pub mod index;
pub mod scoring;
pub mod search;
pub mod query;
pub mod mvcc;
pub mod writer;
pub mod reader;
pub mod mmap;
pub mod memory;
pub mod compression;
pub mod simd;
pub mod parallel;

/*
┌────────────────────────────────────────────────────────────────────────────────────────────┐
│                            DRUSDENX STRUCT ARCHITECTURE                                     │
└────────────────────────────────────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────── CORE LAYER ──────────────────────────────────────────┐
│                                                                                              │
│  ┌────────────────────────────────────────────────────────────────────────────────────┐    │
│  │                               struct Database                                       │    │
│  │  ┌──────────────────────────────────────────────────────────────────────────────┐ │    │
│  │  │ config: Config                    // Database configuration                  │ │    │
│  │  │ storage: Arc<StorageLayout>       // Storage management                      │ │    │
│  │  │ schema: SchemaWithAnalyzer        // Schema and text analysis               │ │    │
│  │  │ query_parser: QueryParser         // Query string parsing                    │ │    │
│  │  │ query_executor: Arc<QueryExecutor>// Stateless query execution              │ │    │
│  │  │ query_cache: Arc<QueryCache>      // Query result caching                   │ │    │
│  │  │ mvcc: Arc<MVCCController>         // Concurrency control                    │ │    │
│  │  │ writer: Arc<RwLock<IndexWriter>>  // Single writer                          │ │    │
│  │  │ reader_pool: Arc<ReaderPool>      // Pooled readers                         │ │    │
│  │  │ transaction_manager: Option<Arc<TransactionManager>>                        │ │    │
│  │  │ // Metrics                                                                   │ │    │
│  │  │ start_time: Instant                                                         │ │    │
│  │  │ query_count: AtomicU64                                                      │ │    │
│  │  │ write_count: AtomicU64                                                      │ │    │
│  │  └──────────────────────────────────────────────────────────────────────────────┘ │    │
│  └────────────────────────────────────────────────────────────────────────────────────┘    │
│                                                                                              │
│  ┌──────────────────┐  ┌──────────────────┐  ┌───────────────────────────────────────┐    │
│  │ struct Config    │  │ struct Document  │  │ struct DatabaseStats                  │    │
│  │ • storage_path   │  │ • id: DocId      │  │ • uptime_secs                         │    │
│  │ • memory_limit   │  │ • fields:        │  │ • segment_count                       │    │
│  │ • cache_size     │  │   HashMap<String,│  │ • total_documents                     │    │
│  │ • writer_config  │  │   FieldValue>    │  │ • index_size_bytes                    │    │
│  └──────────────────┘  └──────────────────┘  │ • queries_per_second                  │    │
│                                               │ • cache_stats: CacheStats             │    │
│  ┌──────────────────┐  ┌──────────────────┐  └───────────────────────────────────────┘    │
│  │ struct DocId     │  │ enum FieldValue  │                                                │
│  │ • 0: u64         │  │ • Text(String)   │  ┌───────────────────────────────────────┐    │
│  └──────────────────┘  │ • Integer(i64)   │  │ struct ReadDatabase                   │    │
│                        │ • Float(f64)     │  │ • reader_pool: Arc<ReaderPool>        │    │
│                        │ • Boolean(bool)  │  │ • query_cache: Arc<QueryCache>        │    │
│                        │ • Date(DateTime) │  │ • query_executor: Arc<QueryExecutor>  │    │
│                        └──────────────────┘  └───────────────────────────────────────┘    │
└──────────────────────────────────────────────────────────────────────────────────────────────┘

┌──────────────────────────────────── TRANSACTION LAYER ──────────────────────────────────────┐
│                                                                                              │
│  ┌────────────────────────────────────────────────────────────────────────────────────┐    │
│  │                           struct Transaction                                        │    │
│  │  ┌──────────────────────────────────────────────────────────────────────────────┐ │    │
│  │  │ id: u64                                  // Unique transaction ID            │ │    │
│  │  │ isolation_level: IsolationLevel          // ACID isolation                  │ │    │
│  │  │ state: Arc<RwLock<TransactionState>>     // Active/Preparing/Committed      │ │    │
│  │  │ operations: Arc<Mutex<Vec<TransactionOp>>> // Insert/Update/Delete ops      │ │    │
│  │  │ snapshot: Arc<Snapshot>                  // MVCC snapshot                   │ │    │
│  │  │ read_set: Arc<RwLock<HashMap>>           // Read tracking                   │ │    │
│  │  │ write_set: Arc<RwLock<HashMap>>          // Write tracking                  │ │    │
│  │  │ mvcc: Arc<MVCCController>                // Concurrency control             │ │    │
│  │  └──────────────────────────────────────────────────────────────────────────────┘ │    │
│  └────────────────────────────────────────────────────────────────────────────────────┘    │
│                                                                                              │
│  ┌────────────────────────┐  ┌─────────────────────────┐  ┌────────────────────────┐      │
│  │ struct TransactionMgr  │  │ enum TransactionState   │  │ enum TransactionOp   │      │
│  │ • active_txns: HashMap │  │ • Active                │  │ • Insert(Document)   │      │
│  │ • mvcc: Arc<MVCC>      │  │ • Preparing             │  │ • Update(DocId, Doc) │      │
│  └────────────────────────┘  │ • Committed             │  │ • Delete(DocId)      │      │
│                               │ • Aborted               │  └────────────────────────┘      │
│                               └─────────────────────────┘                                   │
└──────────────────────────────────────────────────────────────────────────────────────────────┘

┌───────────────────────────────────── INDEXING LAYER ────────────────────────────────────────┐
│                                                                                              │
│  ┌────────────────────────────────────────────────────────────────────────────────────┐    │
│  │                           struct InvertedIndex                                      │    │
│  │  ┌──────────────────────────────────────────────────────────────────────────────┐ │    │
│  │  │ dictionary: TermDictionary        // Term → TermInfo mapping                │ │    │
│  │  │ postings: HashMap<Term, PostingList> // Term → document postings            │ │    │
│  │  │ skip_lists: HashMap<Term, SkipList>  // Fast intersection support           │ │    │
│  │  │ doc_count: usize                  // Total indexed documents                │ │    │
│  │  │ total_tokens: usize               // Total token count                      │ │    │
│  │  │ prefix_index: Option<PrefixIndex> // Prefix search support                  │ │    │
│  │  └──────────────────────────────────────────────────────────────────────────────┘ │    │
│  └────────────────────────────────────────────────────────────────────────────────────┘    │
│                                                                                              │
│  ┌──────────────────┐  ┌───────────────────┐  ┌────────────────────────────────────┐      │
│  │ struct Term      │  │ struct PostingList │  │ struct Posting                     │      │
│  │ • 0: Vec<u8>     │  │ • data: Vec<u8>     │  │ • doc_id: DocId                    │      │
│  └──────────────────┘  │ • compressed: bool  │  │ • term_freq: u32                   │      │
│                        │ • doc_freq: u32     │  │ • positions: Vec<u32>              │      │
│  ┌──────────────────┐  └───────────────────┘  │ • field_norm: f32                  │      │
│  │ struct TermInfo  │                          └────────────────────────────────────┘      │
│  │ • doc_freq: u32  │  ┌───────────────────┐                                               │
│  │ • total_freq: u64│  │ struct SkipList    │  ┌────────────────────────────────────┐      │
│  │ • idf: f32       │  │ • levels: Vec<Level>│  │ struct ParallelIndexer             │      │
│  │ • posting_offset │  │ • skip_interval: u32│  │ • workers: usize                   │      │
│  │ • posting_size   │  └───────────────────┘  │ • batch_size: usize                │      │
│  └──────────────────┘                          │ • progress: Arc<AtomicUsize>       │      │
│                                                └────────────────────────────────────┘      │
└──────────────────────────────────────────────────────────────────────────────────────────────┘

┌────────────────────────────────────── SEARCH LAYER ─────────────────────────────────────────┐
│                                                                                              │
│  ┌────────────────────────────────────────────────────────────────────────────────────┐    │
│  │                           struct QueryExecutor                                      │    │
│  │  ┌──────────────────────────────────────────────────────────────────────────────┐ │    │
│  │  │ optimizer: QueryOptimizer         // Query optimization                     │ │    │
│  │  │ validator: Option<QueryValidator> // Query validation                       │ │    │
│  │  └──────────────────────────────────────────────────────────────────────────────┘ │    │
│  └────────────────────────────────────────────────────────────────────────────────────┘    │
│                                                                                              │
│  ┌─────────────────────┐  ┌──────────────────────┐  ┌────────────────────────────┐        │
│  │ struct QueryParser  │  │ struct QueryPlanner  │  │ struct QueryOptimizer      │        │
│  │ • default_field     │  │ • statistics: Stats  │  │ • rules: Vec<OptRule>      │        │
│  │ • default_operator  │  │                      │  │ • cost_model: CostModel    │        │
│  │ • allow_wildcards   │  └──────────────────────┘  └────────────────────────────┘        │
│  │ • fuzzy_enabled     │                                                                    │
│  └─────────────────────┘  ┌──────────────────────┐  ┌────────────────────────────┐        │
│                           │ struct DocumentMatch │  │ struct SearchResults       │        │
│  ┌─────────────────────┐  │ • index: Arc<Index>  │  │ • hits: Vec<ScoredDoc>     │        │
│  │ enum Query (AST)    │  └──────────────────────┘  │ • total_hits: usize        │        │
│  │ • Term(TermQuery)   │                            │ • max_score: f32           │        │
│  │ • Bool(BoolQuery)   │  ┌──────────────────────┐  │ • took_ms: u64             │        │
│  │ • Phrase(Phrase)    │  │ struct ScoredDocument│  └────────────────────────────┘        │
│  │ • Range(Range)      │  │ • doc_id: DocId       │                                         │
│  │ • Fuzzy(Fuzzy)      │  │ • score: f32          │  ┌────────────────────────────┐        │
│  │ • Wildcard(Wild)    │  │ • explanation: Option │  │ struct TopKCollector       │        │
│  │ • Prefix(Prefix)    │  └──────────────────────┘  │ • heap: BinaryHeap         │        │
│  │ • MatchAll          │                            │ • k: usize                 │        │
│  └─────────────────────┘                            │ • min_score: Option<f32>   │        │
│                                                      └────────────────────────────┘        │
└──────────────────────────────────────────────────────────────────────────────────────────────┘

┌────────────────────────────────────── SCORING LAYER ────────────────────────────────────────┐
│                                                                                              │
│  ┌─────────────────────┐  ┌─────────────────────┐  ┌────────────────────────────┐         │
│  │ struct BM25Scorer   │  │ struct TfIdfScorer  │  │ trait Scorer               │         │
│  │ • k1: f32 (1.2)     │  │ • normalize: bool   │  │ • score_term()             │         │
│  │ • b: f32 (0.75)     │  └─────────────────────┘  │ • score_phrase()           │         │
│  └─────────────────────┘                            │ • score_bool()             │         │
│                                                      │ • explain()                │         │
│  ┌─────────────────────┐  ┌─────────────────────┐  └────────────────────────────┘         │
│  │ struct DocStats     │  │ struct ScoreExplan  │                                          │
│  │ • doc_id: DocId     │  │ • value: f32        │  ┌────────────────────────────┐         │
│  │ • doc_length: usize │  │ • description: Str  │  │ struct IndexStatistics     │         │
│  │ • field_length: Map │  │ • details: Vec<>    │  │ • total_docs: usize        │         │
│  └─────────────────────┘  └─────────────────────┘  │ • avg_doc_length: f32      │         │
│                                                      │ • field_stats: HashMap     │         │
│                                                      └────────────────────────────┘         │
└──────────────────────────────────────────────────────────────────────────────────────────────┘

┌──────────────────────────────────── STORAGE LAYER ──────────────────────────────────────────┐
│                                                                                              │
│  ┌────────────────────────┐  ┌──────────────────────┐  ┌───────────────────────┐          │
│  │ struct IndexWriter     │  │ struct ReaderPool    │  │ struct WAL            │          │
│  │ • segment_writer       │  │ • readers: Vec<>     │  │ • file: File          │          │
│  │ • wal: WAL             │  │ • mvcc: Arc<MVCC>    │  │ • position: u64       │          │
│  │ • memory_pool          │  │ • max_readers: usize │  │ • sync_mode: SyncMode │          │
│  │ • config: WriterConfig │  │ • segment_cache: Map │  │ • sequence: u64       │          │
│  │ • mvcc: Arc<MVCC>      │  └──────────────────────┘  └───────────────────────┘          │
│  │ • lock: Arc<Mutex>     │                                                                 │
│  │ • merge_policy: Box<>  │  ┌──────────────────────┐  ┌───────────────────────┐          │
│  └────────────────────────┘  │ struct SegmentWriter │  │ struct SegmentReader  │          │
│                               │ • segment: Segment   │  │ • segment: Arc<Seg>   │          │
│  ┌────────────────────────┐  │ • data_writer: Write │  │ • file: Mutex<File>   │          │
│  │ struct Segment         │  │ • index_writer:Write │  │ • header: SegHeader   │          │
│  │ • id: SegmentId        │  │ • buffer_pool: Arc<> │  │ • mmap: Option<Mmap>  │          │
│  │ • doc_count: u32       │  └──────────────────────┘  └───────────────────────┘          │
│  │ • metadata: Metadata   │                                                                 │
│  └────────────────────────┘  ┌──────────────────────┐  ┌───────────────────────┐          │
│                               │ struct StorageLayout │  │ struct BufferPool     │          │
│  ┌────────────────────────┐  │ • root_dir: PathBuf  │  │ • pools: Mutex<Map>   │          │
│  │ struct SegmentId       │  │ • segments_dir: Path │  │ • total_memory: Atomic│          │
│  │ • 0: Uuid              │  │ • wal_dir: PathBuf   │  │ • memory_limit: usize │          │
│  └────────────────────────┘  └──────────────────────┘  └───────────────────────┘          │
└──────────────────────────────────────────────────────────────────────────────────────────────┘

┌───────────────────────────────────── MVCC LAYER ────────────────────────────────────────────┐
│                                                                                              │
│  ┌────────────────────────────────────────────────────────────────────────────────────┐    │
│  │                           struct MVCCController                                     │    │
│  │  ┌──────────────────────────────────────────────────────────────────────────────┐ │    │
│  │  │ versions: Arc<RwLock<BTreeMap<u64, Snapshot>>>  // Version history          │ │    │
│  │  │ active_txns: Arc<RwLock<HashSet<TxId>>>        // Active transactions       │ │    │
│  │  │ current_version: Arc<AtomicU64>                // Latest version            │ │    │
│  │  │ max_versions: usize                            // GC threshold              │ │    │
│  │  └──────────────────────────────────────────────────────────────────────────────┘ │    │
│  └────────────────────────────────────────────────────────────────────────────────────┘    │
│                                                                                              │
│  ┌────────────────────────┐  ┌─────────────────────────┐  ┌────────────────────┐          │
│  │ struct Snapshot        │  │ enum IsolationLevel     │  │ struct TxId        │          │
│  │ • version: u64         │  │ • ReadCommitted         │  │ • 0: u64           │          │
│  │ • segments: Vec<Arc>   │  │ • RepeatableRead        │  └────────────────────┘          │
│  │ • timestamp: DateTime  │  │ • Serializable          │                                   │
│  │ • doc_count: usize     │  └─────────────────────────┘                                   │
│  │ • deleted_docs: Bitmap │                                                                 │
│  └────────────────────────┘                                                                 │
└──────────────────────────────────────────────────────────────────────────────────────────────┘

┌───────────────────────────────── OPTIMIZATION LAYER ────────────────────────────────────────┐
│                                                                                              │
│  ┌────────────────────────┐  ┌──────────────────────┐  ┌─────────────────────┐            │
│  │ struct SimdOps         │  │ struct QueryCache    │  │ struct MemoryPool   │            │
│  │ • intersect_sorted()   │  │ • cache: LruCache    │  │ • arena: Vec<u8>    │            │
│  │ • union_sorted()       │  │ • size_limit: usize  │  │ • allocated: usize  │            │
│  │ • galloping_search()   │  │ • hit_count: Atomic  │  │ • limit: usize      │            │
│  └────────────────────────┘  │ • miss_count: Atomic │  └─────────────────────┘            │
│                               └──────────────────────┘                                      │
│  ┌────────────────────────┐  ┌──────────────────────┐  ┌─────────────────────┐            │
│  │ struct MergePolicy     │  │ struct QueryCacheKey │  │ struct PageCache    │            │
│  │ • TieredMergePolicy    │  │ • query_hash: u64    │  │ • pages: HashMap    │            │
│  │ • LogStructuredPolicy  │  │ • limit: usize       │  │ • eviction: LRU     │            │
│  │ • should_merge()       │  │ • offset: usize      │  │ • dirty: HashSet    │            │
│  │ • select_segments()    │  └──────────────────────┘  └─────────────────────┘            │
│  └────────────────────────┘                                                                 │
└──────────────────────────────────────────────────────────────────────────────────────────────┘

┌──────────────────────────────────── ANALYSIS LAYER ─────────────────────────────────────────┐
│                                                                                              │
│  ┌────────────────────────┐  ┌──────────────────────┐  ┌─────────────────────┐            │
│  │ struct Analyzer        │  │ struct Token         │  │ struct Tokenizer    │            │
│  │ • tokenizer: Box<>     │  │ • text: String       │  │ • StandardTokenizer │            │
│  │ • filters: Vec<Box>    │  │ • position: usize    │  │ • WhitespaceToken   │            │
│  │ • analyze()            │  │ • offset: (u32, u32) │  │ • PatternTokenizer  │            │
│  └────────────────────────┘  │ • token_type: Type   │  └─────────────────────┘            │
│                               └──────────────────────┘                                      │
│  ┌────────────────────────┐  ┌──────────────────────┐  ┌─────────────────────┐            │
│  │ trait TokenFilter      │  │ struct LowercaseFltr │  │ struct StopWordFltr │            │
│  │ • filter()             │  │ • filter()           │  │ • stop_words: Set   │            │
│  └────────────────────────┘  └──────────────────────┘  │ • filter()          │            │
│                                                         └─────────────────────┘            │
└──────────────────────────────────────────────────────────────────────────────────────────────┘

┌────────────────────────────────── RELATIONSHIPS ────────────────────────────────────────────┐
│                                                                                              │
│  Database ──owns──> IndexWriter ──uses──> SegmentWriter ──creates──> Segment                │
│     │                   │                                                                   │
│     ├──owns──> ReaderPool ──manages──> IndexReader ──reads──> SegmentReader                 │
│     │                                      │                                                │
│     ├──owns──> MVCCController ──creates──> Snapshot ──references──> Segment                 │
│     │                                                                                       │
│     ├──owns──> QueryExecutor ──uses──> QueryPlanner ──generates──> LogicalPlan             │
│     │                │                                                                      │
│     │                └──uses──> QueryOptimizer ──optimizes──> Query                        │
│     │                                                                                       │
│     ├──owns──> QueryCache ──stores──> SearchResults                                        │
│     │                                                                                       │
│     └──can_create──> Transaction ──uses──> MVCCController                                  │
│                           │                                                                 │
│                           └──commits_to──> Database                                         │
│                                                                                              │
│  InvertedIndex ──contains──> PostingList ──contains──> Posting                             │
│        │                                                                                    │
│        ├──contains──> TermDictionary ──maps──> TermInfo                                    │
│        │                                                                                    │
│        └──uses──> SimdOps ──for──> intersect_sorted/union_sorted                          │
│                                                                                              │
│  QueryExecutor ──scores_with──> BM25Scorer/TfIdfScorer ──implements──> Scorer              │
│                                                                                              │
│  ParallelIndexer ──parallelizes──> Document ──analysis──> Token ──indexing──> Term         │
│                                                                                              │
└──────────────────────────────────────────────────────────────────────────────────────────────┘
*/