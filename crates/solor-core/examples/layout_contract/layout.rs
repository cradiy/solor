// Compile the library solver at the cache size selected by this experiment.
pub use solor_core::layout::Margin;
include!("../../src/layout/engine.rs");

#[cfg(test)]
#[path = "staged_allocation_tests.rs"]
mod staged_allocation_tests;

#[cfg(test)]
impl Scene {
    pub fn scratch_usage(&self) -> [(usize, usize); 2] {
        [
            (self.scratch.draft.len(), self.scratch.draft.capacity()),
            (
                self.scratch.children.len(),
                self.scratch.children.capacity(),
            ),
        ]
    }

    pub fn staged_usage(&self) -> [(usize, usize); 2] {
        self.scratch.reuse.as_ref().map_or([(0, 0); 2], |reuse| {
            [
                (reuse.candidates.len(), reuse.candidates.capacity()),
                (reuse.live.len(), reuse.live.capacity()),
            ]
        })
    }

    pub fn measurement_reads(&self, id: NodeId, query: Query) -> Option<ReadSet> {
        let entry = self.store.get(id).unwrap();
        if entry.measure_dirty {
            return None;
        }
        entry
            .measure_cache
            .iter()
            .position(|slot| {
                slot.as_ref().is_some_and(|cached| {
                    cached.query == query && cached.containing_block == ContainingBlock::default()
                })
            })
            .map(|slot| {
                entry
                    .records
                    .as_ref()
                    .map_or_else(ReadSet::default, |records| {
                        records.measurements[slot].clone()
                    })
            })
    }

    pub fn published_layout_reads(&self, id: NodeId) -> Option<ReadSet> {
        let entry = self.store.get(id).unwrap();
        entry.geometry.map(|_| {
            entry
                .records
                .as_ref()
                .map_or_else(ReadSet::default, |records| records.layout.clone())
        })
    }

    pub fn has_record_block(&self, id: NodeId) -> bool {
        self.store.get(id).unwrap().records.is_some()
    }

    pub fn cached_queries(&self, node: NodeId) -> usize {
        let entry = self.store.get(node).unwrap();
        if entry.measure_dirty {
            0
        } else {
            entry.measure_cache.iter().flatten().count()
        }
    }
}
