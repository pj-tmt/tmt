//! Mounted page destinations and prefixes over the verified catalog. No alias authority.
/// Remote alone supplies this absolute same-origin mounted root on the private socket.
/// No relative, foreign-origin or request-path-derived redirect is permitted.
pub fn mounted_root(mount: Option<&str>) -> Option<&str> {
    let mount = mount?;
    let prefix = mount.strip_prefix("/r/")?.strip_suffix("/x/colab/")?;
    (prefix.len() == 16
        && prefix
            .bytes()
            .all(|b| matches!(b, b'a'..=b'z' | b'2'..=b'7')))
    .then_some(mount)
}
pub const MIN_PREFIX: usize = 8;
pub fn valid_prefix(prefix: &str) -> bool {
    (MIN_PREFIX..=36).contains(&prefix.len())
        && prefix.bytes().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b == b'-'
            } else {
                b.is_ascii_digit() || (b'a'..=b'f').contains(&b)
            }
        })
}
/// A missing catalog entry retains its full ID: it must never point at another page.
pub fn shortest_id<'a>(page: &'a str, pages: &[String]) -> &'a str {
    if !pages.iter().any(|id| id == page) {
        return page;
    }
    for length in MIN_PREFIX..page.len() {
        let prefix = &page[..length];
        if pages.iter().filter(|id| id.starts_with(prefix)).count() == 1 {
            return prefix;
        }
    }
    page
}
/// Previously printed prefixes can become ambiguous. Callers must show every match.
pub fn matches<'a>(prefix: &str, pages: &'a [String]) -> Vec<&'a str> {
    if !valid_prefix(prefix) {
        return Vec::new();
    }
    pages
        .iter()
        .filter(|id| id.starts_with(prefix))
        .map(String::as_str)
        .collect()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_past_link_becomes_a_choice_instead_of_selecting_another_page() {
        let original = "12345678-0000-4000-8000-000000000001";
        let mut pages = vec![original.to_owned()];
        let past = shortest_id(original, &pages).to_owned();
        assert_eq!(past, "12345678");
        pages.push("12345678-1000-4000-8000-000000000002".to_owned());
        assert_eq!(matches(&past, &pages), vec![original, pages[1].as_str()]);
        assert_eq!(shortest_id(original, &pages), "12345678-0");
        pages.remove(0);
        assert_eq!(shortest_id(original, &pages), original);
        assert!(matches(original, &pages).is_empty());
    }
    #[test]
    fn prefixes_are_bounded_canonical_uuid_prefixes() {
        for bad in [
            "1234567",
            "12345678A",
            "12345678/",
            "12345678-?",
            "FFFFFFFF",
            "123456780",
            "12345678-0000-4000-8000-0000000000010",
        ] {
            assert!(!valid_prefix(bad), "{bad}");
        }
        assert!(valid_prefix("12345678"));
        assert!(valid_prefix("12345678-0000-4000-8000-000000000001"));
    }
}
