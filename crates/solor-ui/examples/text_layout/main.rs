//! Fixed-font, headless text integration; not a public text or widget API.

use solor_core::layout;

mod font_fixture;
#[cfg(test)]
mod tests;
mod workload;

fn main() {
    workload::verify();
    workload::run();
}
