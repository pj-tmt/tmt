//! Door-served browser assets, disjoint from `/r/` and `/x/`: the pairing page
//! at `/pair/<descriptor>`, the device SDK module at `/sdk/remote-v1.js`, and
//! `/sdk/mount`, which tells a page this run's identity and which mounted
//! extension its path belongs to. None of them carries authority.
use crate::{
    http::{Head, Reply, Request},
    mount::Mounts,
};
use serde_json::{Value, json};

/// Built from `typescript/remote-client` by `pnpm build`; CI checks it is current.
const SDK: &str = include_str!("../assets/remote-v1.js");
const PAGE: &str = include_str!("../assets/pair.html");
/// The page runs only the same-origin SDK module and talks only to this door.
const PAGE_POLICY: &str = "default-src 'none'; script-src 'self'; connect-src 'self'; \
    base-uri 'none'; form-action 'none'; frame-ancestors 'none'";
/// Bound on a `/sdk/mount` body: one page path.
const MOUNT_BODY_BYTES: usize = 2048;

pub struct Pages {
    origin: String,
    machine_id: String,
    window_id: String,
    /// Door origin plus route prefix.
    address: String,
}
impl Pages {
    pub fn new(origin: &str, machine_id: String, window_id: String, prefix: &str) -> Self {
        Self {
            origin: origin.to_owned(),
            machine_id,
            window_id,
            address: format!("{origin}{prefix}"),
        }
    }
    pub fn serves(path: &str) -> bool {
        path.starts_with("/pair/") || path.starts_with("/sdk/")
    }
    pub fn admit(&self, head: &Head<'_>) -> Result<usize, Reply> {
        if head.upgrade {
            return Err(Reply::empty(404));
        }
        match (head.method, head.path) {
            ("GET", path) if path == "/sdk/remote-v1.js" || descriptor(path) => {
                // Navigation omits Origin; a cross-origin load is refused.
                if head.origin.is_some_and(|o| o != self.origin) {
                    return Err(Reply::empty(403));
                }
                Ok(0)
            }
            ("POST", "/sdk/mount") => {
                if head.origin != Some(self.origin.as_str()) {
                    return Err(Reply::empty(403));
                }
                if head.content_type != Some("application/json") {
                    return Err(Reply::empty(400));
                }
                Ok(MOUNT_BODY_BYTES)
            }
            _ => Err(Reply::empty(404)),
        }
    }
    pub fn handle(&self, request: &Request, mounts: &Mounts) -> Reply {
        match request.path.as_str() {
            "/sdk/remote-v1.js" => {
                // The path names the SDK interface version, not a build, so it
                // is not cached across upgrades.
                asset("text/javascript; charset=utf-8", SDK, None)
            }
            "/sdk/mount" => self.mount(&request.body, mounts),
            _ => asset("text/html; charset=utf-8", PAGE, Some(PAGE_POLICY)),
        }
    }
    /// `{path}` to this run and the page's own mount, from the door's mapping.
    /// It scopes honest use only: mounted extensions share one trust domain.
    fn mount(&self, body: &[u8], mounts: &Mounts) -> Reply {
        let path = match serde_json::from_slice::<Value>(body) {
            Ok(Value::Object(object)) if object.len() == 1 => object
                .get("path")
                .and_then(Value::as_str)
                .map(str::to_owned),
            _ => None,
        };
        let Some(path) = path else {
            return Reply::empty(400);
        };
        let extension = mounts.extension_of(&path);
        let mut reply = Reply::empty(200);
        reply.body = json!({
            "machineId": self.machine_id,
            "windowId": self.window_id,
            "address": self.address,
            "extension": extension,
            "mount": extension.map(|name| format!("/x/{name}/")),
        })
        .to_string()
        .into_bytes();
        reply
    }
}
/// `/pair/<base64url descriptor>`; the page itself reads the descriptor.
fn descriptor(path: &str) -> bool {
    path.strip_prefix("/pair/").is_some_and(|encoded| {
        (1..=4096).contains(&encoded.len())
            && encoded
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    })
}
fn asset(content_type: &str, body: &str, policy: Option<&str>) -> Reply {
    let mut reply = Reply::empty(200);
    reply.headers = vec![("content-type".into(), content_type.into())];
    if let Some(policy) = policy {
        reply
            .headers
            .push(("content-security-policy".into(), policy.into()));
    }
    reply.body = body.as_bytes().to_vec();
    reply
}
