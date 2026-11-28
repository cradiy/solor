use std::{cell::Cell, path::PathBuf};

use anyhow::{Context, Result, ensure};
use block2::RcBlock;
use futures_channel::oneshot;
use objc2_app_kit::{NSApplication, NSModalResponseCancel, NSModalResponseOK, NSOpenPanel};
use objc2_foundation::{MainThreadMarker, NSArray, NSString, NSURL};
use objc2_uniform_type_identifiers::UTType;
use solor::platform::FilePickerOptions;

struct PendingPanel<'a>(Option<&'a NSOpenPanel>);

impl Drop for PendingPanel<'_> {
    fn drop(&mut self) {
        let Some(panel) = self.0 else { return };
        #[allow(unsafe_code)]
        // SAFETY: This main-thread panel accepts nil as the sender of its cancel action.
        unsafe {
            panel.cancel(None)
        };
    }
}

/// Opens an AppKit file picker on the main thread. Cancelling returns an empty list.
pub async fn select_files(options: FilePickerOptions, multiple: bool) -> Result<Vec<PathBuf>> {
    options.validate()?;
    let mtm = MainThreadMarker::new().context("open the file picker on the main thread")?;
    let panel = NSOpenPanel::openPanel(mtm);
    panel.setCanChooseFiles(!options.directories);
    panel.setCanChooseDirectories(options.directories);
    panel.setAllowsMultipleSelection(multiple);
    if let Some(title) = &options.title {
        panel.setTitle(Some(&NSString::from_str(title)));
    }
    if let Some(label) = &options.accept_label {
        panel.setPrompt(Some(&NSString::from_str(label)));
    }
    if let Some(dir) = &options.initial_directory {
        let url = NSURL::from_directory_path(dir).context("invalid picker directory")?;
        panel.setDirectoryURL(Some(&url));
    }
    if !options.filters.is_empty() {
        let types = options
            .filters
            .iter()
            .flat_map(|filter| &filter.extensions)
            .map(|ext| {
                UTType::typeWithFilenameExtension(&NSString::from_str(ext))
                    .with_context(|| format!("cannot resolve file type for extension {ext:?}"))
            })
            .collect::<Result<Vec<_>>>()?;
        panel.setAllowedContentTypes(&NSArray::from_retained_slice(&types));
    }
    let (tx, rx) = oneshot::channel();
    let tx = Cell::new(Some(tx));
    let completed = RcBlock::new(move |status| {
        if let Some(tx) = tx.take() {
            let _ = tx.send(status);
        }
    });
    if let Some(window) = NSApplication::sharedApplication(mtm).keyWindow() {
        panel.beginSheetModalForWindow_completionHandler(&window, &completed);
    } else {
        panel.beginWithCompletionHandler(&completed);
    }
    let mut pending = PendingPanel(Some(&panel));
    let status = rx.await?;
    pending.0 = None;
    if status == NSModalResponseCancel {
        return Ok(Vec::new());
    }
    ensure!(
        status == NSModalResponseOK,
        "file picker ended with response {status}"
    );
    let urls = panel.URLs();
    (0..urls.count())
        .map(|index| {
            urls.objectAtIndex(index)
                .to_file_path()
                .context("file picker returned an invalid file URL")
        })
        .collect()
}
