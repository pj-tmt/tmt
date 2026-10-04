//! Shared build-time and runtime admission of exact browser asset bytes.
use std::collections::BTreeMap;

pub const APP_BYTES: usize = 16 * 1024 * 1024;
pub const APP_FILES: usize = 128;

pub fn content_type(route: &str) -> Option<&'static str> {
    let name = match route {
        "/index.html" | "/renderer.html" | "/reader.html" => {
            return Some("text/html; charset=utf-8");
        }
        "/THIRD-PARTY-NOTICES.txt" => return Some("text/plain; charset=utf-8"),
        _ => route.strip_prefix("/assets/")?,
    };
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
    if !inventory.contains_key("/index.html") {
        return Err("App index is missing.");
    }
    if !inventory.contains_key("/renderer.html") {
        return Err("App renderer is missing.");
    }
    if !inventory.contains_key("/reader.html") {
        return Err("App reader entry is missing.");
    }
    for route in ["/index.html", "/renderer.html", "/reader.html"] {
        if let Some(bytes) = inventory.get(route) {
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
