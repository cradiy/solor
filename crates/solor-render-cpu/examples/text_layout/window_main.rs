//! Linux Wayland presentation of the fixed two-row text experiment.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

mod disclosure;
mod disclosure_layout;
#[allow(dead_code)]
mod frame;
#[cfg(test)]
use solor_render_cpu::{damage, glyph_masks, pixels};
mod input;
#[cfg(test)]
mod lifecycle_tests;
use solor::layout;
#[allow(dead_code)]
mod font_fixture;
mod window_content;

#[cfg(test)]
#[path = "../../tests/window/submission.rs"]
mod submission_tests;
#[cfg(any(target_os = "linux", test))]
mod window_renderer;
#[cfg(target_os = "linux")]
fn main() -> anyhow::Result<()> {
    use window_content::Content;

    if let Some(mode) = std::env::var_os("SOLOR_DISCLOSURE") {
        let mut row = if mode == "long-list" {
            let labels: Vec<_> = (3..=1024).map(|n| format!("Item {n}")).collect();
            let tail: Vec<_> = labels.iter().map(String::as_str).collect();
            disclosure::Disclosure::new(&["Item 1"], &tail)
        } else if mode == "list" {
            disclosure::Disclosure::new(&["Previous item"], &["Next item"])
        } else {
            disclosure::Disclosure::new(&[], &[])
        }
        .map_err(|err| std::io::Error::other(format!("disclosure fixture: {err:?}")))?;
        row.set_detail("Hello 世界！Rust GUI 123", disclosure::DETAIL_COLOR)?;
        run(
            "Solor disclosure",
            "solor.disclosure",
            Content::disclosure(row),
        )
    } else {
        let text = Content::text()
            .map_err(|err| std::io::Error::other(format!("text fixture: {err:?}")))?;
        run("Solor text window", "solor.text-window", text)
    }
}

#[cfg(not(target_os = "linux"))]
fn main() -> std::process::ExitCode {
    eprintln!("text_window requires Linux with a Wayland compositor");
    std::process::ExitCode::FAILURE
}

#[cfg(target_os = "linux")]
fn run(title: &str, app_id: &str, view: window_content::Content) -> anyhow::Result<()> {
    use solor::{
        platform::{Platform, WindowOptions},
        px,
    };
    use solor_linux::Wayland;
    let audit = std::env::var_os("SOLOR_VERIFY_SHM").is_some();
    let platform = Wayland {
        verify_shm: audit,
        trace_submissions: true,
    };
    platform.run_window(
        WindowOptions::new(title)
            .app_id(app_id)
            .size(px(320.0), px(100.0)),
        Box::new(window_renderer::Renderer::new(view, audit)),
    )
}
