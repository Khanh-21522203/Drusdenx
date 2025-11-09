use std::sync::Arc;
use crate::storage::segment::Segment;

/// Policy for deciding when and how to merge segments
pub trait MergePolicy: Send + Sync {
    /// Check if segments should be merged
    fn should_merge(&self, segments: &[Arc<Segment>]) -> bool;
    
    /// Select segments to merge
    fn select_segments_to_merge(&self, segments: &[Arc<Segment>]) -> Vec<Arc<Segment>>;
}

/// Tiered merge policy (similar to Lucene's TieredMergePolicy)
pub struct TieredMergePolicy {
    pub max_segments_per_tier: usize,
    pub max_segment_size_mb: usize,
    pub min_segments_to_merge: usize,
    pub max_segments_to_merge: usize,
}

impl Default for TieredMergePolicy {
    fn default() -> Self {
        TieredMergePolicy {
            max_segments_per_tier: 10,
            max_segment_size_mb: 512,  // 512MB max segment size
            min_segments_to_merge: 2,
            max_segments_to_merge: 10,
        }
    }
}

impl MergePolicy for TieredMergePolicy {
    fn should_merge(&self, segments: &[Arc<Segment>]) -> bool {
        // Trigger merge if we have too many segments
        if segments.len() > self.max_segments_per_tier {
            return true;
        }
        
        // Count small segments (< 10MB)
        let small_segments = segments.iter()
            .filter(|s| s.metadata.size_bytes < 10 * 1024 * 1024)
            .count();
        
        // Merge if we have many small segments
        small_segments >= self.min_segments_to_merge
    }
    
    fn select_segments_to_merge(&self, segments: &[Arc<Segment>]) -> Vec<Arc<Segment>> {
        // Sort segments by size
        let mut sorted_segments = segments.to_vec();
        sorted_segments.sort_by_key(|s| s.metadata.size_bytes);
        
        // Select small segments to merge
        let mut selected = Vec::new();
        let max_merge_size = self.max_segment_size_mb * 1024 * 1024;
        let mut current_size = 0;
        
        for segment in sorted_segments {
            // Skip large segments
            if segment.metadata.size_bytes > max_merge_size / 2 {
                continue;
            }
            
            // Check if adding this segment would exceed max size
            if current_size + segment.metadata.size_bytes > max_merge_size {
                break;
            }
            
            selected.push(segment.clone());
            current_size += segment.metadata.size_bytes;
            
            // Don't merge too many segments at once
            if selected.len() >= self.max_segments_to_merge {
                break;
            }
        }
        
        // Only merge if we have enough segments
        if selected.len() < self.min_segments_to_merge {
            Vec::new()
        } else {
            selected
        }
    }
}

/// Log-structured merge policy (for write-heavy workloads)
pub struct LogStructuredMergePolicy {
    pub size_ratio: f32,  // Size ratio between levels
    pub min_merge_size_mb: usize,
}

impl Default for LogStructuredMergePolicy {
    fn default() -> Self {
        LogStructuredMergePolicy {
            size_ratio: 10.0,  // Each level is 10x larger
            min_merge_size_mb: 1,
        }
    }
}

impl MergePolicy for LogStructuredMergePolicy {
    fn should_merge(&self, segments: &[Arc<Segment>]) -> bool {
        // Group segments by size tier
        let mut tiers: Vec<Vec<Arc<Segment>>> = Vec::new();
        let min_size = self.min_merge_size_mb * 1024 * 1024;
        
        for segment in segments {
            // Find appropriate tier for this segment
            let tier_index = ((segment.metadata.size_bytes as f32 / min_size as f32).log10() 
                / self.size_ratio.log10()) as usize;
            
            // Ensure we have enough tiers
            while tiers.len() <= tier_index {
                tiers.push(Vec::new());
            }
            
            tiers[tier_index].push(segment.clone());
        }
        
        // Check if any tier has too many segments
        tiers.iter().any(|tier| tier.len() >= 4)
    }
    
    fn select_segments_to_merge(&self, segments: &[Arc<Segment>]) -> Vec<Arc<Segment>> {
        // Find segments of similar size to merge
        let min_size = self.min_merge_size_mb * 1024 * 1024;
        let mut tiers: Vec<Vec<Arc<Segment>>> = Vec::new();
        
        for segment in segments {
            let tier_index = ((segment.metadata.size_bytes as f32 / min_size as f32).log10() 
                / self.size_ratio.log10()) as usize;
            
            while tiers.len() <= tier_index {
                tiers.push(Vec::new());
            }
            
            tiers[tier_index].push(segment.clone());
        }
        
        // Find first tier with enough segments to merge
        for tier in tiers {
            if tier.len() >= 4 {
                return tier;
            }
        }
        
        Vec::new()
    }
}
