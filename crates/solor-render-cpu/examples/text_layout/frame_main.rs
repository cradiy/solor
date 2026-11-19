//! Atomic text drawing snapshots over the private headless layout experiment.

use solor::layout;
#[allow(dead_code)]
mod font_fixture;

mod frame;
mod frame_cost;
#[cfg(test)]
mod frame_tests;

fn main() {
    frame_cost::verify();
    frame_cost::run();
}
