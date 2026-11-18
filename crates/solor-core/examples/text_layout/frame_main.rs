//! Atomic text drawing snapshots over the private headless layout experiment.
const MEASUREMENT_CACHE_SLOTS: usize = 3;

#[allow(dead_code)]
#[path = "../layout_contract/fixtures.rs"]
mod fixtures;
#[allow(dead_code)]
#[path = "../layout_contract/layout.rs"]
mod layout;
#[allow(dead_code)]
mod text;

mod frame;
mod frame_cost;
#[cfg(test)]
mod frame_tests;

fn main() {
    frame_cost::verify();
    frame_cost::run();
}
