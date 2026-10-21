//! The one TMT web app document (`shell.html`) at every TMT route, the pairing page, public
//! extension entries, page refusals and their stylesheet, outside the route prefix. `/sdk/mount` tells a page this
//! run's identity and which mounted extension its path belongs to. None of these
//! routes carries authority.
use crate::{
    http::{Head, Reply, Request},
    mount::Mounts,
    pairing::Pairing,
};
use base64::Engine;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::sync::Arc;

/// Built from `typescript/remote-client` by `pnpm build`; CI checks it is current.
const SDK: &str = include_str!("../assets/remote-v1.js");
const BOOTSTRAP: &str = include_str!("../assets/pair.js");
const PAGE: &str = include_str!("../assets/pair.html");
const ENTRY: &str = include_str!("../assets/landing.js");
const SHELL_ENTRY: &str = include_str!("../assets/shell.js");
/// The one TMT web app (#3937): its routes are modules in the SDK (`shell-app.ts`), whose page
/// markup is built in from the page files; Colab's module loads from Colab's own mount.
const SHELL: &str = include_str!("../assets/shell.html");
const ERROR: &str = include_str!("../assets/error.html");
const SETTINGS_SCRIPT: &str = include_str!("../assets/settings-v1.js");
const SETTINGS_STYLE: &str = include_str!("../assets/settings.css");
// Checked shared presentation plus Remote-owned layout; no Cargo-time generator.
const STYLE: &str = concat!(
    include_str!("../../../../../design/browser-ui/generated/static.css"),
    "\n",
    include_str!("../assets/pages.css"),
    "\n",
    include_str!("../assets/tmt-settings.css"),
);
/// The page runs only the same-origin SDK module and talks only to this door.
const PAGE_POLICY: &str = "default-src 'none'; script-src 'self'; style-src 'self'; img-src data:; connect-src 'self'; \
    base-uri 'none'; form-action 'none'; frame-ancestors 'none'";
