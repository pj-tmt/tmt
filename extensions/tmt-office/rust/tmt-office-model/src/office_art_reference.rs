//! Pure grammar for immutable Office art references shared by core consumers and adapters.

pub const KEY_LIMIT: usize = 32;

pub fn parse_office_art_reference(value: &str) -> Option<(&str, &str)> {
    let (digest, key) = value.split_once('/')?;
    let hex = digest.strip_prefix("sha256:")?;
    (hex.len() == 64
        && hex
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        && valid_office_art_key(key))
    .then_some((digest, key))
}

pub fn valid_office_art_key(value: &str) -> bool {
    (1..=KEY_LIMIT).contains(&value.len())
        && value.bytes().enumerate().all(|(index, byte)| match byte {
            b'a'..=b'z' => true,
            b'0'..=b'9' | b'-' => index > 0,
            _ => false,
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn immutable_reference_grammar_is_exact() {
        let digest = format!("sha256:{}", "0".repeat(64));
        assert_eq!(
            parse_office_art_reference(&format!("{digest}/signal-bot")),
            Some((digest.as_str(), "signal-bot"))
        );
        for invalid in [
            format!("sha256:{}/Signal", "0".repeat(64)),
            format!("sha256:{}/-signal", "0".repeat(64)),
            format!("sha256:{}/signal/extra", "0".repeat(64)),
            format!("sha256:{}/signal", "A".repeat(64)),
            format!("sha256:{}/{}", "0".repeat(64), "a".repeat(KEY_LIMIT + 1)),
        ] {
            assert_eq!(parse_office_art_reference(&invalid), None, "{invalid}");
        }
    }
}
