// Both targets compile the same implementation with different inline cache sizes.
use super::MEASUREMENT_CACHE_SLOTS;

#[path = "../layout_contract/column.rs"]
mod column;
use solor_core::{node_store, tree};
#[allow(dead_code)]
#[path = "../layout_contract/fixtures.rs"]
mod fixtures;
#[allow(dead_code)]
#[path = "../layout_contract/layout.rs"]
mod layout;
#[allow(dead_code)]
#[path = "../layout_contract/negotiation.rs"]
mod negotiation;

#[path = "workload.rs"]
mod workload;

pub fn run() {
    workload::run();
}
