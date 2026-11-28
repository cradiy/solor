use std::{os::unix::ffi::OsStringExt, path::PathBuf};

pub(crate) fn local_path(line: &str) -> Option<PathBuf> {
    let (scheme, rest) = line.split_once(':')?;
    if !scheme.eq_ignore_ascii_case("file") || rest.contains(['?', '#']) {
        return None;
    }
    let path = if let Some(authority) = rest.strip_prefix("//") {
        let (host, path) = authority.split_at(authority.find('/')?);
        if !host.is_empty() && !host.eq_ignore_ascii_case("localhost") {
            return None;
        }
        path
    } else {
        rest
    };
    if !path.starts_with('/')
        || path.match_indices('%').any(|(i, _)| {
            !path
                .as_bytes()
                .get(i + 1..i + 3)
                .is_some_and(|pair| pair.iter().all(u8::is_ascii_hexdigit))
        })
    {
        return None;
    }
    let decoded = percent_encoding::percent_decode_str(path).collect::<Vec<_>>();
    if decoded.contains(&0) {
        return None;
    }
    Some(PathBuf::from(std::ffi::OsString::from_vec(decoded)))
}
