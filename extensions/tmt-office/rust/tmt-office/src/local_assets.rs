include!(concat!(env!("OUT_DIR"), "/office_assets.rs"));

pub fn find(path: &str) -> Option<(&'static str, &'static [u8])> {
    let path = if path == "/local" {
        "/index.html"
    } else {
        path
    };
    ASSETS
        .iter()
        .find(|(route, _, _)| *route == path)
        .map(|(_, content_type, bytes)| (*content_type, *bytes))
}

pub fn index() -> (&'static str, &'static [u8]) {
    find("/index.html").expect("the embedded Office SPA always has an index")
}

pub fn prove() -> bool {
    let (_, index) = index();
    !index.is_empty()
        && ASSETS.iter().any(|(route, content_type, bytes)| {
            route.starts_with("/assets/")
                && *content_type == "text/javascript; charset=utf-8"
                && !bytes.is_empty()
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_inventory_has_nested_route_fallback_and_types() {
        assert!(prove());
        assert_eq!(find("/local"), Some(index()));
        assert!(ASSETS.iter().all(|(route, content_type, bytes)| {
            route.starts_with('/')
                && !route.contains("..")
                && match route.rsplit('.').next() {
                    Some("html") => *content_type == "text/html; charset=utf-8",
                    Some("js") => *content_type == "text/javascript; charset=utf-8",
                    Some("css") => *content_type == "text/css; charset=utf-8",
                    Some("png") => {
                        *content_type == "image/png" && bytes.starts_with(b"\x89PNG\r\n\x1a\n")
                    }
                    _ => false,
                }
                && !bytes.is_empty()
        }));
    }
}
