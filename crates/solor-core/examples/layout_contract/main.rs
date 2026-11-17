//! Headless layout contract experiment, not a public UI API.

const MEASUREMENT_CACHE_SLOTS: usize = 3;

mod fixtures;
mod layout;
mod negotiation;

// Reuse the access prototype; this target exercises only part of its API.
#[allow(dead_code)]
#[path = "../node_store/store.rs"]
mod store;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod negotiation_tests;

use fixtures::{Wrapping, nested};
use layout::Query;

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
    negotiation::run();
}
