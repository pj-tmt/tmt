//! Bounded HTTPS acquisition for the canonical native release service.

use std::{
    cell::Cell,
    io,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use ureq::{
    Agent,
    http::{StatusCode, Uri},
    tls::{RootCerts, TlsConfig},
};

const MAX_REDIRECTS: usize = 3;
const MAX_RESPONSE_HEADER: usize = 64 * 1024;
const USER_AGENT: &str = "tmt";

const ALLOWED_HOSTS: &[&str] = &[
    "api.github.com",
    "github.com",
    "release-assets.githubusercontent.com",
    "objects.githubusercontent.com",
];

/// Acquisition bytes and the optional pagination header. Discovery, not HTTPS,
/// owns whether and where another metadata page may be requested.
#[derive(Debug)]
pub(crate) struct Response {
    pub body: Vec<u8>,
    pub link: Option<String>,
}

#[cfg(test)]
impl From<Vec<u8>> for Response {
    fn from(body: Vec<u8>) -> Self {
        Self { body, link: None }
    }
}

/// The bounded synchronous HTTPS client used by native release acquisition.
pub(crate) struct Https {
    agent: Agent,
    token: Option<String>,
    waited: Cell<bool>,
    #[cfg(test)]
    wait: fn(Duration),
    #[cfg(test)]
    now: fn() -> SystemTime,
    #[cfg(test)]
    allow_loopback: bool,
}

impl Https {
    pub(crate) fn new() -> Self {
        let config = Agent::config_builder()
            .https_only(true)
            .max_redirects(0)
            .max_response_header_size(MAX_RESPONSE_HEADER)
            .user_agent(USER_AGENT)
            .tls_config(
                TlsConfig::builder()
                    .root_certs(RootCerts::PlatformVerifier)
                    .build(),
            )
            .build();

        Self {
            agent: Agent::new_with_config(config),
            token: std::env::var("GITHUB_TOKEN")
                .ok()
                .filter(|token| !token.is_empty()),
            waited: Cell::new(false),
            #[cfg(test)]
            wait: std::thread::sleep,
            #[cfg(test)]
            now: SystemTime::now,
            #[cfg(test)]
            allow_loopback: false,
        }
    }

    #[cfg(test)]
    pub(super) fn with_test_agent(agent: Agent) -> Self {
        Self {
            agent,
            token: None,
            waited: Cell::new(false),
            wait: std::thread::sleep,
            now: SystemTime::now,
            allow_loopback: true,
        }
    }

    pub(crate) fn get(
        &self,
        url: &str,
        accept: &str,
        maximum: usize,
        deadline: Instant,
    ) -> io::Result<Response> {
        #[cfg(test)]
        let mut current = validate_url_with_loopback(url, self.allow_loopback)?;
        #[cfg(not(test))]
        let mut current = validate_url(url)?;
        let body_limit = bounded_body_limit(maximum)?;

        let mut redirects = 0;
        loop {
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .ok_or_else(timeout_error)?;
            if remaining == Duration::ZERO {
                return Err(timeout_error());
            }

            let mut request = self.agent.get(current.clone()).header("Accept", accept);
            if is_api(&current)
                && let Some(token) = &self.token
            {
                request = request.header("Authorization", format!("Bearer {token}"));
            }
            let mut response = request
                .config()
                .http_status_as_error(false)
                .timeout_global(Some(remaining))
                .build()
                .call()
                .map_err(map_ureq_error)?;

            #[cfg(test)]
            let now = (self.now)();
            #[cfg(not(test))]
            let now = SystemTime::now();
            let status = response.status();
            if is_api(&current)
                && matches!(
                    status,
                    StatusCode::FORBIDDEN | StatusCode::TOO_MANY_REQUESTS
                )
                && let Some(limit) = RateLimit::from_headers(response.headers(), now)
            {
                let jitter =
                    Duration::from_millis(50 + u64::from(uuid::Uuid::new_v4().as_bytes()[0]));
                let wait = limit.delay.and_then(|delay| delay.checked_add(jitter));
                let remaining = deadline.checked_duration_since(Instant::now());
                let reason = if self.waited.get() {
                    Some("the single retry was exhausted")
                } else if wait
                    .zip(remaining)
                    .is_none_or(|(wait, remaining)| wait >= remaining)
                {
                    Some("the required wait exceeds the remaining deadline")
                } else {
                    None
                };
                if let Some(reason) = reason {
                    return Err(limit.error(reason));
                }
                self.waited.set(true);
                // Release the response before waiting. The caller's absolute
                // deadline covers every request, redirect and this one wait.
                drop(response);
                #[cfg(test)]
                (self.wait)(wait.expect("checked wait"));
                #[cfg(not(test))]
                std::thread::sleep(wait.expect("checked wait"));
                continue;
            }
            if status.is_redirection() {
                if redirects == MAX_REDIRECTS {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "too many HTTPS redirects",
                    ));
                }
                let location = response
                    .headers()
                    .get("location")
                    .ok_or_else(|| invalid_response("redirect has no location"))?
                    .to_str()
                    .map_err(|_| invalid_response("redirect location is invalid"))?;
                #[cfg(test)]
                let next = validate_url_with_loopback(location, self.allow_loopback);
                #[cfg(not(test))]
                let next = validate_url(location);
                current =
                    next.map_err(|_| invalid_response("redirect location is not approved"))?;
                redirects += 1;
                continue;
            }

            if status == StatusCode::NOT_FOUND {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    "HTTPS resource was not found",
                ));
            }
            if !status.is_success() {
                return Err(io::Error::other(format!(
                    "HTTPS request failed: http status: {}",
                    status.as_u16()
                )));
            }

            if let Some(length) = response.headers().get("content-length") {
                let length = length
                    .to_str()
                    .ok()
                    .and_then(|value| value.parse::<u64>().ok())
                    .ok_or_else(|| invalid_response("HTTPS response length is invalid"))?;
                if length > maximum as u64 {
                    return Err(invalid_response("HTTPS response exceeds its size limit"));
                }
            }

            let links = response
                .headers()
                .get_all("link")
                .iter()
                .map(|value| {
                    value
                        .to_str()
                        .map(str::to_owned)
                        .map_err(|_| invalid_response("invalid pagination header"))
                })
                .collect::<io::Result<Vec<_>>>()?;
            let link = (!links.is_empty()).then(|| links.join(","));
            let bytes = response
                .body_mut()
                .with_config()
                .limit(body_limit)
                .read_to_vec()
                .map_err(map_ureq_error)?;
            if bytes.len() > maximum {
                return Err(invalid_response("HTTPS response exceeds its size limit"));
            }
            return Ok(Response { body: bytes, link });
        }
    }
}

