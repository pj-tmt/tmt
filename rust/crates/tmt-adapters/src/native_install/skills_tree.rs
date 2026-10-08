//! The bounded agent-skills tree an extension release may carry. It is covered
//! by the same integrity as the binaries: listed in the manifest inventory,
//! authenticated by the archive checksum, and digested file by file in the
//! receipt. Any violation rejects the whole release; nothing is partial.

use super::{Product, invalid};
use crate::skill_installation::{
    MAXIMUM_FILE_BYTES, MAXIMUM_FILES, MAXIMUM_SKILLS, valid_skill_file, valid_skill_name,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

/// The release directory holding `skills/<name>/<path>`.
pub(super) const ROOT: &str = "skills";

/// The CLI has no skills; its PR receipt carries two bounded application-schema
/// source closures (at most 64 paths/digests each) and one admission snapshot.
pub(super) const CLI_RECEIPT_BYTES: usize = 64 * 1024;

/// The largest receipt entry for one skill file: its quoted key
/// (`skills/<name ≤ 64>/<path ≤ 512>`), a SHA-256 digest and JSON punctuation
/// and indentation.
const RECEIPT_ENTRY_BYTES: usize = ROOT.len() + 1 + 64 + 1 + 512 + 64 + 16;

/// Receipt bound per product: extensions add room for the most skill files a
/// release may carry, derived from the skill bounds rather than rounded.
pub(super) const fn receipt_limit(product: Product) -> usize {
    match product {
        Product::Cli => CLI_RECEIPT_BYTES,
        _ => CLI_RECEIPT_BYTES + MAXIMUM_TREE_FILES * RECEIPT_ENTRY_BYTES,
    }
}

/// The most files a release's tree may hold, checked while decoding so an
/// archive never makes the reader keep more.
pub(super) const MAXIMUM_TREE_FILES: usize = MAXIMUM_SKILLS * MAXIMUM_FILES;

pub(super) fn is_skill_path(path: &str) -> bool {
    path.starts_with("skills/")
}

/// Every non-required release path must be a skill file of an extension:
/// `skills/<name>/<canonical relative path>`, at most `MAXIMUM_SKILLS` skills
/// of `MAXIMUM_FILES` files, each with a top-level `SKILL.md`, and no path that
/// is both a file and a directory.
pub(super) fn validate<'a>(
    product: Product,
    paths: impl IntoIterator<Item = &'a str>,
) -> io::Result<()> {
    let mut skills = BTreeMap::<&str, BTreeSet<&str>>::new();
    for path in paths {
        if product == Product::Cli {
            return Err(invalid("The TMT CLI release carries no agent skills."));
        }
        let (name, file) = path
            .strip_prefix("skills/")
            .and_then(|rest| rest.split_once('/'))
            .filter(|(name, file)| valid_skill_name(name) && valid_skill_file(file))
            .ok_or_else(|| invalid("Unexpected native archive path."))?;
        if !skills.entry(name).or_default().insert(file) {
            return Err(invalid("Duplicate native archive file."));
        }
    }
    if skills.len() > MAXIMUM_SKILLS {
        return Err(invalid("A release carries too many agent skills."));
    }
    for files in skills.values() {
        if files.len() > MAXIMUM_FILES || !files.contains("SKILL.md") {
            return Err(invalid(
                "Each bundled agent skill needs a SKILL.md and at most 64 files.",
            ));
        }
        if files.iter().any(|file| {
            files.iter().any(|other| {
                other
                    .strip_prefix(file)
                    .is_some_and(|rest| rest.starts_with('/'))
            })
        }) {
            return Err(invalid(
                "A bundled skill path is both a file and a directory.",
            ));
        }
    }
    Ok(())
}

/// A skill file's size bound, applied before its bytes are kept.
pub(super) fn file_fits(size: u64) -> bool {
    size > 0 && size <= MAXIMUM_FILE_BYTES as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_extension_releases_carry_bounded_canonical_skill_trees() {
        let valid = [
            "skills/tmt-ops/SKILL.md",
            "skills/tmt-ops/references/usage.md",
            "skills/tmt-sq-play/SKILL.md",
        ];
        validate(Product::Ops, valid).unwrap();
        validate(Product::Office, []).unwrap();
        assert!(validate(Product::Cli, ["skills/tmux-team/SKILL.md"]).is_err());
        for bad in [
            vec!["skills/tmt-ops/SKILL.md", "notes.txt"],
            vec!["skills/tmt-ops/SKILL.md", "skills/tmt-ops//x.md"],
            vec!["skills/tmt-ops/SKILL.md", "skills/tmt-ops/ref/"],
            vec!["skills/tmt-ops/SKILL.md", "skills/tmt-ops/.hidden"],
            vec!["skills/tmt-ops/SKILL.md", "skills/tmt-ops/../escape"],
            vec!["skills/Squad/SKILL.md"],
            vec!["skills/tmt-ops/README.md"],
            vec!["skills/tmt-ops/SKILL.md", "skills/tmt-ops/SKILL.md"],
            vec![
                "skills/tmt-ops/SKILL.md",
                "skills/tmt-ops/ref",
                "skills/tmt-ops/ref/inner.md",
            ],
        ] {
            assert!(validate(Product::Ops, bad.clone()).is_err(), "{bad:?}");
        }
        let many_skills = (0..=MAXIMUM_SKILLS)
            .map(|index| format!("skills/s{index}/SKILL.md"))
            .collect::<Vec<_>>();
        assert!(validate(Product::Ops, many_skills.iter().map(String::as_str)).is_err());
        let many_files = std::iter::once("skills/s/SKILL.md".to_owned())
            .chain((0..MAXIMUM_FILES).map(|index| format!("skills/s/f{index}.md")))
            .collect::<Vec<_>>();
        assert!(validate(Product::Ops, many_files.iter().map(String::as_str)).is_err());
        assert!(file_fits(1) && file_fits(MAXIMUM_FILE_BYTES as u64));
        assert!(!file_fits(0) && !file_fits(MAXIMUM_FILE_BYTES as u64 + 1));
    }

    #[test]
    fn only_extension_receipts_get_room_for_the_largest_skill_tree() {
        assert_eq!(receipt_limit(Product::Cli), 64 * 1024);
        let largest_entry = format!(
            "    \"skills/{}/{}\": \"{}\",\n",
            "n".repeat(64),
            "p".repeat(512),
            "0".repeat(64)
        );
        assert!(largest_entry.len() <= RECEIPT_ENTRY_BYTES);
        for product in [Product::Office, Product::Ops] {
            assert_eq!(
                receipt_limit(product),
                64 * 1024 + MAXIMUM_SKILLS * MAXIMUM_FILES * RECEIPT_ENTRY_BYTES
            );
        }
    }
}
