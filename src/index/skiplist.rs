use crate::core::types::DocId;
use crate::index::posting::PostingList;

/// Skip list for fast intersection
pub struct SkipList {
    pub entries: Vec<SkipEntry>,
    pub skip_interval: usize,
}

pub struct SkipEntry {
    pub doc_id: DocId,
    pub skip_to: Option<usize>,
}

impl SkipList {
    pub fn build(posting_list: &PostingList) -> Self {
        let interval = (posting_list.len() as f32).sqrt() as usize;
        let mut entries = Vec::new();

        for (i, posting) in posting_list.postings.iter().enumerate() {
            let skip_to = if (i + 1) % interval == 0 && i + interval < posting_list.len() {
                Some(i + interval)
            } else {
                None
            };

            entries.push(SkipEntry {
                doc_id: posting.doc_id,
                skip_to,
            });
        }

        SkipList {
            entries,
            skip_interval: interval,
        }
    }

    pub fn find(&self, target: DocId) -> Option<usize> {
        let mut i = 0;

        while i < self.entries.len() {
            if self.entries[i].doc_id >= target {
                return Some(i);
            }

            if let Some(skip_to) = self.entries[i].skip_to {
                if skip_to < self.entries.len() &&
                    self.entries[skip_to].doc_id <= target {
                    i = skip_to;
                    continue;
                }
            }

            i += 1;
        }

        None
    }
}