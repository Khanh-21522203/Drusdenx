# Drusdenx — Architecture Evolution Plan

## Background

**What Drusdenx is today:**
An embedded, single-process full-text search library (like a mini-Tantivy). The "master/slave" topology in `database_rw.rs` is **purely an in-process illusion** — all readers share the same underlying `MVCCController`, `ReaderPool`, and `IndexWriter`. There is no network, no gossip, no replication.

Key data flows:
1. **Write path**: `SearchIndex::add_document` → `SearchEngine::write_document` → `IndexWriter::add_document` → WAL append + `SegmentWriter` → `MVCCController::create_snapshot`
2. **Read path**: `SearchIndex::search` → `SearchEngine::run_search` → `QueryCache` → `ReaderPool::get_reader` → `QueryExecutor::execute` → `TopKCollector`
3. **Transaction path**: `with_transaction` → staged `TransactionOp` list → optimistic conflict check → apply all ops via write path → `flush`
4. **MVCC**: Snapshot versioning with lease-based GC. Every write creates a new snapshot; readers pin snapshots via `Arc<SnapshotLease>`.

---

## User Review Required

> [!IMPORTANT]
> Both proposals below are **significant** architectural shifts. This plan is a design recommendation. No code will change until you approve a specific direction.

> [!WARNING]
> **Dynamo-style cluster**: This fundamentally changes the operational model. Dynamo is AP — under partition, writes are accepted on any quorum-reachable node and may diverge. Conflict resolution (vector clock + LWW) means some concurrent writes to the same `DocId` will silently drop the "losing" version.

> [!CAUTION]
> **Observer pattern**: The write path (WAL → segment → MVCC snapshot) is inherently synchronous and sequential by design. Forcing async observers into that chain can introduce partial-write visibility bugs. The plan explicitly keeps that flow sync.

---

## Proposal 1: Embedded → Dynamo-Style AP Cluster

### Core Dynamo properties

Amazon Dynamo's five pillars, and how they map to Drusdenx:

| Dynamo Pillar | What it means | Drusdenx mapping |
|---|---|---|
| **Consistent hashing** | Documents partitioned across nodes by key hash | `DocId` hash → position on ring → responsible node(s) |
| **Sloppy quorum** | Write/read needs W/R of N preference-list nodes, even across failures | Write to W coordinator + replicas; read from R nodes |
| **Hinted handoff** | If a node is down, a non-owner node accepts writes with a "hint" to forward later | `HintedWrite { target_node, doc, vector_clock }` stored locally |
| **Merkle tree anti-entropy** | Background repair: nodes compare subtree hashes to find diverged segments | One Merkle tree per virtual node (vnode); hash over `(DocId, vector_clock)` pairs |
| **Vector clocks + conflict resolution** | Track causality; detect true conflicts vs stale reads | `VectorClock { node_id → counter }` attached to each `Document`; LWW or caller-defined merge |

### Consistent hashing ring

```
                   0
              ┌────┴────┐
         315° │ Node C  │ 45°
              │ (vnodes)│
     270° ────┤         ├──── 90°
              │ Node A  │
         225° │ (vnodes)│ 135°
              └────┬────┘
                   │ 180°
                Node B (vnodes)
```

- Ring is divided into **virtual nodes (vnodes)** — each physical node owns several non-contiguous vnodes for load balance.
- `DocId` is hashed (e.g., `xxhash64`) onto the ring; the **preference list** = next N distinct physical nodes clockwise.
- Rebalancing on join/leave: only the vnodes of the joining/leaving node need data transfer.

**N, W, R defaults (tunable per request):**

| Parameter | Recommended default | Meaning |
|---|---|---|
| N | 3 | Replication factor |
| W | 2 | Writes must reach W nodes before ack |
| R | 2 | Reads must contact R nodes; merge responses |

W + R > N ensures read-your-writes for non-partitioned cases. Under partition, sloppy quorum allows W from any reachable nodes (not necessarily the preference list).

### Write path (Dynamo-style)

