use std::{os::unix::ffi::OsStrExt, path::PathBuf, thread, time::Duration};

use anyhow::{Context, Result, bail, ensure};
use dbus::{
    Message, Path,
    arg::{PropMap, Variant, prop_cast},
    blocking::{Connection, stdintf::org_freedesktop_dbus::Properties},
    message::MatchRule,
};
use futures_channel::oneshot;
use solor::platform::FilePickerOptions;

const PORTAL: &str = "org.freedesktop.portal.Desktop";
const DESKTOP_PATH: &str = "/org/freedesktop/portal/desktop";
const CHOOSER: &str = "org.freedesktop.portal.FileChooser";
const REQUEST: &str = "org.freedesktop.portal.Request";
const CALL_TIMEOUT: Duration = Duration::from_secs(25);

/// Opens the desktop portal's file chooser. Cancelling returns an empty list.
pub async fn select_files(options: FilePickerOptions, multiple: bool) -> Result<Vec<PathBuf>> {
    options.validate()?;
    let (tx, rx) = oneshot::channel();
    thread::Builder::new()
        .name("solor-file-picker".into())
        .spawn(move || {
            let result = open_files(options, multiple, || tx.is_canceled());
            let _ = tx.send(result);
        })?;
    rx.await?
}

fn open_files(
    picker: FilePickerOptions,
    multiple: bool,
    cancelled: impl Fn() -> bool,
) -> Result<Vec<PathBuf>> {
    let conn = Connection::new_session().context("connect to the desktop session bus")?;
    // Subscribe before OpenFile: the portal may answer before its method reply is dispatched.
    let response = MatchRule::new_signal(REQUEST, "Response").with_sender(PORTAL);
    conn.add_match_no_cb(&response.match_str())?;
    let owner_changed = MatchRule::new_signal("org.freedesktop.DBus", "NameOwnerChanged")
        .with_strict_sender("org.freedesktop.DBus");
    conn.add_match_no_cb(&format!("{},arg0='{PORTAL}'", owner_changed.match_str()))?;

    let mut options = PropMap::new();
    options.insert("multiple".into(), Variant(Box::new(multiple)));
    if picker.directories {
        let portal = conn.with_proxy(PORTAL, DESKTOP_PATH, CALL_TIMEOUT);
        let version: u32 = portal.get(CHOOSER, "version")?;
        ensure!(
            version >= 3,
            "the desktop portal does not support selecting folders"
        );
        options.insert("directory".into(), Variant(Box::new(true)));
    }
    if let Some(label) = picker.accept_label {
        options.insert("accept_label".into(), Variant(Box::new(label)));
    }
    if let Some(directory) = picker.initial_directory {
        let mut bytes = std::path::absolute(directory)?
            .as_os_str()
            .as_bytes()
            .to_vec();
        bytes.push(0);
        options.insert("current_folder".into(), Variant(Box::new(bytes)));
    }
    if !picker.filters.is_empty() {
        let filters: Vec<_> = picker
            .filters
            .into_iter()
            .map(|filter| {
                let patterns: Vec<_> = filter
                    .extensions
                    .iter()
                    .map(|ext| (0u32, extension_glob(ext)))
                    .collect();
                (filter.name, patterns)
            })
            .collect();
        options.insert("filters".into(), Variant(Box::new(filters)));
    }
    let title = picker
        .title
        .as_deref()
        .unwrap_or(match (picker.directories, multiple) {
            (true, true) => "Open Folders",
            (true, false) => "Open Folder",
            (false, true) => "Open Files",
            (false, false) => "Open File",
        });
    let call = Message::new_method_call(PORTAL, DESKTOP_PATH, CHOOSER, "OpenFile")
        .map_err(anyhow::Error::msg)?
        .append3("", title, options);
    let channel = conn.channel();
    let reply = channel
        .send_with_reply_and_block(call, CALL_TIMEOUT)
        .context("open the system file chooser")?;
    let path: Path = reply.read1()?;
    let owner = reply.sender().context("missing portal sender")?;
    let response = response
        .with_strict_sender(owner.clone())
        .with_path(path.clone());

    loop {
        if cancelled() {
            let proxy = conn.with_proxy(owner, path, CALL_TIMEOUT);
            let _: () = proxy.method_call(REQUEST, "Close", ())?;
            return Ok(Vec::new());
        }
        let Some(msg) = channel.blocking_pop_message(Duration::from_millis(500))? else {
            continue;
        };
        if response.matches(&msg) {
            let (status, values): (u32, PropMap) = msg.read2()?;
            if status == 1 {
                return Ok(Vec::new());
            }
            ensure!(status == 0, "file chooser ended with response {status}");
            let uris = prop_cast::<Vec<String>>(&values, "uris")
                .context("file chooser returned no URI list")?;
            ensure!(
                !uris.is_empty() && (multiple || uris.len() == 1),
                "unexpected file selection size"
            );
            return uris
                .iter()
                .map(|uri| {
                    crate::file_uri::local_path(uri)
                        .context("file chooser returned a non-local or invalid URI")
                })
                .collect();
        }
        if owner_changed.matches(&msg) {
            let (name, previous, _): (&str, &str, &str) = msg.read3()?;
            if name == PORTAL && previous == &*owner {
                bail!("desktop portal disconnected while selecting files");
            }
        }
    }
}

fn extension_glob(extension: &str) -> String {
    let mut glob = String::from("*.");
    for c in extension.chars() {
        if c.is_ascii_alphabetic() {
            glob.extend(['[', c.to_ascii_lowercase(), c.to_ascii_uppercase(), ']']);
        } else {
            glob.push(c);
        }
    }
    glob
}
