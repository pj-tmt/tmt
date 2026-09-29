//! Canonical, credential-free repository identifiers (`host[:port]/path`).

pub fn valid_repository_id(value: &str) -> bool {
    if value.is_empty()
        || value.len() > 2048
        || value.contains(['?', '#', '\\', '%', '@'])
        || value.ends_with('/')
        || value.chars().any(char::is_control)
    {
        return false;
    }
    let Some((host_port, path)) = value.split_once('/') else {
        return false;
    };
    if host_port.is_empty()
        || host_port != host_port.to_ascii_lowercase()
        || host_port.starts_with(['.', '-'])
        || host_port.ends_with(['.', '-'])
        || host_port.contains("..")
        || path
            .split('/')
            .any(|part| part.is_empty() || matches!(part, "." | ".."))
    {
        return false;
    }
    let (host, port) = match host_port.rsplit_once(':') {
        Some((host, port)) => (host, Some(port)),
        None => (host_port, None),
    };
    !host.is_empty()
        && host.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'-')
        })
        && port.is_none_or(|port| port.parse::<u16>().ok().is_some_and(|value| value > 0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repository_ids_are_canonical_credential_free_values() {
        for value in ["github.com/Org/Repo", "git.example:8443/Team/R.git"] {
            assert!(valid_repository_id(value));
        }
        for value in [
            "GitHub.com/Org/Repo",
            "user@github.com/Org/Repo",
            "github.com/Org/../Repo",
            "github.com/Org/%2e%2e/Repo",
            "github.com/Org/Repo?token=x",
            "/Org/Repo",
        ] {
            assert!(!valid_repository_id(value), "accepted {value}");
        }
    }
}
