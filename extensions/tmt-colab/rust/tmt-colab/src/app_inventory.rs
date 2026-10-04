//! Shared build-time and runtime admission of exact browser asset bytes.
use std::collections::BTreeMap;

pub const APP_BYTES: usize = 16 * 1024 * 1024;
pub const APP_FILES: usize = 128;

/// The top-level app files, declared once for this inventory and the packaging proof.
const ENTRIES: &str = include_str!("../app-entries.txt");

/// Top-level file names of one kind (`html` or `text`) from `app-entries.txt`.
fn entries(kind: &'static str) -> impl Iterator<Item = &'static str> {
    ENTRIES.lines().filter_map(move |line| {
        let (declared, name) = line.split_once(' ')?;
        (declared == kind).then_some(name)
    })
}

pub fn content_type(route: &str) -> Option<&'static str> {
    let top = route.strip_prefix('/')?;
    if entries("html").any(|entry| entry == top) {
        return Some("text/html; charset=utf-8");
    }
    if entries("text").any(|entry| entry == top) {
        return Some("text/plain; charset=utf-8");
    }
    let name = route.strip_prefix("/assets/")?;
    if name.starts_with('.')
        || name.is_empty()
        || name.len() > 128
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
    {
        return None;
    }
    Some(match name.rsplit('.').next()? {
        "html" => "text/html; charset=utf-8",
        "txt" => "text/plain; charset=utf-8",
        "js" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "svg" => "image/svg+xml",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        _ => return None,
    })
}

pub fn validate(files: &[(&str, &[u8])]) -> Result<(), &'static str> {
    if files.len() > APP_FILES {
        return Err("App has too many assets.");
    }
    let mut total = 0usize;
    let mut inventory = BTreeMap::new();
    for &(route, bytes) in files {
        if content_type(route).is_none() {
            return Err("Unsupported or unsafe app asset route.");
        }
        total = total.checked_add(bytes.len()).ok_or("App size overflow.")?;
        if bytes.is_empty() || total > APP_BYTES {
            return Err("App assets must be nonempty and bounded.");
        }
        if inventory.insert(route, bytes).is_some() {
            return Err("Duplicate app asset route.");
        }
    }
    for name in entries("html") {
        if !inventory.contains_key(format!("/{name}").as_str()) {
            return Err("A declared app entry is missing.");
        }
    }
    for name in entries("html") {
        if let Some(bytes) = inventory.get(format!("/{name}").as_str()) {
            let html = std::str::from_utf8(bytes).map_err(|_| "App HTML must be UTF-8.")?;
            for reference in html.split("\"./assets/").skip(1) {
                let name = reference
                    .split('"')
                    .next()
                    .ok_or("Invalid app entry reference.")?;
                if !inventory.contains_key(format!("/assets/{name}").as_str()) {
                    return Err("App entry asset is missing.");
                }
            }
        }
    }
    for kind in ["text/javascript; charset=utf-8", "text/css; charset=utf-8"] {
        if !inventory
            .keys()
            .any(|route| content_type(route) == Some(kind))
        {
            return Err("App build requires JavaScript and CSS.");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declared_entries_are_the_only_top_level_files() {
        let html: Vec<_> = entries("html").collect();
        assert_eq!(html, ["index.html", "renderer.html", "reader.html"]);
        assert_eq!(
            entries("text").collect::<Vec<_>>(),
            ["THIRD-PARTY-NOTICES.txt"]
        );
        for name in &html {
            assert_eq!(
                content_type(&format!("/{name}")),
                Some("text/html; charset=utf-8")
            );
        }
        assert_eq!(
            content_type("/THIRD-PARTY-NOTICES.txt"),
            Some("text/plain; charset=utf-8")
        );
        for undeclared in ["/other.html", "/index.js", "/", "/app-entries.txt"] {
            assert_eq!(content_type(undeclared), None, "{undeclared}");
        }
    }

    #[test]
    fn every_declared_html_entry_is_required() {
        let page: &[u8] = b"<html></html>";
        let complete = [
            ("/index.html", page),
            ("/renderer.html", page),
            ("/reader.html", page),
            ("/assets/a.js", b"js"),
            ("/assets/a.css", b"css"),
        ];
        assert_eq!(validate(&complete), Ok(()));
        for missing in ["/index.html", "/renderer.html", "/reader.html"] {
            let files: Vec<_> = complete
                .iter()
                .copied()
                .filter(|f| f.0 != missing)
                .collect();
            assert!(validate(&files).is_err(), "{missing}");
        }
    }
}
