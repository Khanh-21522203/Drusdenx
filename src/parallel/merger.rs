use std::collections::HashMap;
use chrono::Utc;
use crate::core::types::DocId;
use crate::storage::segment::{Segment, SegmentId, SegmentMetadata};
use crate::core::error::Result;

/// Merge policy
pub struct MergePolicy {
    pub max_segments_per_tier: usize,
    pub max_segment_size: usize,
    pub merge_factor: usize,
}

/// Background segment merger
pub struct SegmentMerger {
    pub policy: MergePolicy,
}

impl SegmentMerger {
    pub fn select_merges(&self, segments: &[Segment]) -> Vec<Vec<SegmentId>> {
        let mut merges = Vec::new();
        let mut groups: HashMap<usize, Vec<&Segment>> = HashMap::new();

        // Group by size tier
        for segment in segments {
            let tier = (segment.metadata.size_bytes as f64).log10() as usize;
            groups.entry(tier).or_default().push(segment);
        }

        // Select merge candidates
        for (_tier, group) in groups {
            if group.len() >= self.policy.merge_factor {
                let to_merge: Vec<_> = group.into_iter()
                    .take(self.policy.merge_factor)
                    .map(|s| s.id)
                    .collect();
                merges.push(to_merge);
            }
        }

        merges
    }

    pub fn merge(&self, segments: Vec<Segment>) -> Result<Segment> {
        let total_doc_count: u32 = segments.iter().map(|s| s.doc_count).sum();
        let total_size: usize = segments.iter()
            .map(|s| s.metadata.size_bytes)
            .sum();

        // Merge process:
        // 1. Read all segments using SegmentReader
        // 2. Merge posting lists from multiple segments
        // 3. Write merged result using SegmentWriter
        // TODO: (Actual merge logic would use InvertedIndex with SkipList)

        let new_metadata = SegmentMetadata {
            created_at: Utc::now(),
            size_bytes: total_size,
            min_doc_id: segments.iter()
                .map(|s| s.metadata.min_doc_id)
                .min()
                .unwrap_or(DocId(0)),
            max_doc_id: segments.iter()
                .map(|s| s.metadata.max_doc_id)
                .max()
                .unwrap_or(DocId(0)),
        };

        Ok(Segment {
            id: SegmentId::new(),
            doc_count: total_doc_count,
            metadata: new_metadata,
        })
    }
}