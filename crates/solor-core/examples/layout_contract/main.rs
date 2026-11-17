//! Headless layout contract experiment, not a public UI API.

mod fixtures;
mod layout;

// Reuse the access prototype; this target exercises only part of its API.
#[allow(dead_code)]
#[path = "../node_store/store.rs"]
mod store;

#[cfg(test)]
mod tests;

use fixtures::{Wrapping, nested};

fn main() {
    let (mut incremental, ids) = nested();
    let (mut reference, reference_ids) = nested();
    incremental.run(100, false).unwrap();
    reference.run(100, true).unwrap();
    assert_eq!(incremental.snapshot(), reference.snapshot());
    let before = incremental.counts();
    incremental
        .edit_layout::<Wrapping>(ids.text, |text| text.units = 100)
        .unwrap();
    reference
        .edit_layout::<Wrapping>(reference_ids.text, |text| text.units = 100)
        .unwrap();
    let published = incremental.run(100, false).unwrap();
    reference.run(100, true).unwrap();
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
    assert_eq!(incremental.run(100, false), Ok(0));
    reference.run(100, true).unwrap();
    assert_eq!(incremental.snapshot(), reference.snapshot());
    assert_eq!(incremental.counts(), before);
    println!("ink edit: no measurement or layout; geometry matches full recomputation");
}
