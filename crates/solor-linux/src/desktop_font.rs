//! Linux desktop interface font selection at application startup.
use std::{
    path::PathBuf,
    sync::{OnceLock, mpsc},
    time::{Duration, Instant},
};

use dbus::{
    arg::{RefArg, Variant},
    blocking::Connection,
};

/// Reads and caches the desktop's preferred interface family for native windows.
pub fn system_ui_font() -> Option<String> {
    static FAMILY: OnceLock<Option<String>> = OnceLock::new();
    FAMILY
        .get_or_init(|| {
            let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
            let is_desktop = |name: &str| desktop.split(':').any(|d| d.eq_ignore_ascii_case(name));
            let kde = is_desktop("KDE");
            let xfce = is_desktop("XFCE");
            let budget = Duration::from_millis(200);
            let deadline = Instant::now() + budget;
            let (tx, rx) = mpsc::channel();
            // Session bus connection and authentication can also block.
            let worker = std::thread::Builder::new()
                .name("desktop-font".into())
                .spawn(move || {
                    let _ = tx.send(read_settings(kde, xfce, deadline));
                });
            worker
                .ok()
                .and_then(|_| {
                    rx.recv_timeout(deadline.saturating_duration_since(Instant::now()))
                        .ok()
                        .flatten()
                })
                .or_else(|| configured_family(kde))
        })
        .clone()
}

fn read_settings(kde: bool, xfce: bool, deadline: Instant) -> Option<String> {
    let bus = Connection::new_session().ok()?;
    let remaining = || deadline.checked_duration_since(Instant::now());
    if xfce {
        let proxy = bus.with_proxy("org.xfce.Xfconf", "/org/xfce/Xfconf", remaining()?);
        let value: Result<(Variant<Box<dyn RefArg>>,), _> = proxy.method_call(
            "org.xfce.Xfconf",
            "GetProperty",
            ("xsettings", "/Gtk/FontName"),
        );
        if let Some(family) = value
            .ok()
            .and_then(|(v,)| parse_family(v.0.as_str()?, false))
        {
            return Some(family);
        }
    }
    let mut keys = [
        ("org.gnome.desktop.interface", "font-name", false),
        ("org.kde.kdeglobals.General", "font", true),
    ];
    if kde {
        keys.swap(0, 1);
    }
    for (namespace, key, qt) in keys {
        let proxy = bus.with_proxy(
            "org.freedesktop.portal.Desktop",
            "/org/freedesktop/portal/desktop",
            remaining()?,
        );
        let value: Result<(Variant<Box<dyn RefArg>>,), _> =
            proxy.method_call("org.freedesktop.portal.Settings", "Read", (namespace, key));
        if let Some(family) = value.ok().and_then(|(v,)| parse_family(v.0.as_str()?, qt)) {
            return Some(family);
        }
    }
    None
}

fn configured_family(kde: bool) -> Option<String> {
    let user = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")));
    let system = std::env::var_os("XDG_CONFIG_DIRS")
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "/etc/xdg".into());
    let roots = user
        .into_iter()
        .chain(std::env::split_paths(&system).filter(|path| path.is_absolute()));
    let mut files = vec![
        ("gtk-4.0/settings.ini", "Settings", "gtk-font-name", false),
        ("gtk-3.0/settings.ini", "Settings", "gtk-font-name", false),
    ];
    if kde {
        files.insert(0, ("kdeglobals", "General", "font", true));
    }
    for root in roots {
        for (file, section, key, qt) in &files {
            if let Ok(contents) = std::fs::read_to_string(root.join(file))
                && let Some(family) =
                    ini_value(&contents, section, key).and_then(|value| parse_family(value, *qt))
            {
                return Some(family);
            }
        }
    }
    None
}

fn ini_value<'a>(contents: &'a str, section: &str, key: &str) -> Option<&'a str> {
    let mut current = "";
    let mut result = None;
    for line in contents.lines().map(str::trim) {
        if line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if let Some(name) = line
            .strip_prefix('[')
            .and_then(|line| line.strip_suffix(']'))
        {
            current = name;
        } else if current == section
            && let Some((name, value)) = line.split_once('=')
            && name.trim() == key
        {
            result = Some(value.trim());
        }
    }
    result
}

fn parse_family(description: &str, qt: bool) -> Option<String> {
    let mut description = description.trim();
    if qt {
        description = description.split(',').next()?.trim();
    } else {
        // Pango descriptions append style, size, variation and feature fields.
        for marker in [" #", " @"] {
            description = description.split(marker).next()?.trim_end();
        }
        if let Some(last) = description.split_whitespace().next_back() {
            let size = last.strip_suffix("px").unwrap_or(last);
            if size
                .parse::<f64>()
                .is_ok_and(|size| size.is_finite() && size >= 0.0)
            {
                description = description[..description.len() - last.len()].trim_end();
            }
        }
        while let Some(last) = description.split_whitespace().next_back() {
            let normalized = last.to_ascii_lowercase().replace('-', "");
            if !matches!(
                normalized.as_str(),
                "normal"
                    | "roman"
                    | "oblique"
                    | "italic"
                    | "smallcaps"
                    | "allsmallcaps"
                    | "petitecaps"
                    | "allpetitecaps"
                    | "unicase"
                    | "titlecaps"
                    | "thin"
                    | "ultralight"
                    | "extralight"
                    | "light"
                    | "semilight"
                    | "demilight"
                    | "book"
                    | "regular"
                    | "medium"
                    | "semibold"
                    | "demibold"
                    | "bold"
                    | "ultrabold"
                    | "extrabold"
                    | "heavy"
                    | "black"
                    | "ultrablack"
                    | "extrablack"
                    | "ultracondensed"
                    | "extracondensed"
                    | "condensed"
                    | "semicondensed"
                    | "semiexpanded"
                    | "expanded"
                    | "extraexpanded"
                    | "ultraexpanded"
                    | "notrotated"
                    | "south"
                    | "upsidedown"
                    | "north"
                    | "rotatedleft"
                    | "east"
                    | "rotatedright"
                    | "west"
                    | "withcolor"
                    | "withoutcolor"
            ) {
                break;
            }
            description = description[..description.len() - last.len()].trim_end();
        }
        description = description.split(',').next()?.trim();
    }
    (!description.is_empty()).then(|| description.to_owned())
}

#[cfg(test)]
#[path = "../tests/unit/desktop_font.rs"]
mod tests;
