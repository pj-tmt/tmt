//! `tmt storage` (#5036): what each local store occupies, and whether the
//! one-time compaction of the Core database is still pending. Read-only.

use crate::{invocation::OutputMode, output::Failure};
use serde_json::{Value, json};
use std::{
    io::{self, Write},
    path::Path,
};
use tmt_adapters::{
    config::ConfigPaths,
    storage::{AutoVacuum, DatabaseFootprint, Footprint, footprint},
};
use tmt_cli_style::{Terminal, detail, progress::bytes, value};

/// The command that finishes the compaction: `tmt upgrade` runs it as a
/// post-step, and the step itself is an internal entry point.
const FINISH_COMPACTION: &str = "tmt upgrade";

/// Whether the one-time compaction is still to run: `None` when the database's
/// mode could not be read.
fn compaction_pending(database: Option<&DatabaseFootprint>) -> Option<bool> {
    match database {
        None => Some(false),
        Some(database) => database
            .pages
            .as_ref()
            .ok()
            .map(|pages| pages.auto_vacuum != AutoVacuum::Incremental),
    }
}

fn path_text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

pub(crate) fn document(footprint: &Footprint) -> Value {
    let pending = compaction_pending(footprint.database.as_ref());
    let database = match &footprint.database {
        None => json!({"path": path_text(&footprint.database_path), "present": false}),
        Some(database) => {
            let pages = database.pages.as_ref().ok();
            json!({
                "path": path_text(&database.path),
                "present": true,
                "bytes": database.bytes,
                "walBytes": database.wal_bytes,
                "pageSize": pages.map(|pages| pages.page_size),
                "pageCount": pages.map(|pages| pages.page_count),
                "freePages": pages.map(|pages| pages.freelist_count),
                "freeBytes": pages.map(|pages| pages.free_bytes()),
                "autoVacuum": pages.map(|pages| pages.auto_vacuum.as_str()),
                "error": database.pages.as_ref().err(),
            })
        }
    };
    let notes = match &footprint.notes {
        None => json!({"path": path_text(&footprint.notes_path), "present": false}),
        Some(notes) => json!({
            "path": path_text(&notes.path),
            "present": true,
            "bytes": notes.bytes,
            "files": notes.files,
            "complete": notes.complete,
        }),
    };
    let colab = match &footprint.colab {
        None => json!({"path": path_text(&footprint.colab_path), "present": false}),
        Some(colab) => json!({
            "path": path_text(&colab.path),
            "present": true,
            "bytes": colab.bytes,
            "walBytes": colab.wal_bytes,
            "backups": colab.backups.iter().map(|backup| json!({
                "path": path_text(&backup.path),
                "bytes": backup.bytes,
            })).collect::<Vec<_>>(),
        }),
    };
    json!({
        "dataRoot": path_text(&footprint.data_root),
        "database": database,
        "notes": notes,
        "colab": colab,
        "compaction": {
            "pending": pending,
            "command": pending.unwrap_or(false).then_some(FINISH_COMPACTION),
        },
    })
}

fn with_log(size: u64, wal: u64) -> String {
    if wal == 0 {
        bytes(size)
    } else {
        format!("{} and a {} log", bytes(size), bytes(wal))
    }
}

