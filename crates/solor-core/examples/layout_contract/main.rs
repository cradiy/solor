//! Headless layout contract experiment, not a public UI API.

const MEASUREMENT_CACHE_SLOTS: usize = 3;

mod column;
use solor_core::{node_store, tree};
mod fixtures;
#[allow(dead_code, reason = "the fixture exercises a subset of the layout API")]
mod layout;
mod negotiation;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod negotiation_tests;

#[cfg(test)]
mod dependency_tests;

use fixtures::{Column, Drawing, Wrapping, nested};
use layout::validation::{CheckOutcome, CheckReport, CheckTarget, Checkpoint};
use layout::{Query, Scene, Size};

fn main() {
    let (mut incremental, ids) = nested();
    let (mut reference, reference_ids) = nested();
    incremental.run(Query::Exact(100), None, false).unwrap();
    reference.run(Query::Exact(100), None, true).unwrap();
    assert_eq!(incremental.snapshot(), reference.snapshot());
    let before = incremental.counts();
    incremental
        .edit_layout::<Wrapping>(ids.text, |text| text.units = 100)
        .unwrap();
    reference
        .edit_layout::<Wrapping>(reference_ids.text, |text| text.units = 100)
        .unwrap();
    let published = incremental.run(Query::Exact(100), None, false).unwrap();
    reference.run(Query::Exact(100), None, true).unwrap();
    assert_eq!(incremental.snapshot(), reference.snapshot());
    let after = incremental.counts();
    println!(
        "text edit: {} measurement bodies, {} layout bodies, {published} published entries",
        after.measures - before.measures,
        after.layouts - before.layouts,
    );

    let before = incremental.counts();
    incremental.set_ink(ids.drawing, 17).unwrap();
    reference.set_ink(reference_ids.drawing, 17).unwrap();
    assert_eq!(incremental.run(Query::Exact(100), None, false), Ok(0));
    reference.run(Query::Exact(100), None, true).unwrap();
    assert_eq!(incremental.snapshot(), reference.snapshot());
    assert_eq!(incremental.counts(), before);
    println!("ink edit: no measurement or layout; geometry matches full recomputation");
    incremental.release_scratch();
    assert_eq!(incremental.run(Query::Exact(100), None, false), Ok(0));
    assert_eq!(incremental.snapshot(), reference.snapshot());
    println!("scratch released: clean layout and published geometry remain valid");
    let (reads, bytes, incomplete) = incremental.dependency_stats();
    println!(
        "dependency evidence: {reads} reads, {bytes} reserved bytes, {incomplete} incomplete records"
    );
    negotiation::run();
    validate_dependency_results();
}

fn validate_dependency_results() {
    let mut scene = Scene::new(Column { padding: 0, gap: 0 });
    let root = scene.root();
    let child = scene.append(root, Drawing::new(20, 5)).unwrap();
    scene.run(Query::Exact(10), None, false).unwrap();
    let measured = CheckTarget::Measure(Query::Exact(10));
    let allocated = CheckTarget::Layout(Size {
        width: 10,
        height: 5,
    });
    let checkpoint: Checkpoint = scene.capture_dependencies(root, measured).unwrap().unwrap();
    let layout = scene
        .capture_dependencies(root, allocated)
        .unwrap()
        .unwrap();
    let mut selective = scene
        .capture_selective_dependencies(root, allocated)
        .unwrap()
        .unwrap();
    let mut selected_measure = scene
        .capture_selective_dependencies(root, measured)
        .unwrap()
        .unwrap();
    let published = scene.snapshot();
    scene
        .edit_layout::<Drawing>(child, |leaf| leaf.size.width = 21)
        .unwrap();
    let measurement: CheckReport = scene.check_dependencies(&checkpoint, measured).unwrap();
    let placement = scene.check_dependencies(&layout, allocated).unwrap();
    assert_eq!(
        scene
            .check_selective_dependencies(&mut selective, allocated)
            .unwrap()
            .outcome,
        placement.outcome
    );
    assert_eq!(measurement.outcome, CheckOutcome::Unchanged);
    assert_eq!(placement.outcome, CheckOutcome::Changed);
    assert_eq!(scene.snapshot(), published);
    println!(
        "dependency audit: measurement={measurement:?}, layout={placement:?}; geometry unpublished"
    );
    let result = scene
        .run_validated(
            Query::Exact(10),
            Size {
                width: 10,
                height: 5,
            },
            &mut selected_measure,
            &mut selective,
        )
        .unwrap();
    assert_eq!(result.published, 2);
    assert!(!result.skipped_parent_layout);
    assert_eq!(result.reused_layouts, 1);
    assert_eq!(result.checks[1].outcome, CheckOutcome::Changed);
    println!(
        "shared attempt: {} adopted subtree, {} published entries",
        result.reused_layouts, result.published
    );
    let incremental = scene.snapshot();
    assert!(
        scene
            .refresh_selective_dependencies(&mut selective, allocated)
            .unwrap()
    );
    assert_eq!(
        scene
            .check_selective_dependencies(&mut selective, allocated)
            .unwrap()
            .outcome,
        CheckOutcome::Unchanged
    );
    scene.run(Query::Exact(10), None, true).unwrap();
    assert_eq!(scene.snapshot(), incremental);
    // Refresh after the forced reference publication, then validate a stable edit.
    assert!(
        scene
            .refresh_selective_dependencies(&mut selective, allocated)
            .unwrap()
    );
    assert!(
        scene
            .refresh_selective_dependencies(&mut selected_measure, measured)
            .unwrap()
    );
    scene.edit_layout::<Drawing>(child, |_| {}).unwrap();
    let result = scene
        .run_pruned(
            Query::Exact(10),
            Size {
                width: 10,
                height: 5,
            },
            &mut selected_measure,
            &mut selective,
        )
        .unwrap();
    assert!(result.skipped_parent_layout);
    assert_eq!(scene.snapshot(), incremental);
    println!("equal dependencies: parent layout skipped; adopted geometry matches reference");
}
