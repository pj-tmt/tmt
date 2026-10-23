//! The running versions the header and Help name: the installed `tmt` CLI and this Ops build.
use crate::core::Core;

/// The Ops build's own version.
pub(super) const OPS: &str = env!("CARGO_PKG_VERSION");

/// Read once per board process on the lane; the board paints the mark alone until it arrives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Versions {
    pub cli: String,
    pub ops: String,
}

impl Versions {
    pub fn read(core: &Core) -> Option<Self> {
        Some(Self::new(core.cli_version()?, OPS))
    }

    pub fn new(cli: impl Into<String>, ops: impl Into<String>) -> Self {
        Self {
            cli: cli.into(),
            ops: ops.into(),
        }
    }

    /// What follows the mark: the CLI version, and the Ops build's own when they differ.
    pub fn named(&self) -> (&str, Option<&str>) {
        (
            &self.cli,
            (self.ops != self.cli).then_some(self.ops.as_str()),
        )
    }

    /// The short form of the CLI version, for a header with no room for the whole.
    pub fn compact(&self) -> &str {
        compact(&self.cli)
    }
}

/// The short form of a version: its prerelease tail (`alpha.378` of `5.0.0-alpha.378`), or the
/// plain version when it has none. Build metadata is never part of it.
fn compact(version: &str) -> &str {
    let version = version.split('+').next().unwrap_or(version);
    version.split_once('-').map_or(version, |(_, tail)| tail)
}

#[cfg(test)]
pub(in crate::board) mod tests {
    use super::*;

    use crate::board::{app::App, app::workspace::tests::board, view::render_frame};
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    /// Oracle scenes: the header with the CLI and Ops versions, equal or differing, and Help.
    pub(in crate::board) const SCENES: &[&str] = &[
        "header-version-differs",
        "header-version-same",
        "help-versions",
    ];

    pub(in crate::board) fn capture_case(scene: &str, look: crate::look::Look) -> App {
        let mut app = board();
        let view = app.view.as_mut().unwrap();
        view.look = look;
        view.home.as_mut().unwrap().workspace.squad_looks.clear();
        app.versions = Some(match scene {
            "header-version-same" => Versions::new("5.0.0-alpha.378", "5.0.0-alpha.378"),
            _ => Versions::new("5.0.0-alpha.378", "0.1.0-alpha.4"),
        });
        if scene == "help-versions" {
            app.key(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE));
            assert!(app.modal_open(), "Help is open");
        }
        app
    }
    use ratatui::{Terminal, backend::TestBackend};

    fn first_row(app: &App, width: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, 36)).unwrap();
        terminal
            .draw(|frame| render_frame(frame, app, None, Vec::new()))
            .unwrap();
        let buffer = terminal.backend().buffer();
        (0..width).map(|x| buffer[(x, 0)].symbol()).collect()
    }

    #[test]
    fn the_header_paints_the_mark_alone_until_the_versions_arrive_and_help_names_both() {
        let mut app = board();
        assert!(app.versions.is_none());
        for width in [150, 80] {
            let row = first_row(&app, width);
            assert!(row.starts_with(" ▚ TMT   OPS / UI"), "{width}: {row:?}");
        }
        assert!(
            !crate::board::help::model(&app)
                .value()
                .to_string()
                .contains("versions")
        );

        app.versions = Some(Versions::new("5.0.0-alpha.378", "0.1.0-alpha.4"));
        let row = first_row(&app, 150);
        assert!(
            row.starts_with(" ▚ TMT 5.0.0-alpha.378 · ops 0.1.0-alpha.4   OPS / UI"),
            "{row:?}"
        );
        // At 80 columns the version wins over the Snapshots control; the counts stay.
        let row = first_row(&app, 80);
        assert!(
            row.starts_with(" ▚ TMT 5.0.0-alpha.378   OPS / UI")
                && !row.contains("Snapshots")
                && row.contains("1 online"),
            "{row:?}"
        );
        let help = crate::board::help::model(&app).value();
        assert_eq!(
            help["sections"][0]["entries"],
            serde_json::json!([
                {"id": "row-0", "keys": "tmt", "description": "5.0.0-alpha.378"},
                {"id": "row-1", "keys": "ops", "description": "0.1.0-alpha.4"},
            ]),
            "Help opens with both versions in full"
        );
    }

    #[test]
    fn the_lane_reads_the_versions_once_and_an_unreadable_cli_leaves_them_unknown() {
        let dir = std::env::temp_dir().join(format!("ops-versions-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let fake = dir.join("tmt");
        crate::test_support::write_ready_executable(&fake, "#!/bin/sh\necho 'tmt 5.0.0-alpha.9'\n");
        let read = |core: &Core| match crate::board::lane::run(
            core,
            None,
            crate::board::lane::Job::Versions,
        ) {
            crate::board::BoardEvent::VersionsRead(versions) => versions,
            _ => panic!("the versions job answers with its own event"),
        };
        assert_eq!(
            read(&Core::at(fake)),
            Some(Versions::new("5.0.0-alpha.9", OPS))
        );
        let broken = dir.join("broken");
        crate::test_support::write_ready_executable(&broken, "#!/bin/sh\nexit 1\n");
        assert_eq!(read(&Core::at(broken)), None);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_compact_form_is_the_prerelease_tail_or_the_plain_version() {
        for (full, short) in [
            ("5.0.0-alpha.378", "alpha.378"),
            ("5.0.0-alpha.378+abc", "alpha.378"),
            ("5.0.0-rc.1-x", "rc.1-x"),
            ("5.0.0", "5.0.0"),
            ("5.0.0+build.7", "5.0.0"),
        ] {
            assert_eq!(compact(full), short, "{full}");
        }
    }

    #[test]
    fn the_ops_build_is_named_only_when_it_differs_from_the_cli() {
        assert_eq!(Versions::new("5.0.0", "5.0.0").named(), ("5.0.0", None));
        assert_eq!(
            Versions::new("5.0.0-alpha.378", "0.1.0-alpha.4").named(),
            ("5.0.0-alpha.378", Some("0.1.0-alpha.4"))
        );
    }
}
