//! The `tmt` that companions must use to reach core.
//!
//! Core declares itself explicitly at startup; no file name is inspected, so a
//! CLI installed under any name (`tmt`, `tmux-team`) hands its own path down.
//! A process that is not core (the Office companion's direct mode) declares
//! nothing and passes along the `tmt` that launched it.

use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

static DECLARED: OnceLock<PathBuf> = OnceLock::new();

/// Called once by the core binary: this process is the `tmt` companions must use.
pub fn declare_core() {
    if let Ok(path) = std::env::current_exe() {
        let _ = DECLARED.set(path);
    }
}

/// The declared core, else the inherited absolute `TMT_EXECUTABLE`, else this executable.
pub fn selected() -> io::Result<PathBuf> {
    Ok(select(
        DECLARED.get().map(PathBuf::as_path),
        std::env::var_os("TMT_EXECUTABLE"),
        std::env::current_exe()?,
    ))
}

fn select(declared: Option<&Path>, inherited: Option<OsString>, current: PathBuf) -> PathBuf {
    if let Some(declared) = declared {
        return declared.to_path_buf();
    }
    inherited
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or(current)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declared_core_wins_under_any_name_and_over_the_inherited_selection() {
        let chosen = select(
            Some(Path::new("/opt/bin/tmux-team")),
            Some("/other/tmt".into()),
            "/x/tmt-office".into(),
        );
        assert_eq!(chosen, Path::new("/opt/bin/tmux-team"));
    }

    #[test]
    fn an_embedded_process_passes_the_launching_tmt_and_ignores_relative_values() {
        let current = PathBuf::from("/x/tmt-office");
        assert_eq!(
            select(None, Some("/opt/tmt".into()), current.clone()),
            Path::new("/opt/tmt")
        );
        assert_eq!(select(None, Some("tmt".into()), current.clone()), current);
        assert_eq!(select(None, None, current.clone()), current);
    }
}
