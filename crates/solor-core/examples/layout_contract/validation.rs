use std::collections::{HashMap, HashSet, VecDeque};

use super::{
    AccessError, Attempt, Context, Entry, LayoutError, NodeId, Query, Read, ReadSet, Result, Scene,
    Scratch, Size, layout, measure,
};

pub const MAX_TRACKED_EDITS: usize = 256;
const END: usize = usize::MAX;

#[derive(Default)]
pub(super) struct ChangeLog {
    revision: u64,
    edits: VecDeque<NodeId>,
}

impl ChangeLog {
    pub(super) fn record(&mut self, node: NodeId) -> Result<()> {
        let revision = self
            .revision
            .checked_add(1)
            .ok_or(LayoutError::RevisionExhausted)?;
        if self.edits.len() == MAX_TRACKED_EDITS {
            self.edits.pop_front();
        }
        self.edits.push_back(node);
        self.revision = revision;
        Ok(())
    }
}

/// A bounded-history cursor and an index into one checkpoint's original read order.
pub struct SelectiveCheckpoint {
    evidence: Checkpoint,
    revision: u64,
    heads: HashMap<NodeId, usize>,
    next: Vec<usize>,
}

impl Read {
    fn node(self) -> NodeId {
        match self {
            Self::Measure { node, .. } | Self::Layout { node, .. } => node,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckTarget {
    Measure(Query),
    Layout(Size),
}

/// Owned evidence captured from one valid result after topology is locked.
pub struct Checkpoint {
    owner: NodeId,
    input_revision: u64,
    target: CheckTarget,
    reads: ReadSet,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckOutcome {
    Unchanged,
    Changed,
    Unverifiable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CheckReport {
    pub outcome: CheckOutcome,
    pub checked_reads: usize,
    pub staged_geometry: usize,
}

impl Scene {
    pub fn capture_selective_dependencies(
        &mut self,
        owner: NodeId,
        target: CheckTarget,
    ) -> Result<Option<SelectiveCheckpoint>> {
        let Some(evidence) = self.capture_dependencies(owner, target)? else {
            return Ok(None);
        };
        let revision = self.changes.get_or_insert_with(ChangeLog::default).revision;
        let mut heads = HashMap::new();
        let mut next = vec![END; evidence.reads.reads.len()];
        for (index, read) in evidence.reads.reads.iter().enumerate().rev() {
            next[index] = heads.insert(read.node(), index).unwrap_or(END);
        }
        Ok(Some(SelectiveCheckpoint {
            evidence,
            revision,
            heads,
            next,
        }))
    }

    /// Select only reads reached by recorded edits; never marks layout work complete.
    pub fn check_selective_dependencies(
        &mut self,
        checkpoint: &mut SelectiveCheckpoint,
        target: CheckTarget,
    ) -> Result<CheckReport> {
        if !self.can_check(&checkpoint.evidence, target)? {
            return Ok(CheckReport::empty(CheckOutcome::Unverifiable));
        }
        let changes = self
            .changes
            .as_ref()
            .expect("capture enables change tracking");
        let elapsed = changes.revision - checkpoint.revision;
        if elapsed > changes.edits.len() as u64 {
            return Ok(CheckReport::empty(CheckOutcome::Unverifiable));
        }
        let revision = changes.revision;
        let owner = checkpoint.evidence.owner;
        let mut children = HashSet::new();
        for &node in changes
            .edits
            .iter()
            .skip(changes.edits.len() - elapsed as usize)
        {
            let mut current = node;
            while let Some(parent) = self.store.parent(current).map_err(AccessError::from)? {
                if parent == owner {
                    children.insert(current);
                    break;
                }
                current = parent;
            }
        }
        let mut selected = Vec::new();
        for child in children {
            let mut index = checkpoint.heads.get(&child).copied().unwrap_or(END);
            while index != END {
                selected.push(index);
                index = checkpoint.next[index];
            }
        }
        // Hash iteration order must not change query eviction or failure order.
        selected.sort_unstable();
        let report = if selected.is_empty() {
            CheckReport::empty(CheckOutcome::Unchanged)
        } else {
            self.invalidate(owner)?;
            let attempt = Attempt(&mut self.scratch);
            self.store.execute(owner, |_entry, access| {
                Ok(check_reads(
                    access,
                    selected
                        .iter()
                        .map(|&index| &checkpoint.evidence.reads.reads[index]),
                    attempt.0,
                ))
            })??
        };
        // Only successful equality extends the evidence to the current edit cursor.
        // Changed results, errors and unwinds must retain the earlier cursor.
        if report.outcome == CheckOutcome::Unchanged {
            checkpoint.revision = revision;
        }
        Ok(report)
    }

    fn can_check(&self, checkpoint: &Checkpoint, target: CheckTarget) -> Result<bool> {
        let entry = self.store.get(checkpoint.owner)?;
        Ok(checkpoint.target == target
            && checkpoint.input_revision == entry.input_revision
            && checkpoint.reads.complete)
    }

    pub fn capture_dependencies(
        &self,
        owner: NodeId,
        target: CheckTarget,
    ) -> Result<Option<Checkpoint>> {
        let entry = self.store.get(owner)?;
        if self.request.is_none() {
            return Ok(None);
        }
        let reads = match target {
            CheckTarget::Measure(query) => {
                if entry.measure_dirty {
                    return Ok(None);
                }
                let Some(slot) = entry
                    .measure_cache
                    .iter()
                    .position(|slot| slot.as_ref().is_some_and(|cached| cached.query == query))
                else {
                    return Ok(None);
                };
                entry
                    .records
                    .as_ref()
                    .map(|records| &records.measurements[slot])
            }
            CheckTarget::Layout(assigned) => {
                if entry.layout_dirty
                    || !entry
                        .geometry
                        .is_some_and(|geometry| geometry.metrics.size == assigned)
                {
                    return Ok(None);
                }
                entry.records.as_ref().map(|records| &records.layout)
            }
        };
        Ok(Some(Checkpoint {
            owner,
            input_revision: entry.input_revision,
            target,
            reads: reads.cloned().unwrap_or_default(),
        }))
    }

    /// Audit the recorded reads, not permission to reuse or publish a parent result.
    /// Speculative child layout is discarded; normal layout must still reconcile work.
    pub fn check_dependencies(
        &mut self,
        checkpoint: &Checkpoint,
        target: CheckTarget,
    ) -> Result<CheckReport> {
        if !self.can_check(checkpoint, target)? {
            return Ok(CheckReport::empty(CheckOutcome::Unverifiable));
        }
        if checkpoint.reads.reads.is_empty() {
            return Ok(CheckReport {
                outcome: CheckOutcome::Unchanged,
                checked_reads: 0,
                staged_geometry: 0,
            });
        }
        // Speculation can mutate private caches and dirty flags. Require a normal
        // conservative pass afterward, including when a child fails or unwinds.
        self.invalidate(checkpoint.owner)?;
        let attempt = Attempt(&mut self.scratch);
        self.store.execute(checkpoint.owner, |_entry, access| {
            Ok(check_reads(access, &checkpoint.reads.reads, attempt.0))
        })?
    }
}

impl CheckReport {
    fn empty(outcome: CheckOutcome) -> Self {
        Self {
            outcome,
            checked_reads: 0,
            staged_geometry: 0,
        }
    }
}

fn check_reads<'a>(
    access: &mut Context<'_, Entry>,
    reads: impl IntoIterator<Item = &'a Read>,
    scratch: &mut Scratch,
) -> Result<CheckReport> {
    let mut checked_reads = 0;
    for read in reads {
        let equal = match *read {
            Read::Measure {
                node,
                query,
                metrics,
            } => {
                let current = access.execute_child(node, |entry, child| {
                    Ok(measure(entry, child, query, false))
                })??;
                current == metrics
            }
            Read::Layout {
                node,
                assigned,
                output,
            } => {
                let (_, current) = access.execute_child(node, |entry, child| {
                    Ok(layout(entry, child, assigned, scratch, false))
                })??;
                current == output
            }
        };
        checked_reads += 1;
        if !equal {
            return Ok(CheckReport {
                outcome: CheckOutcome::Changed,
                checked_reads,
                staged_geometry: scratch.draft.len(),
            });
        }
    }
    Ok(CheckReport {
        outcome: CheckOutcome::Unchanged,
        checked_reads,
        staged_geometry: scratch.draft.len(),
    })
}
