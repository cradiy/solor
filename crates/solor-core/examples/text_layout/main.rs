//! Fixed-font, headless text integration; not a public text or widget API.
const MEASUREMENT_CACHE_SLOTS: usize = 3;

#[allow(dead_code)]
#[path = "../layout_contract/fixtures.rs"]
mod fixtures;
#[allow(dead_code)]
#[path = "../layout_contract/layout.rs"]
mod layout;
#[allow(dead_code)]
#[path = "../node_store/store.rs"]
mod store;

#[cfg(test)]
mod tests;
mod text;
mod workload;

fn main() {
    workload::verify();
    workload::run();
}
