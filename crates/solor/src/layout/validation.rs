use std::collections::{HashMap, HashSet, VecDeque};

use super::{
    AccessError, Attempt, ContainingBlock, Context, Draft, Entry, LayoutError, NodeId, Query, Read,
    ReadSet, Result, RootRequest, Scene, Scratch, Size, layout, layout_root, measure,
    publish_layout,
};

/// Maximum number of recent edits retained for selective validation.
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

fn index_reads(reads: &[Read]) -> (HashMap<NodeId, usize>, Vec<usize>) {
    let mut heads = HashMap::new();
    let mut next = vec![END; reads.len()];
    for (index, read) in reads.iter().enumerate().rev() {
        next[index] = heads.insert(read.node(), index).unwrap_or(END);
    }
    (heads, next)
}

struct EvidenceRef<'a> {
    input_revision: u64,
    reads: &'a [Read],
    complete: bool,
}

impl Read {
    fn node(self) -> NodeId {
        match self {
            Self::Measure { node, .. } | Self::Layout { node, .. } => node,
        }
    }
}

/// Operation whose recorded child reads should be checked.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CheckTarget {
    /// Preferred-size measurement for the supplied constraint.
    Measure(Query),
    /// Child placement at the supplied size.
    Layout(Size),
}

/// Owned evidence captured for one result and one topology revision.
pub struct Checkpoint {
    owner: NodeId,
    input_revision: u64,
    target: CheckTarget,
    reads: ReadSet,
    publication: Option<u64>,
    topology_revision: u64,
}

/// Result of comparing recorded and current dependency outputs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckOutcome {
    /// All verified reads still have the recorded values.
    Unchanged,
    /// At least one child returned a different value.
    Changed,
    /// The retained evidence cannot establish equivalence.
    Unverifiable,
}

/// Outcome and work performed by one dependency check.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CheckReport {
    /// Whether the recorded output can still be reused.
    pub outcome: CheckOutcome,
    /// Number of recorded reads replayed.
    pub checked_reads: usize,
    /// Number of geometry entries staged during verification.
    pub staged_geometry: usize,
}

/// Publication and dependency-check results for one audited run.
#[derive(Debug)]
pub struct ValidatedRun {
    /// Number of nodes whose published geometry changed.
    pub published: usize,
    /// Reports for measurement and placement in that order.
    pub checks: [CheckReport; 2],
    /// Number of staged child layouts reused.
    pub reused_layouts: usize,
    /// Whether verified child results permitted skipping parent placement.
    pub skipped_parent_layout: bool,
}

impl Scene {
    /// Captures a checkpoint indexed by the child nodes it read.
    pub fn capture_selective_dependencies(
        &mut self,
        owner: NodeId,
        target: CheckTarget,
    ) -> Result<Option<SelectiveCheckpoint>> {
        let Some(evidence) = self.capture_dependencies(owner, target)? else {
            return Ok(None);
        };
        let revision = self.changes.get_or_insert_with(ChangeLog::default).revision;
        let (heads, next) = index_reads(&evidence.reads.reads);
        Ok(Some(SelectiveCheckpoint {
            evidence,
            revision,
            heads,
            next,
        }))
    }

    /// Refresh from a valid result; unavailable evidence leaves the checkpoint intact.
    pub fn refresh_selective_dependencies(
        &self,
        checkpoint: &mut SelectiveCheckpoint,
        target: CheckTarget,
    ) -> Result<bool> {
        let Some(current) = self.dependency_record(checkpoint.evidence.owner, target)? else {
            return Ok(false);
        };
        let same_order = checkpoint.evidence.reads.reads.len() == current.reads.len()
            && checkpoint
                .evidence
                .reads
                .reads
                .iter()
                .zip(current.reads)
                .all(|(old, new)| old.node() == new.node());
        if same_order {
            checkpoint
                .evidence
                .reads
                .reads
                .copy_from_slice(current.reads);
        } else {
            // Changed structure owns fresh storage, releasing the previous high-water mark.
            let reads = current.reads.to_vec();
            let (heads, next) = index_reads(&reads);
            checkpoint.evidence.reads.reads = reads;
            checkpoint.heads = heads;
            checkpoint.next = next;
        }
        checkpoint.evidence.reads.complete = current.complete;
        checkpoint.evidence.topology_revision = self.topology_revision;
        checkpoint.evidence.input_revision = current.input_revision;
        checkpoint.evidence.target = target;
        checkpoint.evidence.publication = Some(self.publication);
        checkpoint.revision = self
            .changes
            .as_ref()
            .expect("capture enables change tracking")
            .revision;
        Ok(true)
    }

