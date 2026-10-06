//! Strict bounded metadata parsing for the PR acquisition trust boundary.

use serde::{
    Deserialize, Deserializer,
    de::{DeserializeOwned, MapAccess, Visitor},
};
use serde_json::value::RawValue;
use std::{collections::BTreeSet, fmt, io};

struct Fields<'a>(Vec<&'a RawValue>);

impl<'de> Deserialize<'de> for Fields<'de> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Object;
        impl<'de> Visitor<'de> for Object {
            type Value = Fields<'de>;
            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("an object without duplicate keys")
            }
            fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Self::Value, M::Error> {
                let mut keys = BTreeSet::new();
                let mut values = Vec::new();
                while let Some((key, value)) = map.next_entry::<String, &'de RawValue>()? {
                    if key == "$serde_json::private::Number" || !keys.insert(key) {
                        return Err(serde::de::Error::custom("duplicate PR metadata key"));
                    }
                    values.push(value);
                }
                Ok(Fields(values))
            }
        }
        deserializer.deserialize_map(Object)
    }
}

fn check(value: &RawValue, depth: usize) -> Result<(), serde_json::Error> {
    if depth > 12 {
        return Err(serde::de::Error::custom(
            "PR metadata nesting exceeds its bound",
        ));
    }
    let children = match value.get().as_bytes()[0] {
        b'{' => serde_json::from_str::<Fields<'_>>(value.get())?.0,
        b'[' => serde_json::from_str::<Vec<&RawValue>>(value.get())?,
        _ => return Ok(()),
    };
    for child in children {
        check(child, depth + 1)?;
    }
    Ok(())
}

pub(super) fn parse<T: DeserializeOwned>(bytes: &[u8], maximum: usize) -> io::Result<T> {
    let invalid = || super::invalid("Invalid or excessive PR metadata JSON.");
    if bytes.len() > maximum {
        return Err(invalid());
    }
    let raw: &RawValue = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    check(raw, 0).map_err(|_| invalid())?;
    serde_json::from_slice(bytes).map_err(|_| invalid())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_duplicate_keys_at_every_level_and_preserves_integer_precision() {
        for bytes in [
            br#"{"id":1,"id":2}"#.as_slice(),
            br#"{"items":[{"id":1,"id":2}]}"#,
        ] {
            assert!(parse::<serde_json::Value>(bytes, 1024).is_err());
        }
        assert!(
            parse::<serde_json::Value>(br#"{"id":{"$serde_json::private::Number":"1"}}"#, 1024)
                .is_err()
        );
        let value: serde_json::Value = parse(
            br#"{"id":9007199254740991,"extra":123456789012345678901234567890}"#,
            1024,
        )
        .unwrap();
        assert_eq!(value["id"].as_u64(), Some(9007199254740991));
        assert_eq!(value["extra"].to_string(), "123456789012345678901234567890");
    }
    #[test]
    fn rejects_excessive_nesting_size_invalid_utf8_and_trailing_json() {
        for bytes in [
            vec![b'[', 255, b']'],
            b"{} {}".to_vec(),
            format!("{}0{}", "[".repeat(13), "]".repeat(13)).into_bytes(),
        ] {
            assert!(parse::<serde_json::Value>(&bytes, 1024).is_err());
        }
        assert!(parse::<serde_json::Value>(b"{}", 1).is_err());
    }
}
