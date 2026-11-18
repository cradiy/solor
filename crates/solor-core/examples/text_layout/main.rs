//! Fixed-font, headless text integration; not a public text or widget API.

#[allow(dead_code)]
#[path = "../layout_contract/column.rs"]
mod column;
use solor_core::layout;

#[cfg(test)]
mod tests;
mod text;
mod workload;

fn main() {
    workload::verify();
    workload::run();
}
