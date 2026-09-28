//! Strict JSON representation for the core-owned local profile domain value.

use crate::office_profile::{Appearance, LocalProfile};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProfileDecodeError;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AppearanceDocument {
    hair_style: String,
    hair_color: String,
    skin_tone: String,
    shirt_color: String,
    shirt_mark: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProfileDocument {
    display_label: String,
    description: String,
    appearance: AppearanceDocument,
    #[serde(default)]
    avatar_ref: Option<String>,
}

pub fn decode_slice(bytes: &[u8]) -> Result<LocalProfile, ProfileDecodeError> {
    let document: ProfileDocument =
        serde_json::from_slice(bytes).map_err(|_| ProfileDecodeError)?;
    decode(document)
}

pub fn decode_value(value: Value) -> Result<LocalProfile, ProfileDecodeError> {
    let document: ProfileDocument =
        serde_json::from_value(value).map_err(|_| ProfileDecodeError)?;
    decode(document)
}

fn decode(document: ProfileDocument) -> Result<LocalProfile, ProfileDecodeError> {
    let profile = LocalProfile {
        display_label: document.display_label,
        description: document.description,
        appearance: Appearance {
            hair_style: document.appearance.hair_style,
            hair_color: document.appearance.hair_color,
            skin_tone: document.appearance.skin_tone,
            shirt_color: document.appearance.shirt_color,
            shirt_mark: document.appearance.shirt_mark,
        },
        avatar_ref: document.avatar_ref,
    };
    profile.validate().map_err(|_| ProfileDecodeError)?;
    Ok(profile)
}

pub fn encode_value(profile: &LocalProfile) -> Value {
    let mut value = json!({
        "displayLabel": profile.display_label,
        "description": profile.description,
        "appearance": {
            "hairStyle": profile.appearance.hair_style,
            "hairColor": profile.appearance.hair_color,
            "skinTone": profile.appearance.skin_tone,
            "shirtColor": profile.appearance.shirt_color,
            "shirtMark": profile.appearance.shirt_mark,
        }
    });
    if let Some(avatar_ref) = &profile.avatar_ref {
        value["avatarRef"] = json!(avatar_ref);
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile_value() -> Value {
        json!({
            "displayLabel":"",
            "description":"",
            "appearance":{
                "hairStyle":"short",
                "hairColor":"ink",
                "skinTone":"medium",
                "shirtColor":"blue",
                "shirtMark":""
            }
        })
    }

    #[test]
    fn omitted_and_null_avatar_reference_decode_to_the_same_omitted_wire_value() {
        let omitted = decode_value(profile_value()).unwrap();
        let mut explicit_null = profile_value();
        explicit_null["avatarRef"] = Value::Null;
        assert_eq!(decode_value(explicit_null).unwrap(), omitted);
        assert!(encode_value(&omitted).get("avatarRef").is_none());
    }

    #[test]
    fn valid_avatar_reference_round_trips_and_non_reference_values_reject() {
        let avatar_ref = format!("sha256:{}/signal-bot", "0".repeat(64));
        let mut value = profile_value();
        value["avatarRef"] = json!(avatar_ref);
        let profile = decode_value(value).unwrap();
        assert_eq!(profile.avatar_ref.as_deref(), Some(avatar_ref.as_str()));
        assert_eq!(encode_value(&profile)["avatarRef"], avatar_ref);
        let mut invalid = profile_value();
        invalid["avatarRef"] = json!("https://example.test/avatar");
        assert_eq!(decode_value(invalid), Err(ProfileDecodeError));
    }
}
