//! Linux Wayland presentation of the fixed two-row text experiment.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

#[allow(dead_code)]
mod damage;
mod disclosure;
mod disclosure_layout;
#[allow(dead_code)]
mod frame;
#[allow(dead_code)]
mod glyph_masks;
mod input;
#[cfg(test)]
mod lifecycle_tests;
use solor_core::layout;
#[allow(dead_code)]
mod font_fixture;
#[allow(dead_code)]
mod pixels;
mod window_content;

#[cfg(target_os = "linux")]
mod wayland;
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
        wayland::run(
            "Solor disclosure",
            "solor.disclosure",
            Content::disclosure(row),
        )
    } else {
        let text = Content::text()
            .map_err(|err| std::io::Error::other(format!("text fixture: {err:?}")))?;
        wayland::run("Solor text window", "solor.text-window", text)
    }
}

#[cfg(not(target_os = "linux"))]
fn main() -> std::process::ExitCode {
    eprintln!("text_window requires Linux with a Wayland compositor");
    std::process::ExitCode::FAILURE
}
