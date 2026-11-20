use super::*;

#[test]
fn desktop_font_encodings_preserve_family_names() {
    for (description, qt, expected) in [
        (
            "Liberation Sans Semi-Bold 10.5",
            false,
            Some("Liberation Sans"),
        ),
        (
            "Noto Sans CJK SC 13px @wght=450 #tnum=1",
            false,
            Some("Noto Sans CJK SC"),
        ),
        ("Example Heavy, 12", false, Some("Example Heavy")),
        ("Example 2,11,-1,5,400,0,0,0,0,0", true, Some("Example 2")),
        ("Example 2", true, Some("Example 2")),
        ("", false, None),
        ("Bold 12", false, None),
    ] {
        assert_eq!(
            parse_family(description, qt).as_deref(),
            expected,
            "{description}"
        );
    }
}

#[test]
fn settings_files_use_the_requested_section_and_last_value() {
    let ini = "[Other]\ngtk-font-name=Wrong\n[Settings]\n# gtk-font-name=Ignored\ngtk-font-name=First 9\ngtk-font-name = Last 11\n[Other]\ngtk-font-name=Wrong Again\n";
    assert_eq!(ini_value(ini, "Settings", "gtk-font-name"), Some("Last 11"));
    assert_eq!(ini_value(ini, "General", "font"), None);
}
