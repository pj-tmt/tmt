//! Closed emulator address substitution, shared by the explicit proof feature
//! and the native emulator scenario. It never falls back to official hosts.
use super::{AuthError, AuthErrorKind, Cancellation, Http, Request, Response};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};

pub(super) struct EmulatorRoute {
    project: String,
    auth: SocketAddr,
    firestore: SocketAddr,
}
fn invalid() -> AuthError {
    AuthError::new(AuthErrorKind::InputInvalid)
}
fn loopback(value: &str) -> Result<SocketAddr, AuthError> {
    let address = if let Some(port) = value.strip_prefix("localhost:") {
        SocketAddr::new(
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            port.parse().map_err(|_| invalid())?,
        )
    } else {
        value.parse::<SocketAddr>().map_err(|_| invalid())?
    };
    if address.port() == 0 || !address.ip().is_loopback() {
        return Err(invalid());
    }
    Ok(address)
}
fn document_path(path: &str) -> bool {
    path.strip_prefix('/').is_some_and(|path| {
        !path.is_empty()
            && path.split('/').all(|part| {
                !part.is_empty()
                    && part.bytes().all(|byte| {
                        byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'+')
                    })
            })
    })
}
impl EmulatorRoute {
    pub(super) fn new(project: &str, auth: &str, firestore: &str) -> Result<Self, AuthError> {
        if !crate::deploy_run::project_ok(project)
            || !project
                .strip_prefix("demo-")
                .is_some_and(|suffix| !suffix.is_empty())
        {
            return Err(invalid());
        }
        Ok(Self {
            project: project.into(),
            auth: loopback(auth)?,
            firestore: loopback(firestore)?,
        })
    }
    pub(super) fn url(&self, url: &str) -> Result<String, AuthError> {
        for (host, operation) in [
            ("identitytoolkit.googleapis.com", "v1/accounts:signUp?key="),
            ("identitytoolkit.googleapis.com", "v1/accounts:delete?key="),
            ("securetoken.googleapis.com", "v1/token?key="),
        ] {
            let prefix = format!("https://{host}/{operation}");
            if let Some(key) = url.strip_prefix(&prefix)
                && !key.is_empty()
                && key
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
            {
                return Ok(format!("http://{}/{host}/{operation}{key}", self.auth));
            }
        }
        let prefix = format!(
            "https://firestore.googleapis.com/v1/projects/{}/databases/(default)/documents",
            self.project
        );
        if let Some(path) = url.strip_prefix(&prefix) {
            let allowed = matches!(path, ":commit" | ":runQuery" | ":runAggregationQuery")
                || document_path(path)
                || [":runQuery", ":runAggregationQuery"]
                    .iter()
                    .any(|operation| path.strip_suffix(operation).is_some_and(document_path));
            if allowed {
                return Ok(format!(
                    "http://{}/v1/projects/{}/databases/(default)/documents{path}",
                    self.firestore, self.project
                ));
            }
        }
        Err(invalid())
    }
    pub(super) fn send(
        &self,
        real: &dyn Http,
        request: &Request,
        deadline: u64,
        cancel: &dyn Cancellation,
    ) -> Result<Response, AuthError> {
        let copied = request.with_emulator_url(self.url(request.url())?);
        let reply = real.send(&copied, deadline, cancel)?;
        if (300..400).contains(&reply.status) {
            return Err(AuthError::new(AuthErrorKind::InvalidReply));
        }
        Ok(reply)
    }
}