    /// Select only reads reached by recorded edits; never marks layout work complete.
    pub fn check_selective_dependencies(
        &mut self,
        checkpoint: &mut SelectiveCheckpoint,
        target: CheckTarget,
    ) -> Result<CheckReport> {
        let Some(selected) = self.select_reads(checkpoint, target)? else {
            return Ok(CheckReport::empty(CheckOutcome::Unverifiable));
        };
        let revision = self
            .changes
            .as_ref()
            .expect("capture enables change tracking")
            .revision;
        let owner = checkpoint.evidence.owner;
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
        if report.outcome == CheckOutcome::Unchanged {
            checkpoint.revision = revision;
            if !selected.is_empty() {
                // The audit consumed edits but discarded their unpublished geometry.
                checkpoint.evidence.publication = None;
            }
        }
        Ok(report)
    }

    /// Root audits and normal layout share one unpublished attempt with no input edits.
    pub fn run_validated(
        &mut self,
        query: Query,
        assigned: Size,
        measurement: &mut SelectiveCheckpoint,
        placement: &mut SelectiveCheckpoint,
    ) -> Result<ValidatedRun> {
        self.run_with_checks(query, assigned, measurement, placement, false)
    }

    /// Skip only the root layout when its current published result is validated.
    pub fn run_pruned(
        &mut self,
        query: Query,
        assigned: Size,
        measurement: &mut SelectiveCheckpoint,
        placement: &mut SelectiveCheckpoint,
    ) -> Result<ValidatedRun> {
        self.run_with_checks(query, assigned, measurement, placement, true)
    }

    fn run_with_checks(
        &mut self,
        query: Query,
        assigned: Size,
        measurement: &mut SelectiveCheckpoint,
        placement: &mut SelectiveCheckpoint,
        prune_parent: bool,
    ) -> Result<ValidatedRun> {
        let root = self.root();
        if measurement.evidence.owner != root || placement.evidence.owner != root {
            return Err(LayoutError::InvalidCheckpointOwner);
        }
        let selected = [
            self.select_reads(measurement, CheckTarget::Measure(query))?,
            self.select_reads(placement, CheckTarget::Layout(assigned))?,
        ];
        let revision = self
            .changes
            .as_ref()
            .expect("capture enables change tracking")
            .revision;
        let mut checks = selected.each_ref().map(|indices| {
            CheckReport::empty(if indices.is_some() {
                CheckOutcome::Unchanged
            } else {
                CheckOutcome::Unverifiable
            })
        });
        let (published, reused_layouts, skipped_parent_layout) =
            if selected.iter().flatten().all(Vec::is_empty) {
                (self.run(query, Some(assigned), false)?, 0, false)
            } else {
                let request = RootRequest::Measure(query, Some(assigned));
                let publication = self
                    .publication
                    .checked_add(1)
                    .ok_or(LayoutError::RevisionExhausted)?;
                self.invalidate(root)?;
                let attempt = Attempt(&mut self.scratch);
                let reuse = attempt.0.reuse.get_or_insert_with(Default::default);
                reuse.active = true;
                reuse.auditing = true;
                for (index, checkpoint) in [&*measurement, &*placement].iter().enumerate() {
                    if let Some(indices) = &selected[index] {
                        checks[index] = self.store.execute(root, |_entry, access| {
                            Ok(check_reads(
                                access,
                                indices.iter().map(|&i| &checkpoint.evidence.reads.reads[i]),
                                attempt.0,
                            ))
                        })??;
                    }
                }
                attempt.0.reuse.as_mut().unwrap().auditing = false;
                let old_geometry = self.store.get(root)?.geometry;
                let skipped = prune_parent
                    && checks[1].outcome == CheckOutcome::Unchanged
                    && placement.evidence.publication == Some(self.publication)
                    && old_geometry.is_some_and(|geometry| geometry.metrics.size == assigned);
                if skipped {
                    // Measurement remains conservative; it can fail before any publication.
                    self.store.execute(root, |entry, access| {
                        Ok(measure(
                            entry,
                            access,
                            query,
                            ContainingBlock::default(),
                            false,
                        ))
                    })??;
                    let reuse = attempt.0.reuse.as_mut().unwrap();
                    for &index in selected[1]
                        .as_ref()
                        .expect("unchanged check has complete evidence")
                    {
                        if let Read::Layout { node, .. } = placement.evidence.reads.reads[index]
                            && let Some(range) = reuse.candidates.get(&node)
                        {
                            // Only a direct child's root inherits its previous placement.
                            // Its descendants retain the newly computed internal geometry.
                            let origin = self
                                .store
                                .get(node)?
                                .geometry
                                .expect("published child")
                                .origin;
                            attempt.0.draft[range.end - 1].geometry.origin = origin;
                            reuse.live[range.clone()].fill(true);
                            reuse.hits += 1;
                        }
                    }
                    let position = attempt.0.draft.len();
                    attempt.0.draft.push(Draft {
                        node: root,
                        geometry: old_geometry.unwrap(),
                        reads: None,
                    });
                    attempt.0.record_draft(position, None);
                } else {
                    layout_root(&mut self.store, attempt.0, request, false)?;
                }
                let published = publish_layout(&mut self.store, attempt.0);
                let hits = attempt.0.reuse.as_ref().unwrap().hits;
                drop(attempt);
                self.finish_layout(request, published);
                self.publication = publication;
                (published, hits, skipped)
            };
        // No checkpoint cursor commits unless the entire update succeeds.
        for (checkpoint, report) in [measurement, placement].into_iter().zip(checks) {
            if report.outcome == CheckOutcome::Unchanged {
                checkpoint.revision = revision;
                checkpoint.evidence.publication = Some(self.publication);
            }
        }
        Ok(ValidatedRun {
            published,
            checks,
            reused_layouts,
            skipped_parent_layout,
        })
    }

