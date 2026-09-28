//! A squad is the core room `squad-<name>`; member fields are the identity
//! metadata keys `squad.<name>.<field>`. There is no other squad state.

use crate::core::SquadError;

pub fn valid_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    (1..=24).contains(&bytes.len())
        && bytes[0].is_ascii_lowercase()
        && bytes[1..]
            .iter()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-')
}

/// The core room that is a squad.
pub fn room_name(name: &str) -> String {
    format!("squad-{name}")
}

pub fn name_invalid(name: &str) -> SquadError {
    SquadError::new(
        "SQUAD_NAME_INVALID",
        format!("Squad name '{name}' must match [a-z][a-z0-9-]{{0,23}}."),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_fit_core_metadata_keys_and_room_names() {
        for name in ["a", "product", "pr-queue-2", &"x".repeat(24)] {
            assert!(valid_name(name), "{name}");
        }
        for name in ["", "Product", "1a", "a_b", "a.b", "-a", &"x".repeat(25)] {
            assert!(!valid_name(name), "{name}");
        }
        assert_eq!(room_name("product"), "squad-product");
    }
}