/// The app document also hosts Colab, so it carries Colab's own document policy
/// (`tmt-colab` `browser_policy::POLICY`): same-origin scripts, styles, images, fonts, workers and
/// frames (Colab's renderer stays its own sandboxed opaque frame with its own policy).
pub(crate) const SHELL_POLICY: &str = "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self' data:; font-src 'self'; connect-src 'self'; worker-src 'self'; frame-src 'self'; base-uri 'none'; form-action 'none'; object-src 'none'; frame-ancestors 'none'";
/// The app's routes: an explicit list, never a catch-all. `/sdk`, `/x`, `/read` and `/pair`
/// keep their own handling.
pub(crate) fn app_route(path: &str) -> bool {
    matches!(
        path,
        "/" | "/settings"
            | "/settings/"
            | "/remote"
            | "/remote/"
            | "/remote/settings"
            | "/ops"
            | "/ops/"
            | "/digest"
            | "/digest/"
            | "/colab"
            | "/colab/"
    ) || path.strip_prefix("/colab/").is_some_and(|rest| {
        rest.len() <= 160
            && rest.split('/').all(|segment| {
                !segment.is_empty()
                    && segment
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
            })
    })
}
/// The owner's short page links (`/p/<id>`) open that page in the app at `/colab/p/<id>`.
pub(crate) fn short_page(path: &str) -> Option<&str> {
    path.strip_prefix("/p/").filter(|id| {
        (4..=64).contains(&id.len())
            && id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
    })
}
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
        let path = path.split('?').next().unwrap_or(path);
        app_route(path)
            || path == "/remote/pair"
            || path == "/p"
            || path.starts_with("/p/")
            || legacy_pair(path)
            || path.starts_with("/remote/pair/")
            || path.starts_with("/pair/")
            || Mounts::short_route(path)
            || path.starts_with("/sdk/")
    }
    pub fn admit(&self, head: &Head<'_>) -> Result<usize, Reply> {
        if head.upgrade {
            return Err(if Mounts::short_route(head.path) {
                page_refusal(head.path, 404)
            } else {
                Reply::empty(404)
            });
        }
        if legacy_pair(head.path) {
            if !matches!(head.method, "GET" | "HEAD") {
                return Err(Reply::empty(404));
            }
            if head.origin.is_some_and(|o| o != self.origin) {
                return Err(page_refusal(head.path, 403));
            }
            return Ok(0);
        }
        let bare = head.path.split('?').next().unwrap_or(head.path);
        match (head.method, head.path) {
            ("GET" | "HEAD", path) if app_route(bare) || short_page(bare).is_some() => {
                // Navigation omits Origin; a cross-origin load is refused.
                if head.origin.is_some_and(|o| o != self.origin) {
                    return Err(page_refusal(path, 403));
                }
                Ok(0)
            }
            ("GET", path)
                if matches!(
                    path,
                    "/remote/pair"
                        | "/sdk/pair.js"
                        | "/sdk/landing.js"
                        | "/sdk/shell.js"
                        | "/sdk/pair-offer"
                        | "/sdk/remote-v1.js"
                        | "/sdk/pages.css"
                        | "/sdk/settings-v1.js"
                        | "/sdk/settings.css"
                ) || path.starts_with("/pair/")
                    || path.starts_with("/remote/pair/")
                    || Mounts::short_route(path) =>
            {
                // Navigation omits Origin; a cross-origin load is refused.
                if head.origin.is_some_and(|o| o != self.origin) {
                    return Err(page_refusal(path, 403));
                }
                if path.starts_with("/pair/")
                    || path.starts_with("/remote/pair/")
                    || (Mounts::short_route(path) && !Mounts::valid_short_route(path))
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
            (_, path) if Mounts::short_route(path) || bare == "/p" || bare.starts_with("/p/") => {
                Err(page_refusal(path, 404))
            }
            _ => Err(Reply::empty(404)),
        }
    }
    pub fn handle(&self, request: &Request, mounts: &Mounts) -> Reply {
        if legacy_pair(&request.path) {
            if request
                .headers
                .iter()
                .any(|(name, value)| name == "sec-fetch-site" && value == "cross-site")
            {
                return page_refusal(&request.path, 403);
            }
            let mut reply = asset("text/html; charset=utf-8", "", Some(PAGE_POLICY));
            reply.status = 308;
            reply
                .headers
                .push(("location".into(), "/remote/pair".into()));
            return reply;
        }
        let (path, query) = request
            .path
            .split_once('?')
            .map_or((request.path.as_str(), ""), |(path, query)| (path, query));
        if app_route(path) {
            return asset("text/html; charset=utf-8", SHELL, Some(SHELL_POLICY));
        }
        if let Some(id) = short_page(path) {
            // The fragment (a reading, anchor or thread) is the browser's and follows the redirect.
            let mut reply = asset("text/html; charset=utf-8", "", Some(PAGE_POLICY));
            reply.status = 308;
            let location = if query.is_empty() {
                format!("/colab/p/{id}")
            } else {
                format!("/colab/p/{id}?{query}")
            };
            reply.headers.push(("location".into(), location));
            return reply;
        }
        match request.path.as_str() {
            "/sdk/remote-v1.js" => {
                // The path names the SDK interface version, not a build, so it
                // is not cached across upgrades.
                asset("text/javascript; charset=utf-8", SDK, None)
            }
            "/sdk/settings-v1.js" => asset("text/javascript; charset=utf-8", SETTINGS_SCRIPT, None),
            "/sdk/settings.css" => asset("text/css; charset=utf-8", SETTINGS_STYLE, None),
            "/sdk/pair.js" => asset("text/javascript; charset=utf-8", BOOTSTRAP, None),
            "/sdk/landing.js" => asset("text/javascript; charset=utf-8", ENTRY, None),
            "/sdk/shell.js" => asset("text/javascript; charset=utf-8", SHELL_ENTRY, None),
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
            "/sdk/mount" => self.mount(&request.body, mounts),
            _ => {
                // The exact inline bytes erase the fragment before any subresource is parsed.
                let body = PAGE.replace("__TMT_PAIR_BOOTSTRAP__", BOOTSTRAP);
                let inline = body
                    .split_once("<script>")
                    .expect("pair bootstrap")
                    .1
                    .split_once("</script>")
                    .expect("pair bootstrap end")
                    .0;
                let hash = base64::engine::general_purpose::STANDARD
                    .encode(Sha256::digest(inline.as_bytes()));
                let policy = PAGE_POLICY.replace(
                    "script-src 'self';",
                    &format!("script-src 'self' 'sha256-{hash}';"),
                );
                asset("text/html; charset=utf-8", &body, Some(&policy))
            }
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
fn legacy_pair(path: &str) -> bool {
    path == "/pair" || path.starts_with("/pair?")
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
pub(crate) fn page_refusal(path: &str, status: u16) -> Reply {
    if !app_route(path)
        && path != "/remote/pair"
        && !path.starts_with("/p/")
        && !path.starts_with("/remote/pair/")
        && !legacy_pair(path)
        && !path.starts_with("/pair/")
        && !Mounts::short_route(path)
    {
        return Reply::empty(status);
    }
    let mut reply = asset("text/html; charset=utf-8", ERROR, Some(PAGE_POLICY));
    reply.status = status;
    reply
}
