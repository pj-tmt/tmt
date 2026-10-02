//! Bounded HTTPS acquisition for the canonical native release service.

use std::{
    io,
    time::{Duration, Instant},
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

/// The bounded synchronous HTTPS client used by native release acquisition.
pub(crate) struct Https {
    agent: Agent,
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
            #[cfg(test)]
            allow_loopback: false,
        }
    }

    #[cfg(test)]
    pub(super) fn with_test_agent(agent: Agent) -> Self {
        Self {
            agent,
            allow_loopback: true,
        }
    }

    pub(crate) fn get(
        &self,
        url: &str,
        accept: &str,
        maximum: usize,
        deadline: Instant,
    ) -> io::Result<Vec<u8>> {
        #[cfg(test)]
        let mut current = validate_url_with_loopback(url, self.allow_loopback)?;
        #[cfg(not(test))]
        let mut current = validate_url(url)?;
        let body_limit = bounded_body_limit(maximum)?;

        for redirects in 0..=MAX_REDIRECTS {
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .ok_or_else(timeout_error)?;
            if remaining == Duration::ZERO {
                return Err(timeout_error());
            }

            let mut response = self
                .agent
                .get(current.clone())
                .header("Accept", accept)
                .config()
                .timeout_global(Some(remaining))
                .build()
                .call()
                .map_err(map_ureq_error)?;

            let status = response.status();
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
                continue;
            }

            if status == StatusCode::NOT_FOUND {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    "HTTPS resource was not found",
                ));
            }
            if !status.is_success() {
                return Err(invalid_response("unexpected HTTPS response status"));
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

            let bytes = response
                .body_mut()
                .with_config()
                .limit(body_limit)
                .read_to_vec()
                .map_err(map_ureq_error)?;
            if bytes.len() > maximum {
                return Err(invalid_response("HTTPS response exceeds its size limit"));
            }
            return Ok(bytes);
        }

        unreachable!("redirect loop returns before exhausting its bounded range")
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
