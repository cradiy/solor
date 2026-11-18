//! Layout workload with one cached measurement per node.
const MEASUREMENT_CACHE_SLOTS: usize = 1;

mod shared;

fn main() {
    shared::run();
}