    fn select_reads(
        &self,
        checkpoint: &SelectiveCheckpoint,
        target: CheckTarget,
    ) -> Result<Option<Vec<usize>>> {
        if !self.can_check(&checkpoint.evidence, target)? {
            return Ok(None);
        }
        let changes = self
            .changes
            .as_ref()
            .expect("capture enables change tracking");
        let elapsed = changes.revision - checkpoint.revision;
        if elapsed > changes.edits.len() as u64 {
            return Ok(None);
        }
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
        Ok(Some(selected))
    }

    fn can_check(&self, checkpoint: &Checkpoint, target: CheckTarget) -> Result<bool> {
        if checkpoint.topology_revision != self.topology_revision {
            return Ok(false);
        }
        let entry = self.store.get(checkpoint.owner)?;
        Ok(checkpoint.target == target
            && checkpoint.input_revision == entry.input_revision
            && checkpoint.reads.complete)
    }

    /// Captures retained child-read evidence for the given operation.
    pub fn capture_dependencies(
        &self,
        owner: NodeId,
        target: CheckTarget,
    ) -> Result<Option<Checkpoint>> {
        Ok(self
            .dependency_record(owner, target)?
            .map(|current| Checkpoint {
                owner,
                input_revision: current.input_revision,
                target,
                reads: ReadSet {
                    reads: current.reads.to_vec(),
                    complete: current.complete,
                },
                publication: Some(self.publication),
                topology_revision: self.topology_revision,
            }))
    }

    fn dependency_record(
        &self,
        owner: NodeId,
        target: CheckTarget,
    ) -> Result<Option<EvidenceRef<'_>>> {
        let entry = self.store.get(owner)?;
        if self.request.is_none() {
            return Ok(None);
        }
        let reads = match target {
            CheckTarget::Measure(query) => {
                if entry.measure_dirty {
                    return Ok(None);
                }
                let Some(slot) = entry.measure_cache.iter().position(|slot| {
                    slot.as_ref().is_some_and(|cached| {
                        cached.query == query
                            && cached.containing_block == ContainingBlock::default()
                    })
                }) else {
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
        Ok(Some(EvidenceRef {
            input_revision: entry.input_revision,
            reads: reads.map_or(&[], |reads| reads.reads.as_slice()),
            complete: reads.is_none_or(|reads| reads.complete),
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
                containing_block,
                margins,
                column_align,
            } => {
                let current = access.execute_child(node, |entry, child| {
                    Ok(super::measure_item(
                        entry,
                        child,
                        query,
                        containing_block,
                        false,
                        match (column_align, margins) {
                            (Some(align), _) => super::ItemQuery::Column(align),
                            (_, Some(_)) => super::ItemQuery::Stack,
                            _ => super::ItemQuery::Direct,
                        },
                    ))
                })??;
                current.0 == metrics && margins.is_none_or(|edges| edges == current.1)
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