pub(crate) fn write_human(
    output: &mut impl Write,
    terminal: Terminal,
    footprint: &Footprint,
    home: Option<&Path>,
) -> io::Result<()> {
    let shown = |path: &Path| value::home_path(path, home);
    let absent = || ("state", "absent".to_owned());
    detail::write(
        output,
        terminal,
        "Local storage",
        &[("data root", shown(&footprint.data_root))],
    )?;
    writeln!(output)?;
    let database = match &footprint.database {
        None => vec![("path", shown(&footprint.database_path)), absent()],
        Some(database) => {
            let mut fields = vec![
                ("path", shown(&database.path)),
                ("size", with_log(database.bytes, database.wal_bytes)),
            ];
            match &database.pages {
                Ok(pages) => {
                    let pending = pages.auto_vacuum != AutoVacuum::Incremental;
                    fields.extend([
                        (
                            "free",
                            format!(
                                "{} in {} pages",
                                bytes(pages.free_bytes()),
                                pages.freelist_count
                            ),
                        ),
                        ("auto-vacuum", pages.auto_vacuum.as_str().to_owned()),
                        (
                            "compaction",
                            if pending { "pending" } else { "done" }.to_owned(),
                        ),
                    ]);
                }
                Err(error) => fields.push(("pages", format!("unreadable: {error}"))),
            }
            fields
        }
    };
    detail::write(output, terminal, "Core database", &database)?;
    writeln!(output)?;
    let notes = match &footprint.notes {
        None => vec![("path", shown(&footprint.notes_path)), absent()],
        Some(notes) => {
            let at_least = if notes.complete { "" } else { "at least " };
            vec![
                ("path", shown(&notes.path)),
                (
                    "size",
                    format!("{at_least}{} in {} files", bytes(notes.bytes), notes.files),
                ),
            ]
        }
    };
    detail::write(output, terminal, "Notes", &notes)?;
    writeln!(output)?;
    let colab = match &footprint.colab {
        None => vec![("path", shown(&footprint.colab_path)), absent()],
        Some(colab) => {
            let backups = if colab.backups.is_empty() {
                "none".to_owned()
            } else {
                colab
                    .backups
                    .iter()
                    .map(|backup| {
                        let name = backup
                            .path
                            .file_name()
                            .map_or_else(String::new, |name| name.to_string_lossy().into_owned());
                        format!("{name} {}", bytes(backup.bytes))
                    })
                    .collect::<Vec<_>>()
                    .join(" · ")
            };
            vec![
                ("path", shown(&colab.path)),
                ("size", with_log(colab.bytes, colab.wal_bytes)),
                ("backups", backups),
            ]
        }
    };
    detail::write(output, terminal, "Colab", &colab)?;
    if compaction_pending(footprint.database.as_ref()) == Some(true) {
        tmt_cli_style::message::hint(
            output,
            terminal,
            "finish the one-time compaction of tmt.db with: tmt upgrade",
        )?;
    }
    Ok(())
}

pub fn execute(mode: OutputMode) -> io::Result<u8> {
    let result = ConfigPaths::discover()
        .map_err(io::Error::other)
        .and_then(|paths| footprint(&paths));
    let footprint = match result {
        Ok(footprint) => footprint,
        Err(error) => {
            return Failure::new(
                "STORAGE_REPORT_FAILED",
                format!("Could not read the local stores: {error}."),
                1,
            )
            .caused_by(error)
            .publish(mode);
        }
    };
    let mut output = tmt_cli_style::stream::stdout(mode.json);
    if mode.json {
        writeln!(output, "{}", document(&footprint))?;
    } else {
        let terminal = output.terminal();
        write_human(
            &mut output,
            terminal,
            &footprint,
            std::env::home_dir().as_deref(),
        )?;
    }
    Ok(0)
}

