//! Canonical local profile JSON file boundary used by the CLI.

use std::path::Path;
use tmt_office_model::office_profile::LocalProfile;
use tmt_office_model::office_profile::MAX_PROFILE_FILE_BYTES;
use tmt_office_model::office_protocol::OfficeError;

pub fn read_profile_file(path: &Path) -> Result<LocalProfile, OfficeError> {
    let bytes = tmt_adapters::bounded_file::read(path, MAX_PROFILE_FILE_BYTES)
        .map_err(|_| OfficeError::ProfileInvalid)?;
    tmt_office_model::codec::office_profile_wire::decode_slice(&bytes)
        .map_err(|_| OfficeError::ProfileInvalid)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TestDirectory;

    #[test]
    fn profile_file_is_exact_bounded_and_never_rewritten() {
        let directory = TestDirectory::new();
        let file = directory.path.join("profile.json");
        let value = br#"{"displayLabel":"","description":"Architecture review","appearance":{"hairStyle":"short","hairColor":"ink","skinTone":"medium","shirtColor":"blue","shirtMark":"AI"}}"#;
        std::fs::write(&file, value).unwrap();
        assert_eq!(
            read_profile_file(&file).unwrap().description,
            "Architecture review"
        );
        assert_eq!(std::fs::read(&file).unwrap(), value);

        let mut oversized = value.to_vec();
        oversized.resize(MAX_PROFILE_FILE_BYTES + 1, b' ');
        std::fs::write(&file, &oversized).unwrap();
        assert_eq!(read_profile_file(&file), Err(OfficeError::ProfileInvalid));

        std::fs::write(&file, br#"{"displayLabel":"","description":"","appearance":{"hairStyle":"short","hairColor":"ink","skinTone":"medium","shirtColor":"blue","shirtMark":""},"asset":"https://example.test/avatar"}"#).unwrap();
        assert_eq!(read_profile_file(&file), Err(OfficeError::ProfileInvalid));
    }
}
