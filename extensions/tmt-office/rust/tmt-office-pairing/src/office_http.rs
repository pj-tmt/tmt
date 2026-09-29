//! Shared bounded Office transport. Callers select fixed validated endpoints.

use crate::office_deployment::DeploymentMode;
use std::{
    io,
    time::{Duration, Instant},
};
use ureq::{
    Agent, Body,
    http::Response,
    tls::{RootCerts, TlsConfig},
};

pub(crate) fn agent(mode: DeploymentMode, deadline: Instant) -> io::Result<Agent> {
    let remaining = deadline
        .checked_duration_since(Instant::now())
        .filter(|remaining| !remaining.is_zero())
        .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "Office operation timed out."))?
        .min(Duration::from_secs(10));
    Ok(Agent::new_with_config(
        Agent::config_builder()
            .https_only(mode == DeploymentMode::Cloud)
            .http_status_as_error(false)
            .max_redirects(0)
            .max_response_header_size(16 * 1024)
            .timeout_global(Some(remaining))
            .tls_config(
                TlsConfig::builder()
                    .root_certs(RootCerts::PlatformVerifier)
                    .build(),
            )
            .build(),
    ))
}

pub(crate) fn json_body(response: &mut Response<Body>, limit: usize) -> io::Result<Vec<u8>> {
    if !response
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value
                .split(';')
                .next()
                .is_some_and(|mime| mime.trim().eq_ignore_ascii_case("application/json"))
        })
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Invalid Office response.",
        ));
    }
    let bytes = response
        .body_mut()
        .with_config()
        .limit((limit + 1) as u64)
        .read_to_vec()
        .map_err(failure)?;
    if bytes.len() > limit {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Office response exceeds its bound.",
        ));
    }
    Ok(bytes)
}

pub(crate) fn failure(error: ureq::Error) -> io::Error {
    let kind = match error {
        ureq::Error::Timeout(_) => io::ErrorKind::TimedOut,
        ureq::Error::BodyExceedsLimit(_) => io::ErrorKind::InvalidData,
        _ => io::ErrorKind::Other,
    };
    io::Error::new(kind, "Office request could not be confirmed.")
}

#[cfg(test)]
#[path = "office_http_tests.rs"]
pub(crate) mod tests;
