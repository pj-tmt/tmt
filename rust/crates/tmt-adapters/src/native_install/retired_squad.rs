//! Squad, the product Ops replaced. TMT no longer upgrades, replaces or switches an
//! installation of it; a leftover is reported, never touched.

use std::{
    fs,
    path::{Path, PathBuf},
};

const NAMESPACE: &str = "lib/tmt-squad";
const LINKS: [&str; 2] = ["tmt-squad", "tmt-sq"];

/// What a Squad installation left under an installation prefix.
#[derive(Debug, PartialEq, Eq)]
pub struct RetiredSquad {
    /// The Squad directory, `lib/tmt-squad` under the prefix.
    pub root: PathBuf,
    /// The Squad command links that still exist in `bin`.
    pub links: Vec<PathBuf>,
}

/// The Squad installation left under `prefix`, or `None`. Names only; nothing is
/// read from it and nothing is verified, so a damaged leftover is reported too.
pub fn retired_squad(prefix: &Path) -> Option<RetiredSquad> {
    let root = prefix.join(NAMESPACE);
    fs::symlink_metadata(&root).ok()?;
    let links = LINKS
        .iter()
        .map(|name| prefix.join("bin").join(name))
        .filter(|link| fs::symlink_metadata(link).is_ok())
        .collect();
    Some(RetiredSquad { root, links })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TestDirectory;

    #[test]
    fn reports_the_directory_and_only_the_links_that_exist() {
        let directory = TestDirectory::new();
        let prefix = directory.path.as_path();
        assert_eq!(retired_squad(prefix), None);
        fs::create_dir_all(prefix.join("bin")).unwrap();
        fs::create_dir_all(prefix.join("lib/tmt-ops")).unwrap();
        assert_eq!(retired_squad(prefix), None, "Ops is not Squad");
        fs::create_dir_all(prefix.join(NAMESPACE)).unwrap();
        fs::write(prefix.join("bin/tmt-sq"), "").unwrap();
        let found = retired_squad(prefix).unwrap();
        assert_eq!(found.root, prefix.join(NAMESPACE));
        assert_eq!(found.links, [prefix.join("bin/tmt-sq")]);
    }
}