fn is_api(uri: &Uri) -> bool {
    uri.host()
        .is_some_and(|host| host.eq_ignore_ascii_case("api.github.com"))
}

struct RateLimit {
    delay: Option<Duration>,
    retry_at: Option<u64>,
}

impl RateLimit {
    fn from_headers(headers: &ureq::http::HeaderMap, now: SystemTime) -> Option<Self> {
        let number = |name| headers.get(name)?.to_str().ok()?.parse::<u64>().ok();
        let primary = headers
            .get("x-ratelimit-remaining")
            .is_some_and(|value| value == "0")
            && headers.contains_key("x-ratelimit-reset");
        if !primary && !headers.contains_key("retry-after") {
            return None;
        }
        let now_epoch = now.duration_since(UNIX_EPOCH).ok();
        let reset = primary.then(|| number("x-ratelimit-reset")).flatten();
        let reset_delay = reset
            .zip(now_epoch)
            .map(|(reset, now)| Duration::from_secs(reset).saturating_sub(now));
        let retry = headers.contains_key("retry-after");
        let retry_delay = retry
            .then(|| number("retry-after").map(Duration::from_secs))
            .flatten();
        // A malformed applicable constraint must never permit an early retry.
        let delay = if (primary && reset_delay.is_none()) || (retry && retry_delay.is_none()) {
            None
        } else {
            Some(
                reset_delay
                    .unwrap_or_default()
                    .max(retry_delay.unwrap_or_default()),
            )
        };
        let retry_at = reset
            .into_iter()
            .chain(retry_delay.zip(now_epoch).and_then(|(delay, now)| {
                now.as_secs()
                    .checked_add(delay.as_secs())?
                    .checked_add(u64::from(now.subsec_nanos() != 0))
            }))
            .max();
        Some(Self { delay, retry_at })
    }

