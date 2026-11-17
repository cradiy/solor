//! Headless storage-access experiment, not a public UI API.

mod measure;
mod store;
mod workload;

#[cfg(test)]
mod tests;

fn main() {
    measure::run();
    workload::run();
}