```
Client
  │ add_document(doc_id=42, doc, vc=∅)
  ▼
Coordinator Node (owns hash(42))
  ├── Increment own vector clock entry: vc[self] += 1
  ├── Append to local WAL + SegmentWriter (existing SearchIndex)
  ├── Send parallel RPC to (N-1) replica nodes
  │     └── Each replica: WAL append + SegmentWriter + ack
  ├── Wait for (W-1) acks  ← sloppy quorum
  │     └── If a replica is down: store HintedWrite locally
  └── Return Ok to client
```

### Read path (Dynamo-style)

```
Client
  │ search("rust", doc_id=42)
  ▼
Coordinator
  ├── Fan out read to R nodes from preference list
  ├── Collect (doc, vector_clock) responses
  ├── Compare vector clocks:
  │     ├── One dominates → return it (stale replica, no conflict)
  │     └── Concurrent (neither dominates) → CONFLICT → resolve
  │           └── Resolution: LWW (higher timestamp wins) or custom merge
  ├── Read-repair: push winning version back to stale replicas (async)
  └── Return resolved result to client
```

### Conflict resolution strategy

Dynamo doesn't mandate a single conflict resolution; you choose. For a **search index**, the practical options are:

| Strategy | How it works | Recommended for |
|---|---|---|
| **LWW (Last-Write-Wins) with HLC** | Compare `(physical_ms, logical_counter)` in vector clock; higher wins. Ties broken by node ID. | **Default** — simple, correct for most search/indexing use cases where the latest version is always preferred |
| **Merge by field** | On conflict, take the union of non-null fields; for same field, LWW applies per field | Useful if documents are sparse and different nodes write disjoint fields |
| **Caller-supplied resolver** | Application provides a `fn resolve(DocId, Vec<(Document, VectorClock)>) -> Document` | For domains where business logic determines which write wins |

**Recommendation**: Start with **LWW + HLC**. It covers 95% of search index use cases (last-indexed version is ground truth) and is trivially correct. Expose the resolver trait so callers can override it.

### How the local index is handled

Unlike CRDT, **Dynamo does not require a special data structure for the index itself**. Each node:

1. Owns a subset of `DocId`s (its vnode ranges)
2. Maintains a normal `SearchIndex` (existing Drusdenx) scoped to its owned documents
3. On write, applies the resolved document to its local `SearchIndex`
4. On **read-repair**, applies the winning document (potentially deleting the loser)

**Scatter-gather for search queries**: Since documents are partitioned, a search query must fan out to *all* nodes (or a replica subset), collect `ScoredDocument` lists, merge by score, deduplicate, and return the global top-K. The coordinator handles this merge.

```
SearchQuery("rust", top_k=10)
  ├── Fan out to all N nodes (or all R replicas per partition)
  ├── Each node: execute local SearchIndex::search("rust", limit=10)
  ├── Coordinator: merge all scored lists → global re-rank → top-10
  └── Return to client
```

### Hinted handoff

When a write coordinator cannot reach a preference-list node for a `DocId`:

```rust
struct HintedWrite {
    target_node: NodeId,      // who should eventually receive this
    doc: Document,
    vector_clock: VectorClock,
    written_at: HlcTimestamp,
    expires_at: HlcTimestamp, // TTL — drop if node stays dead too long
}
```

- Stored in a local **hint queue** (a small append-only file)
- Background task: when `target_node` recovers (detected via gossip heartbeat), drain the hint queue → forward writes → target applies them
- If expired: log a warning; document is lost on that replica until next Merkle repair

### Merkle tree anti-entropy

Each vnode maintains a **Merkle tree** over its document set:

```
Merkle(vnode)
  └── hash( sorted [ (DocId, hash(doc_bytes, vector_clock)) ... ] )
```

Background anti-entropy task (runs every ~30s):
1. For each vnode, exchange root hash with the replica peer
2. If hashes match → in sync, done
3. If mismatch → bisect the tree to find diverged leaf ranges
4. Sync only the diverged `(DocId, Document, VectorClock)` records
5. Apply via read-repair / hinted-handoff replay