    fn error(&self, reason: &str) -> io::Error {
        let reset = self
            .retry_at
            .and_then(|epoch| {
                time::OffsetDateTime::from_unix_timestamp(i64::try_from(epoch).ok()?)
                    .ok()
                    .map(|date| format!("{date} (UTC epoch {epoch})"))
            })
            .unwrap_or_else(|| "unavailable (missing or invalid timing header)".to_owned());
        io::Error::other(format!(
            "GitHub API rate limit: reset/earliest retry time {reset}; {reason}. Retry later or optionally set GITHUB_TOKEN."
        ))
    }
}

fn validate_url(value: &str) -> io::Result<Uri> {
    validate_url_inner(value, false)
}

#[cfg(test)]
fn validate_url_with_loopback(value: &str, allow_loopback: bool) -> io::Result<Uri> {
    validate_url_inner(value, allow_loopback)
}

fn validate_url_inner(value: &str, allow_loopback: bool) -> io::Result<Uri> {
    if value.contains('#') {
        return Err(invalid_url());
    }
    let uri = value.parse::<Uri>().map_err(|_| invalid_url())?;
    if uri.scheme_str() != Some("https") {
        return Err(invalid_url());
    }
    let authority = uri.authority().ok_or_else(invalid_url)?;
    if authority.as_str().contains('@') {
        return Err(invalid_url());
    }
    let loopback = allow_loopback && matches!(uri.host(), Some("127.0.0.1" | "localhost"));
    if !loopback && uri.port_u16().is_some_and(|port| port != 443) {
        return Err(invalid_url());
    }
    let host = uri.host().ok_or_else(invalid_url)?;
    if !loopback
        && !ALLOWED_HOSTS
            .iter()
            .any(|allowed| host.eq_ignore_ascii_case(allowed))
    {
        return Err(invalid_url());
    }
    Ok(uri)
}

fn bounded_body_limit(maximum: usize) -> io::Result<u64> {
    maximum
        .checked_add(1)
        .and_then(|limit| u64::try_from(limit).ok())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "response limit is too large"))
}

fn map_ureq_error(error: ureq::Error) -> io::Error {
    match error {
        ureq::Error::StatusCode(404) => {
            io::Error::new(io::ErrorKind::NotFound, "HTTPS resource was not found")
        }
        ureq::Error::Timeout(_) => timeout_error(),
        ureq::Error::Io(error) if error.kind() == io::ErrorKind::TimedOut => timeout_error(),
        ureq::Error::BodyExceedsLimit(_) => {
            invalid_response("HTTPS response exceeds its size limit")
        }
        ureq::Error::Io(error) => {
            io::Error::other(format!("HTTPS I/O failed ({:?}): {error}", error.kind()))
        }
        // URI/proxy errors can contain credentials from the environment. Keep
        // their diagnostic class, never the rejected URL or proxy response.
        ureq::Error::BadUri(_) | ureq::Error::RequireHttpsOnly(_) => {
            io::Error::other("HTTPS request failed: invalid HTTPS URI")
        }
        ureq::Error::ConnectProxyFailed(_) => {
            io::Error::other("HTTPS request failed: proxy connection refused")
        }
        error => io::Error::other(format!("HTTPS request failed: {error}")),
    }
}

fn timeout_error() -> io::Error {
    io::Error::new(io::ErrorKind::TimedOut, "HTTPS request timed out")
}

fn invalid_url() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, "HTTPS URL is not approved")
}

fn invalid_response(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_canonical_https_hosts() {
        for host in ALLOWED_HOSTS {
            assert!(validate_url(&format!("https://{host}/release")).is_ok());
        }
        assert!(validate_url("http://api.github.com/release").is_err());
        assert!(validate_url("https://github.com:8443/release").is_err());
        assert!(validate_url("https://github.com.evil.example/release").is_err());
        assert!(validate_url("https://user@github.com/release").is_err());
        assert!(validate_url("https://github.com/release#fragment").is_err());
    }

    #[test]
    fn redirects_must_be_absolute_approved_https_urls() {
        assert!(validate_url("/releases/latest").is_err());
        assert!(validate_url("https://objects.githubusercontent.com/archive").is_ok());
        assert!(validate_url("https://example.com/archive").is_err());
    }

    #[test]
    fn body_limit_reserves_one_byte_for_oversize_detection() {
        assert_eq!(bounded_body_limit(3).unwrap(), 4);
        assert!(bounded_body_limit(usize::MAX).is_err());
    }
}

#[cfg(test)]
#[path = "release_http_tests.rs"]
mod release_http_tests;
