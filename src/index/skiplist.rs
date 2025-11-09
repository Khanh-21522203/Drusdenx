use crate::core::types::DocId;
use crate::index::posting::PostingList;
use crate::core::error::Result;

/// Skip list for fast intersection (with cached decoded doc IDs)
/// Trade-off: Uses extra memory but enables fast queries
pub struct SkipList {
    pub entries: Vec<SkipEntry>,
    pub doc_ids: Vec<u32>,
    pub skip_interval: usize,
}

pub struct SkipEntry {
    pub doc_id: DocId,
    pub position: usize,          // Position in doc_ids array
    pub skip_to: Option<usize>,   // Next skip position
}

impl SkipList {
    /// Build skip list from encoded PostingList
    /// Decodes doc IDs once and caches them
    pub fn build(posting_list: &PostingList) -> Result<Self> {
        // Decode once during build (one-time cost)
        let doc_ids = posting_list.decode_doc_ids()?;
        let len = doc_ids.len();
        let interval = (len as f32).sqrt().max(4.0) as usize;  // At least 4

        let mut entries = Vec::new();

        // Build skip entries at regular intervals
        for i in (0..len).step_by(interval) {
            let skip_to = if i + interval < len {
                Some(i + interval)
            } else {
                None
            };

            entries.push(SkipEntry {
                doc_id: DocId(doc_ids[i] as u64),
                position: i,
                skip_to,
            });
        }

        Ok(SkipList {
            entries,
            doc_ids,
            skip_interval: interval,
        })
    }

    /// Find doc ID using skip list (O(√n) instead of O(n))
    pub fn find(&self, target: DocId) -> Option<usize> {
        let target_u32 = target.0 as u32;

        // Use skip entries to jump
        let mut pos = 0;
        for entry in &self.entries {
            if entry.doc_id.0 as u32 > target_u32 {
                break;
            }
            pos = entry.position;
        }

        // Linear scan from skip position
        for i in pos..self.doc_ids.len() {
            let doc_id = self.doc_ids[i];
            if doc_id == target_u32 {
                return Some(i);
            }
            if doc_id > target_u32 {
                break;
            }
        }

        None
    }

    /// Intersect two skip lists (fast boolean AND)
    pub fn intersect(list1: &SkipList, list2: &SkipList) -> Vec<DocId> {
        let mut result = Vec::new();
        let mut i = 0;
        let mut j = 0;

        while i < list1.doc_ids.len() && j < list2.doc_ids.len() {
            let doc1 = list1.doc_ids[i];
            let doc2 = list2.doc_ids[j];

            if doc1 == doc2 {
                result.push(DocId(doc1 as u64));
                i += 1;
                j += 1;
            } else if doc1 < doc2 {
                // Use skip list to jump ahead
                i = list1.skip_to_ge(doc2, i);
            } else {
                j = list2.skip_to_ge(doc1, j);
            }
        }

        result
    }

    /// Skip to position >= target using skip entries
    fn skip_to_ge(&self, target: u32, from: usize) -> usize {
        // Find appropriate skip entry
        for entry in &self.entries {
            if entry.position < from {
                continue;
            }
            if entry.doc_id.0 as u32 >= target {
                return entry.position;
            }
        }

        // Linear scan if no skip entry found
        for i in from..self.doc_ids.len() {
            if self.doc_ids[i] >= target {
                return i;
            }
        }

        self.doc_ids.len()  // Not found
    }

    /// Intersect multiple skip lists
    pub fn intersect_multiple(lists: &[&SkipList]) -> Result<Vec<DocId>> {
        if lists.is_empty() {
            return Ok(Vec::new());
        }
        if lists.len() == 1 {
            return Ok(lists[0].doc_ids.iter()
                .map(|&id| DocId(id as u64))
                .collect());
        }

        // Start with first two
        let mut result = Self::intersect(lists[0], lists[1]);

        // Intersect with remaining lists
        for &list in &lists[2..] {
            result = result.into_iter()
                .filter(|&doc_id| {
                    list.find(doc_id).is_some()
                })
                .collect();

            if result.is_empty() {
                break;
            }
        }

        Ok(result)
    }
}