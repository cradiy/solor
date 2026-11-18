//! Linux Wayland presentation of the fixed two-row text experiment.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

#[allow(dead_code)]
mod damage;
mod disclosure;
mod disclosure_layout;
#[allow(dead_code)]
#[path = "../layout_contract/fixtures.rs"]
mod fixtures;
#[allow(dead_code)]
mod frame;
#[allow(dead_code)]
mod glyph_masks;
mod input;
#[allow(dead_code)]
#[path = "../layout_contract/layout.rs"]
mod layout;
#[allow(dead_code)]
mod pixels;
#[allow(dead_code)]
mod text;
mod window_content;

const MEASUREMENT_CACHE_SLOTS: usize = 3;

#[cfg(target_os = "linux")]
mod wayland;
#[cfg(target_os = "linux")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use window_content::Content;

    if let Some(mode) = std::env::var_os("SOLOR_DISCLOSURE") {
        let detail = "Hello 世界！Rust GUI 123";
        let row = if mode == "long-list" {
            let labels: Vec<_> = (3..=1024).map(|n| format!("Item {n}")).collect();
            let tail: Vec<_> = labels.iter().map(String::as_str).collect();
            disclosure::Disclosure::new(detail, &["Item 1"], &tail)
        } else if mode == "list" {
            disclosure::Disclosure::new(detail, &["Previous item"], &["Next item"])
        } else {
            disclosure::Disclosure::new(detail, &[], &[])
        }
        .map_err(|err| std::io::Error::other(format!("disclosure fixture: {err:?}")))?;
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