This is efficient: a fully-synced cluster exchanges only `O(vnodes)` hashes per round, not full document sets.

### Cluster topology diagram

```
┌─────────────────────────────────────────────────────────────┐
│  Client (gRPC / HTTP)                                       │
└───────────────────────┬─────────────────────────────────────┘
                        │ hash(DocId) → coordinator
          ┌─────────────┼─────────────┐
          ▼             ▼             ▼
       Node A         Node B         Node C
    ┌──────────┐   ┌──────────┐   ┌──────────┐
    │VectorClk │   │VectorClk │   │VectorClk │
    │HintQueue │   │HintQueue │   │HintQueue │
    │Merkle    │◄──│ Gossip   │──►│Merkle    │
    │SearchIdx │   │Heartbeat │   │SearchIdx │
    │(vnodes)  │   └──────────┘   │(vnodes)  │
    └──────────┘                  └──────────┘
         │ replication RPC (W=2)       │
         └─────────────────────────────┘
```

### What stays the same

- `IndexWriter`, `SegmentWriter`, `WAL`, `MVCCController` — all unchanged per node
- `SearchIndex` / `SearchEngine` / `ReaderPool` — unchanged; cluster layer sits above
- `MasterSlaveDatabase` in `database_rw.rs` — can be repurposed or removed

### New components

| New component | Role |
|---|---|
| `cluster::Ring` | Consistent hash ring; vnode assignment; preference list |
| `cluster::VectorClock` | `HashMap<NodeId, u64>` with partial-order comparison |
| `cluster::HlcTimestamp` | Hybrid Logical Clock for LWW tiebreaking |
| `cluster::Coordinator` | Routes reads/writes to correct vnodes; scatter-gather search |
| `cluster::ReplicaClient` | gRPC/TCP client to peer nodes |
| `cluster::HintQueue` | Durable local queue for hinted handoff |
| `cluster::MerkleTree` | Per-vnode Merkle tree over `(DocId, VectorClock)` |
| `cluster::AntiEntropy` | Background Merkle sync task |
| `cluster::ConflictResolver` | Trait + LWW-HLC default impl |
| `cluster::GossipHeartbeat` | Failure detection for hinted handoff drain |

### New dependencies

```toml
tokio = { version = "1", features = ["full"] }   # already present
tonic = "0.12"          # gRPC for inter-node RPC
prost = "0.13"          # protobuf codegen
xxhash-rust = "0.8"     # fast consistent hash
```

---

## Proposal 2: Observer Pattern Refactoring

### Flows eligible for observer wiring

| Flow | Why it can be observer-driven | Observer event |
|---|---|---|
| **Query cache invalidation** | After a write commit, cache must be invalidated; currently **nothing does this** (bug!) | `Event::SnapshotPublished(version)` |
| **Stats / metrics collection** | Currently polled; can be push-driven | `Event::DocumentWritten`, `Event::SearchExecuted` |
| **Health check sub-checks** | Sub-checks register as observers | `Event::HealthCheckRequested` |
| **Merge policy trigger** | After flush, evaluate merge; currently ad-hoc | `Event::SegmentFlushed(segment_id)` |
| **Low-memory reclaim** | After write, check pressure; currently inline in `write_document` | `Event::MemoryPressureChanged(ratio)` |
| **Segment GC / compaction** | After commit, schedule background compact | `Event::CommitCompleted` |
| **Hinted handoff drain** *(cluster)* | When a peer recovers, drain its hint queue | `Event::PeerRecovered(node_id)` |
| **Read-repair** *(cluster)* | After a read detects a stale replica, push repair | `Event::StaleReplicaDetected { node_id, doc_id }` |

### Flows that MUST stay synchronous (do not observer-ify)

| Flow | Why |
|---|---|
| **WAL append** | Must succeed before returning `Ok`. Failure = data loss. |
| **`SegmentWriter::write_document`** | Must be atomic with WAL append |
| **`MVCCController::create_snapshot`** | Readers must see the new version immediately after write returns |
| **Transaction commit → apply ops** | Ordered application; observer delivery is unordered |
| **Quorum ack collection** *(cluster)* | Coordinator must wait for W acks synchronously before returning Ok |
| **Error propagation on write** | `?` propagates errors to caller; async observers cannot do this |

