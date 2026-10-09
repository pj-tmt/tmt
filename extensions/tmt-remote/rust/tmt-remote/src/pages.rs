//! Static landing and pairing pages, the Colab short alias, page refusals and
//! their stylesheet, outside the route prefix. `/sdk/mount` tells a page this
//! run's identity and which mounted extension its path belongs to. None of these
//! routes carries authority.
use crate::{
    http::{Head, Reply, Request},
    mount::Mounts,
    pairing::Pairing,
};
use serde_json::{Value, json};
use std::sync::Arc;

/// Built from `typescript/remote-client` by `pnpm build`; CI checks it is current.
const SDK: &str = include_str!("../assets/remote-v1.js");
const BOOTSTRAP: &str = include_str!("../assets/pair.js");
const PAGE: &str = include_str!("../assets/pair.html");
const ENTRY: &str = include_str!("../assets/landing.js");
const LANDING: &str = include_str!("../assets/landing.html");
const ERROR: &str = include_str!("../assets/error.html");
const SETTINGS: &str = include_str!("../assets/settings.html");
const SETTINGS_SCRIPT: &str = include_str!("../assets/settings-v1.js");
const SETTINGS_STYLE: &str = include_str!("../assets/settings.css");
// Checked shared presentation plus Remote-owned layout; no Cargo-time generator.
const STYLE: &str = concat!(
    include_str!("../../../../../design/browser-ui/generated/static.css"),
    "\n",
    include_str!("../assets/pages.css"),
);
/// The page runs only the same-origin SDK module and talks only to this door.
const PAGE_POLICY: &str = "default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; \
    base-uri 'none'; form-action 'none'; frame-ancestors 'none'";
/// Bound on a `/sdk/mount` body: one page path.
const MOUNT_BODY_BYTES: usize = 2048;

pub struct Pages {
    pairing: Option<Arc<Pairing>>,
    origin: String,
    machine_id: String,
    window_id: String,
    /// Door origin plus route prefix.
    address: String,
}
impl Pages {
    pub fn new(origin: &str, machine_id: String, window_id: String, prefix: &str) -> Self {
        Self {
            pairing: None,
            origin: origin.to_owned(),
            machine_id,
            window_id,
            address: format!("{origin}{prefix}"),
        }
    }
    pub fn with_pairing(mut self, pairing: Arc<Pairing>) -> Self {
        self.pairing = Some(pairing);
        self
    }
    pub fn serves(path: &str) -> bool {
        path == "/"
            || path == "/settings"
            || path == "/pair"
            || path.starts_with("/pair/")
            || short_page_route(path)
            || path.starts_with("/sdk/")
    }
    pub fn admit(&self, head: &Head<'_>) -> Result<usize, Reply> {
        if head.upgrade {
            return Err(if short_page_route(head.path) {
                page_refusal(head.path, 404)
            } else {
                Reply::empty(404)
            });
        }
        match (head.method, head.path) {
            ("GET", path)
                if matches!(
                    path,
                    "/" | "/settings"
                        | "/pair"
                        | "/sdk/pair.js"
                        | "/sdk/landing.js"
                        | "/sdk/pair-offer"
                        | "/sdk/remote-v1.js"
                        | "/sdk/pages.css"
                        | "/sdk/settings-v1.js"
                        | "/sdk/settings.css"
                ) || path.starts_with("/pair/")
                    || short_page_route(path) =>
            {
                // Navigation omits Origin; a cross-origin load is refused.
                if head.origin.is_some_and(|o| o != self.origin) {
                    return Err(page_refusal(path, 403));
                }
                if path.starts_with("/pair/")
                    || (short_page_route(path) && short_page_id(path).is_none())
                {
                    return Err(page_refusal(path, 404));
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
            (_, path) if short_page_route(path) => Err(page_refusal(path, 404)),
            _ => Err(Reply::empty(404)),
        }
    }
    pub fn handle(&self, request: &Request, mounts: &Mounts) -> Reply {
        if let Some(id) = short_page_id(&request.path) {
            // `address` is the origin followed by this run's route prefix.
            let prefix = &self.address[self.origin.len()..];
            let mut reply = Reply::empty(302);
            reply
                .headers
                .push(("location".into(), format!("{prefix}/x/colab/p/{id}")));
            return reply;
        }
        match request.path.as_str() {
            "/sdk/remote-v1.js" => {
                // The path names the SDK interface version, not a build, so it
                // is not cached across upgrades.
                asset("text/javascript; charset=utf-8", SDK, None)
            }
            "/settings" => asset("text/html; charset=utf-8", SETTINGS, Some(PAGE_POLICY)),
            "/sdk/settings-v1.js" => asset("text/javascript; charset=utf-8", SETTINGS_SCRIPT, None),
            "/sdk/settings.css" => asset("text/css; charset=utf-8", SETTINGS_STYLE, None),
            "/sdk/pair.js" => asset("text/javascript; charset=utf-8", BOOTSTRAP, None),
            "/sdk/landing.js" => asset("text/javascript; charset=utf-8", ENTRY, None),
            "/sdk/pair-offer" => {
                let Some(descriptor) = self
                    .pairing
                    .as_ref()
                    .and_then(|pairing| pairing.descriptor(&self.address))
                else {
                    return Reply::empty(404);
                };
                asset("application/json", &descriptor.to_string(), None)
            }
            "/sdk/pages.css" => asset("text/css; charset=utf-8", STYLE, None),
            "/" => asset("text/html; charset=utf-8", LANDING, Some(PAGE_POLICY)),
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
        let (extension, mount) = mounts.extension_of(&path).unzip();
        let mut reply = Reply::empty(200);
        reply.body = json!({
            "machineId": self.machine_id,
            "windowId": self.window_id,
            "address": self.address,
            "extension": extension,
            "mount": mount,
        })
        .to_string()
        .into_bytes();
        reply
    }
}
fn short_page_route(path: &str) -> bool {
    path == "/p" || path.starts_with("/p/")
}
fn short_page_id(path: &str) -> Option<&str> {
    let id = path.strip_prefix("/p/")?;
    ((4..=64).contains(&id.len())
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-')))
    .then_some(id)
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

/// Only browser page routes receive HTML. SDK and protocol refusals stay JSON.
fn page_refusal(path: &str, status: u16) -> Reply {
    if path != "/"
        && path != "/settings"
        && path != "/pair"
        && !path.starts_with("/pair/")
        && !short_page_route(path)
    {
        return Reply::empty(status);
    }
    let mut reply = asset("text/html; charset=utf-8", ERROR, Some(PAGE_POLICY));
    reply.status = status;
    reply
}
