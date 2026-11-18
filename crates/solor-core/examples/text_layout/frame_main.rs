//! Atomic text drawing snapshots over the private headless layout experiment.

#[allow(dead_code)]
#[path = "../layout_contract/column.rs"]
mod column;
use solor_core::layout;
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
