/// Optimized operations for search (SIMD-like optimizations without external dependencies)
pub struct SimdOps;

impl SimdOps {
    /// Fast union of sorted arrays
    /// Merges two sorted arrays into one sorted array with no duplicates
    pub fn union_sorted(a: &[u32], b: &[u32]) -> Vec<u32> {
        let mut result = Vec::with_capacity(a.len() + b.len());
        let mut i = 0;
        let mut j = 0;
        
        while i < a.len() && j < b.len() {
            if a[i] < b[j] {
                result.push(a[i]);
                i += 1;
            } else if a[i] > b[j] {
                result.push(b[j]);
                j += 1;
            } else {
                // Equal elements - add once
                result.push(a[i]);
                i += 1;
                j += 1;
            }
        }
        
        // Add remaining elements
        while i < a.len() {
            result.push(a[i]);
            i += 1;
        }
        
        while j < b.len() {
            result.push(b[j]);
            j += 1;
        }
        
        result
    }
    
    /// Fast intersection of sorted arrays using galloping search
    /// This is a highly optimized algorithm used in search engines
    pub fn intersect_sorted(a: &[u32], b: &[u32]) -> Vec<u32> {
        if a.is_empty() || b.is_empty() {
            return Vec::new();
        }

        let mut result = Vec::new();
        let mut i = 0;
        let mut j = 0;

        // Use galloping search for better performance on skewed distributions
        const GALLOP_THRESHOLD: usize = 8;

        while i < a.len() && j < b.len() {
            // Check for batch skip opportunities
            if i + GALLOP_THRESHOLD <= a.len() && j + GALLOP_THRESHOLD <= b.len() {
                // Look ahead to see if we can skip chunks
                let max_a = a[i + GALLOP_THRESHOLD - 1];
                let min_b = b[j];

                if max_a < min_b {
                    // All of a's chunk is before b's current position
                    i += GALLOP_THRESHOLD;
                    continue;
                }

                let max_b = b[j + GALLOP_THRESHOLD - 1];
                let min_a = a[i];

                if max_b < min_a {
                    // All of b's chunk is before a's current position
                    j += GALLOP_THRESHOLD;
                    continue;
                }
            }

            // Standard merge intersection
            if a[i] < b[j] {
                i += 1;
            } else if a[i] > b[j] {
                j += 1;
            } else {
                result.push(a[i]);
                i += 1;
                j += 1;
            }
        }

        result
    }

    /// Bulk scoring with manual unrolling for better performance
    pub fn score_documents(scores: &mut [f32], boost: f32) {
        let len = scores.len();
        let mut i = 0;

        // Process 8 elements at a time (manual unrolling)
        while i + 8 <= len {
            scores[i] *= boost;
            scores[i + 1] *= boost;
            scores[i + 2] *= boost;
            scores[i + 3] *= boost;
            scores[i + 4] *= boost;
            scores[i + 5] *= boost;
            scores[i + 6] *= boost;
            scores[i + 7] *= boost;
            i += 8;
        }

        // Handle remaining elements
        while i < len {
            scores[i] *= boost;
            i += 1;
        }
    }

    /// Vectorized dot product for scoring
    pub fn dot_product(a: &[f32], b: &[f32]) -> f32 {
        assert_eq!(a.len(), b.len(), "Arrays must have same length");

        let len = a.len();
        let mut sum = 0.0;
        let mut i = 0;

        // Process 4 elements at a time (helps compiler auto-vectorize)
        while i + 4 <= len {
            sum += a[i] * b[i];
            sum += a[i + 1] * b[i + 1];
            sum += a[i + 2] * b[i + 2];
            sum += a[i + 3] * b[i + 3];
            i += 4;
        }

        // Handle remaining elements
        while i < len {
            sum += a[i] * b[i];
            i += 1;
        }

        sum
    }
}