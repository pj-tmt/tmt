//! The OS login name behind the default `user.name` (#513).

/// The login name of the user running this process, from the password database (not `$USER`,
/// which any shell can change), or none when the account has no usable entry.
pub fn login_name() -> Option<String> {
    nix::unistd::User::from_uid(nix::unistd::getuid())
        .ok()
        .flatten()
        .map(|user| user.name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_login_name_comes_from_the_password_database_not_the_environment() {
        let expected = nix::unistd::User::from_uid(nix::unistd::getuid())
            .unwrap()
            .map(|user| user.name);
        assert_eq!(login_name(), expected);
        if let Some(name) = login_name() {
            assert!(!name.trim().is_empty());
        }
    }
}