### Proposed event bus design

```rust
// src/core/events.rs

pub trait EventHandler: Send + Sync {
    fn on_event(&self, event: &EngineEvent);
}

#[non_exhaustive]
pub enum EngineEvent {
    // Write path
    DocumentWritten   { doc_id: DocId, version: u64 },
    DocumentDeleted   { doc_id: DocId, version: u64 },
    SegmentFlushed    { segment_id: SegmentId },
    SnapshotPublished { version: u64, segment_count: usize },
    CommitCompleted   { version: u64 },
    // Read path
    SearchExecuted    { query: String, took_ms: u64, hits: usize },
    // System
    MemoryPressureChanged { ratio: f32 },
    // Cluster (Phase 2)
    PeerRecovered     { node_id: NodeId },
    StaleReplicaDetected { node_id: NodeId, doc_id: DocId },
    HintedWriteExpired   { target_node: NodeId, doc_id: DocId },
}

/// Synchronous fan-out bus (zero allocation, blocks write path minimally).
/// Background observers should use a tokio broadcast re-export from here.
pub struct EventBus {
    handlers: Arc<RwLock<Vec<Arc<dyn EventHandler>>>>,
}

impl EventBus {
    pub fn subscribe(&self, handler: Arc<dyn EventHandler>) { ... }
    pub fn publish(&self, event: EngineEvent) {
        // Calls each handler inline — handlers must be fast/non-blocking
        for h in self.handlers.read().iter() { h.on_event(&event); }
    }
}
```

**Two-tier bus rule**:
- **Sync handlers** (inline): `CacheInvalidator`, `MetricsCollector` — fast, no I/O
- **Async handlers** (spawn tokio task inside handler): `MergePolicyTrigger`, `SegmentGCScheduler`, cluster `AntiEntropy` triggers

### Wiring locations

| Publish site | Event |
|---|---|
| `SearchEngine::write_document` after Ok | `DocumentWritten` |
| `SearchEngine::delete_document_by_id` after Ok | `DocumentDeleted` |
| `SearchEngine::flush_segments` after Ok | `SegmentFlushed` + `SnapshotPublished` |
| `SearchEngine::commit_wal` after Ok | `CommitCompleted` |
| `SearchEngine::run_search` after result | `SearchExecuted` |
| `write_document` memory pressure block | `MemoryPressureChanged` |

### Observers (replacing inline logic)

| Observer | Replaces |
|---|---|
| `CacheInvalidator` | Missing cache invalidation on write (current bug) |
| `MetricsCollector` | Inline `AtomicU64` counters in `SearchEngine` |
| `MergePolicyTrigger` | Ad-hoc merge call in writer |
| `MemoryReclaimObserver` | Inline pressure check in `write_document` |
| `SegmentGCScheduler` | Manual `compact()` calls |

---

## Open Questions

> [!IMPORTANT]
> **Q1 (cluster topology)**: Do you want **leaderless** (any node is coordinator for any DocId, purely ring-based routing) or **fixed coordinator per partition** (a designated "primary" per vnode, replicas are read-only until failover)? Leaderless is more available; fixed coordinator gives stronger per-vnode ordering.

> [!IMPORTANT]
> **Q2 (conflict resolution)**: LWW with HLC is the default recommendation. Do you want to expose the `ConflictResolver` trait to callers so they can plug in custom logic (e.g., field-level merge), or is LWW always sufficient?

> [!IMPORTANT]
> **Q3 (search scatter-gather)**: Full-text search must fan out to all partition nodes (since any node may have matching docs). Do you want the **coordinator to do the scatter-gather** (sync, simple), or a **dedicated query router** (separate process, more scalable)? For V1, coordinator scatter-gather is the right call.

