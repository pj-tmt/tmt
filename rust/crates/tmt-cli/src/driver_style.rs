//! A driver's display token, from its descriptor's hue: magenta is review,
//! cyan is link.

use tmt_cli_style::{Role, Token};
use tmt_core::driver::descriptor::{DriverDescriptor, Hue};

/// The token for a driver's address; unknown drivers render dimmed.
pub fn token(driver: &str) -> Token {
    token_in(&tmt_core::driver::ALL, driver)
}

pub fn token_in(drivers: &[&DriverDescriptor], driver: &str) -> Token {
    Token::Driver(
        drivers
            .iter()
            .find(|candidate| candidate.name.eq_ignore_ascii_case(driver))
            .and_then(|driver| match driver.hue {
                Hue::Magenta => Some(Role::Review),
                Hue::Cyan => Some(Role::Link),
                Hue::Neutral => None,
            }),
    )
}
