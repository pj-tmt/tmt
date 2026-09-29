//! Human forms of values. Full values stay in `--json`; these are for reading.

use std::path::Path;

/// Characters of an identifier shown in human output.
pub const SHORT_ID: usize = 8;

/// `~/…` for paths under `home`; other paths unchanged.
pub fn home_path(path: &Path, home: Option<&Path>) -> String {
    match home.and_then(|home| path.strip_prefix(home).ok()) {
        Some(rest) if rest.as_os_str().is_empty() => "~".into(),
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

/// The first [`SHORT_ID`] characters of an identifier.
pub fn short_id(id: &str) -> &str {
    id.char_indices()
        .nth(SHORT_ID)
        .map_or(id, |(end, _)| &id[..end])
}

/// `driver:identifier`, with the identifier shortened.
pub fn address(driver: &str, id: &str) -> String {
    format!("{driver}:{}", short_id(id))
}

/// `just now`, `45s ago`, `3m ago`, `2h ago`, `5d ago`, from elapsed time.
pub fn relative_time(elapsed_ms: u64) -> String {
    let seconds = elapsed_ms / 1000;
    match seconds {
        0..5 => "just now".into(),
        5..60 => format!("{seconds}s ago"),
        60..3_600 => format!("{}m ago", seconds / 60),
        3_600..86_400 => format!("{}h ago", seconds / 3_600),
        _ => format!("{}d ago", seconds / 86_400),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_are_home_abbreviated_only_under_home() {
        let home = Path::new("/Users/ada");
        assert_eq!(home_path(Path::new("/Users/ada"), Some(home)), "~");
        assert_eq!(
            home_path(Path::new("/Users/ada/dev/tmt"), Some(home)),
            "~/dev/tmt"
        );
        assert_eq!(
            home_path(Path::new("/Users/adam/dev"), Some(home)),
            "/Users/adam/dev"
        );
        assert_eq!(home_path(Path::new("/tmp/x"), None), "/tmp/x");
    }

    #[test]
    fn identifiers_shorten_to_eight_characters() {
        assert_eq!(short_id("e1c9ab12-77aa-4c3d"), "e1c9ab12");
        assert_eq!(short_id("abc"), "abc");
        assert_eq!(short_id("ééééééééé"), "éééééééé");
        assert_eq!(address("claude", "e1c9ab12-77aa"), "claude:e1c9ab12");
    }

    #[test]
    fn times_are_relative_in_the_largest_whole_unit() {
        let cases = [
            (0, "just now"),
            (4_999, "just now"),
            (5_000, "5s ago"),
            (59_999, "59s ago"),
            (60_000, "1m ago"),
            (3 * 60_000, "3m ago"),
            (3_600_000, "1h ago"),
            (86_400_000, "1d ago"),
            (5 * 86_400_000, "5d ago"),
        ];
        for (elapsed, text) in cases {
            assert_eq!(relative_time(elapsed), text, "{elapsed}");
        }
    }
}