// Source-checked command samples for the printed-command guard.
#[cfg(test)]
pub(crate) const PRINTED_HINTS: &[crate::cli_style_tests::HintSpec] = &[
    crate::cli_style_tests::HintSpec::core("tmt upgrade", &[""], &[]),
    crate::cli_style_tests::HintSpec::core(
        "finish the one-time compaction of tmt.db with: tmt upgrade",
        &[""],
        &[],
    ),
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tmt_adapters::storage::{ColabFootprint, DatabasePages, NotesFootprint, StoreFile};

    fn absent() -> Footprint {
        Footprint {
            data_root: "/home/ada/.config/tmt".into(),
            database_path: "/home/ada/.config/tmt/tmt.db".into(),
            database: None,
            notes_path: "/home/ada/.config/tmt/notes".into(),
            notes: None,
            colab_path: "/home/ada/.config/tmt/colab/space.db".into(),
            colab: None,
        }
    }

    fn present(auto_vacuum: AutoVacuum) -> Footprint {
        Footprint {
            database: Some(DatabaseFootprint {
                path: "/home/ada/.config/tmt/tmt.db".into(),
                bytes: 152_000_000,
                wal_bytes: 4_000_000,
                pages: Ok(DatabasePages {
                    page_size: 4096,
                    page_count: 37_109,
                    freelist_count: 34_180,
                    auto_vacuum,
                }),
            }),
            notes: Some(NotesFootprint {
                path: "/home/ada/.config/tmt/notes".into(),
                bytes: 500,
                files: 2,
                complete: true,
            }),
            colab: Some(ColabFootprint {
                path: "/home/ada/.config/tmt/colab/space.db".into(),
                bytes: 4096,
                wal_bytes: 0,
                backups: vec![StoreFile {
                    path: PathBuf::from("/home/ada/.config/tmt/colab/space.db.pre-v12.bak"),
                    bytes: 100,
                }],
            }),
            ..absent()
        }
    }

    fn human(footprint: &Footprint) -> String {
        let mut output = Vec::new();
        let terminal = Terminal {
            color: false,
            width: None,
            theme: None,
        };
        write_human(
            &mut output,
            terminal,
            footprint,
            Some(Path::new("/home/ada")),
        )
        .unwrap();
        String::from_utf8(output).unwrap()
    }

    #[test]
    fn a_pending_compaction_names_its_command_in_both_forms() {
        let footprint = present(AutoVacuum::None);
        assert_eq!(
            human(&footprint),
            "Local storage\n  data root  ~/.config/tmt\n\n\
             Core database\n  path         ~/.config/tmt/tmt.db\n  size         152.0 MB and a 4.0 MB log\n  free         140.0 MB in 34180 pages\n  auto-vacuum  none\n  compaction   pending\n\n\
             Notes\n  path  ~/.config/tmt/notes\n  size  500 B in 2 files\n\n\
             Colab\n  path     ~/.config/tmt/colab/space.db\n  size     4.1 kB\n  backups  space.db.pre-v12.bak 100 B\n\
             hint: finish the one-time compaction of tmt.db with: tmt upgrade\n"
        );
        let document = document(&footprint);
        assert_eq!(
            document["compaction"],
            json!({"pending": true, "command": "tmt upgrade"})
        );
        assert_eq!(
            document["database"],
            json!({
                "path": "/home/ada/.config/tmt/tmt.db",
                "present": true,
                "bytes": 152_000_000,
                "walBytes": 4_000_000,
                "pageSize": 4096,
                "pageCount": 37_109,
                "freePages": 34_180,
                "freeBytes": 140_001_280_u64,
                "autoVacuum": "none",
                "error": null,
            })
        );
        assert_eq!(
            document["colab"]["backups"],
            json!([{"path": "/home/ada/.config/tmt/colab/space.db.pre-v12.bak", "bytes": 100}])
        );
        assert_eq!(document["notes"]["files"], 2);
    }

    #[test]
    fn a_compacted_database_has_no_pending_step() {
        let footprint = present(AutoVacuum::Incremental);
        let text = human(&footprint);
        assert!(text.contains("  compaction   done\n"), "{text}");
        assert!(!text.contains("hint:"), "{text}");
        assert_eq!(
            document(&footprint)["compaction"],
            json!({"pending": false, "command": null})
        );
    }

    #[test]
    fn absent_stores_are_shown_as_absent() {
        let footprint = absent();
        assert_eq!(
            human(&footprint),
            "Local storage\n  data root  ~/.config/tmt\n\n\
             Core database\n  path   ~/.config/tmt/tmt.db\n  state  absent\n\n\
             Notes\n  path   ~/.config/tmt/notes\n  state  absent\n\n\
             Colab\n  path   ~/.config/tmt/colab/space.db\n  state  absent\n"
        );
        assert_eq!(
            document(&footprint),
            json!({
                "dataRoot": "/home/ada/.config/tmt",
                "database": {"path": "/home/ada/.config/tmt/tmt.db", "present": false},
                "notes": {"path": "/home/ada/.config/tmt/notes", "present": false},
                "colab": {"path": "/home/ada/.config/tmt/colab/space.db", "present": false},
                "compaction": {"pending": false, "command": null},
            })
        );
    }

    #[test]
    fn an_unreadable_database_leaves_the_compaction_state_unknown() {
        let mut footprint = present(AutoVacuum::None);
        footprint.database.as_mut().unwrap().pages =
            Err("Open storage page observation failed".into());
        let text = human(&footprint);
        assert!(
            text.contains("  pages  unreadable: Open storage page observation failed\n"),
            "{text}"
        );
        assert!(!text.contains("hint:"), "{text}");
        let document = document(&footprint);
        assert_eq!(
            document["compaction"],
            json!({"pending": null, "command": null})
        );
        assert_eq!(
            document["database"]["error"],
            "Open storage page observation failed"
        );
    }
}
