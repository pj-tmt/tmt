//! Local Office presentation profiles. Identity, presence and layout stay separate owners.

use uuid::{Uuid, Variant, Version};

pub const MAX_REVISION: u64 = 9_007_199_254_740_991;
pub const MAX_PROFILE_FILE_BYTES: usize = 8 * 1024;
pub const HAIR_STYLES: [&str; 5] = ["short", "bob", "curls", "tied", "bald"];
pub const HAIR_COLORS: [&str; 4] = ["ink", "brown", "gold", "silver"];
pub const SKIN_TONES: [&str; 4] = ["light", "warm", "medium", "deep"];
pub const SHIRT_COLORS: [&str; 6] = ["blue", "green", "clay", "plum", "gold", "ink"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Appearance {
    pub hair_style: String,
    pub hair_color: String,
    pub skin_tone: String,
    pub shirt_color: String,
    pub shirt_mark: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalProfile {
    pub display_label: String,
    pub description: String,
    pub appearance: Appearance,
    pub avatar_ref: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileError {
    InvalidIdentityId,
    InvalidDisplayLabel,
    InvalidDescription,
    InvalidHairStyle,
    InvalidHairColor,
    InvalidSkinTone,
    InvalidShirtColor,
    InvalidShirtMark,
    InvalidAvatarReference,
}

impl LocalProfile {
    pub fn validate(&self) -> Result<(), ProfileError> {
        text(&self.display_label, 80, false).map_err(|_| ProfileError::InvalidDisplayLabel)?;
        text(&self.description, 1024, true).map_err(|_| ProfileError::InvalidDescription)?;
        member(&self.appearance.hair_style, &HAIR_STYLES)
            .map_err(|_| ProfileError::InvalidHairStyle)?;
        member(&self.appearance.hair_color, &HAIR_COLORS)
            .map_err(|_| ProfileError::InvalidHairColor)?;
        member(&self.appearance.skin_tone, &SKIN_TONES)
            .map_err(|_| ProfileError::InvalidSkinTone)?;
        member(&self.appearance.shirt_color, &SHIRT_COLORS)
            .map_err(|_| ProfileError::InvalidShirtColor)?;
        text(&self.appearance.shirt_mark, 16, false).map_err(|_| ProfileError::InvalidShirtMark)?;
        if self.avatar_ref.as_deref().is_some_and(|value| {
            crate::office_art_reference::parse_office_art_reference(value).is_none()
        }) {
            return Err(ProfileError::InvalidAvatarReference);
        }
        Ok(())
    }
}

pub fn deterministic_default(identity_id: &str) -> Result<LocalProfile, ProfileError> {
    let id = Uuid::parse_str(identity_id).map_err(|_| ProfileError::InvalidIdentityId)?;
    if id.get_variant() != Variant::RFC4122
        || id.get_version() != Some(Version::Random)
        || id.to_string() != identity_id
    {
        return Err(ProfileError::InvalidIdentityId);
    }
    let bytes = id.as_bytes();
    Ok(LocalProfile {
        display_label: String::new(),
        description: String::new(),
        appearance: Appearance {
            hair_style: HAIR_STYLES[usize::from(bytes[0]) % HAIR_STYLES.len()].into(),
            hair_color: "ink".into(),
            skin_tone: SKIN_TONES[usize::from(bytes[1]) % SKIN_TONES.len()].into(),
            shirt_color: SHIRT_COLORS[usize::from(bytes[2]) % SHIRT_COLORS.len()].into(),
            shirt_mark: String::new(),
        },
        avatar_ref: None,
    })
}

fn member(value: &str, values: &[&str]) -> Result<(), ()> {
    values.contains(&value).then_some(()).ok_or(())
}

fn text(value: &str, max_bytes: usize, multiline: bool) -> Result<(), ()> {
    if value.len() > max_bytes {
        return Err(());
    }
    if value
        .chars()
        .any(|character| character.is_control() && !(multiline && matches!(character, '\n' | '\t')))
    {
        return Err(());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vector_profile(value: &serde_json::Value) -> LocalProfile {
        LocalProfile {
            display_label: value["displayLabel"].as_str().unwrap().into(),
            description: value["description"].as_str().unwrap().into(),
            appearance: Appearance {
                hair_style: value["appearance"]["hairStyle"].as_str().unwrap().into(),
                hair_color: value["appearance"]["hairColor"].as_str().unwrap().into(),
                skin_tone: value["appearance"]["skinTone"].as_str().unwrap().into(),
                shirt_color: value["appearance"]["shirtColor"].as_str().unwrap().into(),
                shirt_mark: value["appearance"]["shirtMark"].as_str().unwrap().into(),
            },
            avatar_ref: value["avatarRef"].as_str().map(Into::into),
        }
    }

    #[test]
    fn default_uses_literal_uuid_bytes_without_writing_or_randomness() {
        let profile = deterministic_default("01020304-0000-4000-8000-000000000000").unwrap();
        assert_eq!(profile.appearance.hair_style, "bob");
        assert_eq!(profile.appearance.skin_tone, "medium");
        assert_eq!(profile.appearance.shirt_color, "plum");
        assert_eq!(profile.appearance.hair_color, "ink");
        assert_eq!(
            deterministic_default("01020304-0000-1000-8000-000000000000"),
            Err(ProfileError::InvalidIdentityId)
        );
    }

    #[test]
    fn validation_is_byte_bounded_and_rejects_controls_and_unknown_catalog_values() {
        let mut profile = deterministic_default("01020304-0000-4000-8000-000000000000").unwrap();
        profile.display_label = "é".repeat(40);
        assert_eq!(profile.validate(), Ok(()));
        profile.display_label.push('x');
        assert_eq!(profile.validate(), Err(ProfileError::InvalidDisplayLabel));
        profile.display_label = "bad\nlabel".into();
        assert_eq!(profile.validate(), Err(ProfileError::InvalidDisplayLabel));
        profile.display_label.clear();
        profile.description = "line one\n\tline two".into();
        assert_eq!(profile.validate(), Ok(()));
        profile.appearance.hair_style = "random".into();
        assert_eq!(profile.validate(), Err(ProfileError::InvalidHairStyle));
        profile.appearance.hair_style = "short".into();
        profile.avatar_ref = Some("https://example.test/avatar".into());
        assert_eq!(
            profile.validate(),
            Err(ProfileError::InvalidAvatarReference)
        );
    }

    #[test]
    fn shared_conformance_vectors_match_catalog_defaults_and_validation() {
        let vectors: serde_json::Value =
            serde_json::from_str(include_str!("../../../contracts/profile-v1.vectors.json"))
                .unwrap();
        assert_eq!(
            vectors["catalog"]["hairStyles"],
            serde_json::json!(HAIR_STYLES)
        );
        assert_eq!(
            vectors["catalog"]["hairColors"],
            serde_json::json!(HAIR_COLORS)
        );
        assert_eq!(
            vectors["catalog"]["skinTones"],
            serde_json::json!(SKIN_TONES)
        );
        assert_eq!(
            vectors["catalog"]["shirtColors"],
            serde_json::json!(SHIRT_COLORS)
        );
        for vector in vectors["defaults"].as_array().unwrap() {
            let expected = vector_profile(&vector["profile"]);
            assert_eq!(
                deterministic_default(vector["identityId"].as_str().unwrap()).unwrap(),
                expected
            );
        }
        for value in vectors["validProfiles"].as_array().unwrap() {
            assert_eq!(vector_profile(value).validate(), Ok(()));
        }
        for value in vectors["invalidProfiles"].as_array().unwrap() {
            assert!(vector_profile(value).validate().is_err());
        }
    }
}
