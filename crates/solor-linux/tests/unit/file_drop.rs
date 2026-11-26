use super::local_paths;
use std::{ffi::OsStr, os::unix::ffi::OsStrExt, path::Path};

#[test]
fn extracts_only_local_file_uris_and_preserves_unix_path_bytes() {
    let paths = local_paths(concat!(
        "# source comment\r\nfile://localhost/tmp/a%20b\r\n",
        "file://other-host/shared\nhttps://example.org/doc\n",
        "file:relative\nfile:///tmp/incomplete%2\nfile:///tmp/null%00byte\n",
        "file:///tmp/%FF%23\n",
    ));
    assert_eq!(
        paths,
        [
            Path::new("/tmp/a b"),
            Path::new(OsStr::from_bytes(b"/tmp/\xff#"))
        ]
    );
}
