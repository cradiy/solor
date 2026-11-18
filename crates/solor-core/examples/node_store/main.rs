//! Headless workloads for the node store.

mod measure;
mod workload;

fn main() {
    measure::run();
    workload::run();
}
