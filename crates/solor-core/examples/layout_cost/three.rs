//! Layout workload with three cached measurements per node.
const MEASUREMENT_CACHE_SLOTS: usize = 3;

mod shared;

fn main() {
    shared::run();
}