> [!IMPORTANT]
> **Q4 (server vs library)**: Should the cluster node run as a **standalone gRPC server process** (nodes talk over the network), or as an **in-process multi-node library** (for testing/embedding)? A well-designed implementation can support both via a `Transport` trait.

> [!IMPORTANT]
> **Q5 (sequencing)**: Do you want **observer pattern first**, then cluster? Observer pattern makes the cluster integration much cleaner (cluster events slot directly into the existing bus). Or tackle both together?

---

## Proposed Changes Summary

### Phase 1 — Observer Pattern

#### [NEW] `src/core/events.rs`
`EngineEvent` enum, `EventHandler` trait, `EventBus`

#### [MODIFY] `src/core/components.rs`
Add `event_bus: Arc<EventBus>`; register built-in observers in `assemble`

#### [MODIFY] `src/core/engine.rs`
Publish events at write/read/flush/commit sites; remove inline metrics/pressure logic

#### [NEW] `src/core/observers/`
- `cache_invalidator.rs` — clears `QueryCache` on `SnapshotPublished`
- `metrics_collector.rs` — replaces inline atomic counters
- `merge_trigger.rs` — schedules merge on `SegmentFlushed`
- `memory_reclaim.rs` — replaces inline pressure check

---

### Phase 2 — Dynamo-Style Cluster (builds on Phase 1)

#### [NEW] `src/cluster/ring.rs`
Consistent hash ring: vnode placement, preference list, rebalance on join/leave

#### [NEW] `src/cluster/vector_clock.rs`
`VectorClock`, partial-order comparison (`dominates`, `concurrent`, `merge`)

#### [NEW] `src/cluster/hlc.rs`
Hybrid Logical Clock: `HlcTimestamp(physical_ms, logical)`, `tick()`, `receive()`

#### [NEW] `src/cluster/coordinator.rs`
Write coordinator (quorum logic, hinted handoff decision), read coordinator (scatter-gather, conflict resolution, async read-repair)

#### [NEW] `src/cluster/replica_client.rs`
gRPC/TCP client to peer nodes; wraps `tonic` stubs

#### [NEW] `src/cluster/hint_queue.rs`
Durable append-only hint store; background drain on `PeerRecovered` event

#### [NEW] `src/cluster/merkle.rs`
Per-vnode Merkle tree over `(DocId, VectorClock)`; delta-sync algorithm

#### [NEW] `src/cluster/anti_entropy.rs`
Tokio task: periodic Merkle comparison → sync diverged ranges

#### [NEW] `src/cluster/conflict.rs`
`ConflictResolver` trait + `LwwHlcResolver` default impl

#### [NEW] `src/cluster/gossip.rs`
Heartbeat gossip for failure detection (used by hinted handoff drain trigger)

#### [NEW] `src/cluster/node.rs`
`ClusterNode`: top-level struct tying ring + coordinator + local `SearchIndex` + event bus

#### [MODIFY] `src/core/events.rs`
Add cluster events: `PeerRecovered`, `StaleReplicaDetected`, `HintedWriteExpired`

#### [NEW] `proto/drusdenx.proto`
gRPC service definitions: `Write`, `Read`, `Replicate`, `AntiEntropySync`, `Heartbeat`

---

## Verification Plan

### Phase 1 — Observer
- `cargo test` — all existing tests pass
- Unit: `EventBus` delivers to all handlers, in registration order
- Integration: `write_document` → `SnapshotPublished` fires → `CacheInvalidator` clears cache

### Phase 2 — Dynamo Cluster
- **Quorum test**: N=3, W=2; kill one replica mid-write → write still succeeds; killed node recovers via hinted handoff
- **Conflict test**: partition N1 from {N2, N3}; write same `DocId` on both sides; re-join; assert all nodes converge to LWW winner
- **Merkle repair test**: manually corrupt one vnode's documents; run anti-entropy; verify divergence detected and repaired
- **Scatter-gather test**: index docs on different nodes; search returns global top-K across all partitions
- **Vector clock property tests** (`proptest`): `dominates` is transitive; `concurrent` is symmetric; `merge` is commutative and associative
