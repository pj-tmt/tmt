//! One bounded, anonymous GET of the public price file: no token, cookies,
//! redirects, retries or GitHub API. Only the request address, the URL and a
//! User-Agent reach the host.

use super::MAX_FILE_BYTES;
use std::time::Duration;

pub(super) const URL: &str = "https://raw.githubusercontent.com/pj-tmt/tmt/main/extensions/tmt-digest/pricing/pricing-v1.json";
/// Selects the address; `off` (or anything that is not an admitted address)
/// turns the fetch off.
const ENV: &str = "TMT_DIGEST_PRICING_URL";
/// DNS to the last body byte.
const DEADLINE: Duration = Duration::from_secs(2);

pub struct Target {
    url: String,
    loopback: bool,
}

/// The address to fetch, or `None` when the user turned the fetch off.
pub fn target() -> Option<Target> {
    target_from(std::env::var(ENV))
}

/// Unset means the public address; anything else must be admitted.
pub(super) fn target_from(value: Result<String, std::env::VarError>) -> Option<Target> {
    match value {
        Err(std::env::VarError::NotPresent) => Some(Target {
            url: URL.to_owned(),
            loopback: false,
        }),
        Err(_) => None,
        Ok(value) => admit(&value),
    }
}

/// `https://…` anywhere, or `http://` to this machine only (for tests).
pub(super) fn admit(value: &str) -> Option<Target> {
    if value.len() > 512 || !value.bytes().all(|c| (0x21..=0x7e).contains(&c)) {
        return None;
    }
    if value.starts_with("https://") {
        return Some(Target {
            url: value.to_owned(),
            loopback: false,
        });
    }
    let rest = value.strip_prefix("http://")?;
    let host = if rest.starts_with('[') {
        &rest[..rest.find(']')? + 1]
    } else {
        &rest[..rest.find([':', '/']).unwrap_or(rest.len())]
    };
    let end = rest.as_bytes().get(host.len());
    (matches!(host, "127.0.0.1" | "localhost" | "[::1]") && matches!(end, None | Some(b':' | b'/')))
        .then(|| Target {
            url: value.to_owned(),
            loopback: true,
        })
}

/// The body of a 200 answer within the size bound, or nothing.
pub fn get(target: &Target) -> Option<Vec<u8>> {
    let mut config = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .max_redirects(0)
        .max_idle_connections(0)
        .https_only(!target.loopback)
        .timeout_global(Some(DEADLINE));
    if target.loopback {
        // A local test address must never go through an ambient proxy.
        config = config.proxy(None);
    }
    let agent: ureq::Agent = config.build().into();
    let mut response = agent
        .get(&target.url)
        .header(
            "User-Agent",
            concat!("tmt-digest/", env!("CARGO_PKG_VERSION")),
        )
        .call()
        .ok()?;
    if response.status().as_u16() != 200 {
        return None;
    }
    response
        .body_mut()
        .with_config()
        .limit(MAX_FILE_BYTES as u64)
        .read_to_vec()
        .ok()
}

#[cfg(test)]
impl Target {
    /// A loopback target for the fake server.
    pub fn local(url: String) -> Self {
        Self {
            url,
            loopback: true,
        }
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn is_loopback(&self) -> bool {
        self.loopback
    }
}
