//! The person's identity (#513): which name it takes, and what changes when an agent already
//! holds that name. Storage, the OS login lookup and the API belong to adapters.

use crate::names::{NAME_LIMIT, NameError, validate_name};
use crate::settings::UserName;

/// Keeps the person's identity apart from an agent identity that holds the login name.
const AGENT_NAME_SUFFIX: &str = "-user";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserNameError {
    /// No `user.name` setting and no usable OS login name.
    Unavailable,
    /// The OS login name is not a name an identity may take.
    Invalid(NameError),
}

/// The person's name: the `user.name` setting, else the OS login name.
pub fn wanted_name(
    setting: Option<&UserName>,
    login: Option<&str>,
) -> Result<String, UserNameError> {
    if let Some(name) = setting {
        return Ok(name.as_str().to_owned());
    }
    let login = login
        .map(str::trim)
        .filter(|login| !login.is_empty())
        .ok_or(UserNameError::Unavailable)?;
    validate_name(login)
        .map(|name| name.display_name().to_owned())
        .map_err(UserNameError::Invalid)
}

/// The name to use when an agent identity already holds `name`: `<name>-user`, cut to fit.
pub fn beside_agent(name: &str) -> String {
    let room = NAME_LIMIT - AGENT_NAME_SUFFIX.chars().count();
    let base: String = name.chars().take(room).collect();
    format!("{base}{AGENT_NAME_SUFFIX}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_setting_wins_over_the_login_name() {
        let setting = UserName::parse("Ben").unwrap();
        assert_eq!(wanted_name(Some(&setting), Some("root")).unwrap(), "Ben");
        assert_eq!(wanted_name(Some(&setting), None).unwrap(), "Ben");
    }

    #[test]
    fn the_login_name_is_the_default_after_trimming() {
        assert_eq!(wanted_name(None, Some(" benhsieh ")).unwrap(), "benhsieh");
    }

    #[test]
    fn no_setting_and_no_usable_login_is_unavailable() {
        for login in [None, Some(""), Some("  ")] {
            assert_eq!(wanted_name(None, login), Err(UserNameError::Unavailable));
        }
    }

    #[test]
    fn a_login_that_no_identity_may_take_is_invalid_not_unavailable() {
        assert_eq!(
            wanted_name(None, Some(&"a".repeat(NAME_LIMIT + 1))),
            Err(UserNameError::Invalid(NameError::TooLong))
        );
        assert_eq!(
            wanted_name(None, Some("bad\nname")),
            Err(UserNameError::Invalid(NameError::EmptyOrControl))
        );
    }

    #[test]
    fn beside_an_agent_the_name_gets_a_suffix_that_still_fits() {
        assert_eq!(beside_agent("ben"), "ben-user");
        let long = "x".repeat(NAME_LIMIT);
        let beside = beside_agent(&long);
        assert_eq!(beside.chars().count(), NAME_LIMIT);
        assert!(beside.ends_with("-user"));
        assert!(validate_name(&beside).is_ok());
    }
}
