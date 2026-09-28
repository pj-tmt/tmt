//! Office avatar pack file acquisition used by the CLI.

use std::path::Path;
use tmt_office_model::codec::office_avatar::*;

pub fn read_pack_file(path: &Path) -> Result<ValidatedAvatarPack, AvatarPackError> {
    let bytes =
        crate::bounded_file::read_no_follow(path, PACK_INPUT_LIMIT).map_err(
            |error| match error {
                crate::bounded_file::FileReadError::TooLarge => AvatarPackError::TooLarge,
                crate::bounded_file::FileReadError::Io(error) => AvatarPackError::Io(error),
            },
        )?;
    validate_pack(&bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_bytes() -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "formatVersion":1,"label":"Signal bots","credit":"tmux-team","license":"MIT",
            "palette":["#00000000","#ffffffff"],
            "avatars":[{"key":"signal-bot","label":"Signal bot","pixels":vec!["1111111111111111";24]}]
        })).unwrap()
    }

    #[test]
    fn file_acquisition_is_bounded_regular_and_no_follow() {
        let directory = crate::test_support::TestDirectory::new();
        let regular = directory.path.join("pack.json");
        std::fs::write(&regular, valid_bytes()).unwrap();
        assert!(read_pack_file(&regular).is_ok());
        let oversized = directory.path.join("oversized.json");
        std::fs::write(&oversized, vec![b' '; PACK_INPUT_LIMIT + 1]).unwrap();
        assert_eq!(read_pack_file(&oversized), Err(AvatarPackError::TooLarge));
        let link = directory.path.join("link.json");
        std::os::unix::fs::symlink(&regular, &link).unwrap();
        assert!(matches!(read_pack_file(&link), Err(AvatarPackError::Io(_))));
        assert!(matches!(
            read_pack_file(&directory.path),
            Err(AvatarPackError::Io(_))
        ));
    }
}
