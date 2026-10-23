//! One shared modal/scroll/list pipeline; literal source never becomes argv.
use super::*;
use crate::look::Look;
use ratatui::{Frame, layout::Rect};
use serde_json::json;
use std::sync::OnceLock;
use tmt_cli_style::mark::Mark;
use tmt_tui::components::{KeyHint, ListRow, Modal, Placement, footer as fit_hints, surface};

pub(super) const PAGE_SIZE: usize = 150;
const FILE: &str = "ops.recovery.xml";
const MARKUP: &str = r#"<tmt-view version="1"><tmt-modal id="recovery" title="Workspace snapshots" placement="center" class="w-120 h-38"><tmt-scroll id="body"><tmt-repeat each="$.notes" as="note"><tmt-text id-bind="note.id" bind="note.text" wrap="true"/></tmt-repeat><tmt-list id="choices" bind="$.rows" empty=""><tmt-row class="flex-row gap-1"><tmt-text bind="row.cursor" class="w-1 shrink-0"/><tmt-text bind="row.label" wrap="true"/></tmt-row></tmt-list></tmt-scroll><tmt-text slot="query" bind="$.query" token="accent" wrap="true"/><tmt-text slot="status" bind="$.status" token="waiting" wrap="true"/><tmt-text slot="footer" bind="$.footer" token="muted" wrap="true"/></tmt-modal></tmt-view>"#;

pub(super) struct Row {
    pub id: String,
    pub label: String,
    pub disabled: bool,
}
impl Row {
    fn choice(id: &str, label: impl Into<String>, disabled: bool) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            disabled,
        }
    }
}
fn safe(text: &str) -> String {
    // Show hidden direction/format controls visibly before the shared plain-text
    // sanitizer. Raw identifiers that differ by such a control must not collapse
    // to one visible label or one stable note identity.
    let visible: String = text
        .chars()
        .flat_map(|ch| {
            if matches!(ch, '\u{061c}' | '\u{180e}' | '\u{200b}' | '\u{200e}'..='\u{200f}'
            | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{2064}' | '\u{2066}'..='\u{206f}'
            | '\u{feff}' | '\u{fff9}'..='\u{fffb}')
            {
                ch.escape_default().collect::<Vec<_>>()
            } else {
                vec![ch]
            }
        })
        .collect();
    super::super::notes::sanitize(&tmt_cli_style::table::escape(&visible))
}

fn captured(ms: u64) -> String {
    i64::try_from(ms)
        .ok()
        .and_then(|ms| jiff::Timestamp::from_millisecond(ms).ok())
        .map_or_else(
            || format!("timestamp {ms} ms"),
            |time| time.strftime("%Y-%m-%d %H:%M:%S UTC").to_string(),
        )
}
fn mode(layout_only: bool) -> &'static str {
    if layout_only {
        "Layout only — windows and shells; no agents or commands"
    } else {
        "Full — layout, resume agents and rerun recorded tmt commands"
    }
}
impl Controller {
    pub(super) fn rows(&self) -> Vec<Row> {
        let busy = self.pending.as_ref().is_some_and(|t| t.job.writes());
        let mut rows = match self.screen {
            Screen::List => {
                let mut rows: Vec<_> = self
                    .entries
                    .iter()
                    .map(|e| {
                        Row::choice(
                            &e.id(),
                            format!(
                                "{} · {} · {}",
                                e.label(),
                                captured(e.captured_at_ms),
                                if e.name.is_none() {
                                    "mutable latest"
                                } else {
                                    "immutable saved"
                                }
                            ),
                            false,
                        )
                    })
                    .collect();
                rows.extend([
                    Row::choice("details", "Open selected preview", self.preview.is_none()),
                    Row::choice(
                        "save",
                        "Save latest capture",
                        !self.entries.iter().any(|e| e.name.is_none())
                            || self.entries.iter().filter(|e| e.name.is_some()).count() >= 16,
                    ),
                    Row::choice(
                        "delete",
                        "Delete named snapshot…",
                        self.selected_entry().is_none_or(|e| e.name.is_none()),
                    ),
                    Row::choice("refresh", "Refresh inventory", false),
                    Row::choice("close", "Close", false),
                ]);
                rows
            }
            Screen::Preview => vec![
                Row::choice(
                    "mode",
                    format!("Restore mode: {}", mode(self.layout_only)),
                    false,
                ),
                Row::choice(
                    "review",
                    "Review restore…",
                    self.preview.is_none() || self.pending.is_some(),
                ),
                Row::choice("back", "Back to snapshots", false),
            ],
            Screen::Confirm => vec![
                Row::choice("cancel", "Cancel", false),
                Row::choice("confirm", "Restore", self.pending.is_some()),
            ],
            Screen::Save => vec![
                Row::choice("cancel", "Cancel", false),
                Row::choice("save-confirm", "Save latest capture", busy),
            ],
            Screen::Delete => vec![
                Row::choice("cancel", "Cancel", false),
                Row::choice("delete-confirm", "Delete snapshot", busy),
            ],
            Screen::Results => vec![
                Row::choice("back", "Back to snapshots", busy),
                Row::choice("close", "Close (retains this result)", false),
            ],
        };
        if matches!(self.screen, Screen::Preview | Screen::Results)
            && self.details.len() > PAGE_SIZE
        {
            rows.splice(
                0..0,
                [
                    Row::choice(
                        "previous-details",
                        "Previous detail page",
                        self.detail_page == 0,
                    ),
                    Row::choice(
                        "next-details",
                        "Next detail page",
                        (self.detail_page + 1) * PAGE_SIZE >= self.details.len(),
                    ),
                ],
            );
        }
        if busy {
            if !rows.iter().any(|r| r.id == "close") {
                rows.push(Row::choice(
                    "close",
                    "Close (submitted work continues)",
                    false,
                ));
            }
            rows.iter_mut().for_each(|r| r.disabled = r.id != "close");
        }
        rows
    }
    fn notes(&self) -> Vec<String> {
        let mut notes =
            vec!["Save which agents run where, and bring them back after a restart.".into()];
        if let Some(socket) = &self.socket {
            notes.push(format!("Original tmux socket: {}", safe(socket)));
        }
        match self.screen {
            Screen::List => {
                if self.entries.iter().filter(|e| e.name.is_some()).count() >= 16 {
                    notes.push("16 named snapshots retained; explicitly delete a named save before saving another.".into());
                }
                if let Some(p) = &self.preview {
                    notes.push(format!(
                        "Selected {} · captured {}",
                        p.name.as_deref().unwrap_or("Latest"),
                        captured(p.snapshot.captured_at_ms)
                    ));
                    notes.push(p.counts());
                }
            }
            Screen::Save => {
                notes.push(
                    "Save latest capture — keeps the latest automatic snapshot under a name; it does not take a new capture.".into(),
                );
                if let Some(e) = self.entries.iter().find(|e| e.name.is_none()) {
                    notes.push(format!(
                        "Capture time retained: {}",
                        captured(e.captured_at_ms)
                    ));
                }
                notes.push(
                    "Name: 1–64 lowercase letters, digits, _ or -; latest is reserved.".into(),
                );
            }
            Screen::Delete => {
                if let Some(e) = self.selected_entry() {
                    notes.push(format!(
                        "Delete named snapshot {} (captured {})?",
                        e.label(),
                        captured(e.captured_at_ms)
                    ));
                }
                notes.push("This removes the named save only. It does not remove Latest or any live resource.".into());
            }
            Screen::Preview | Screen::Confirm => {
                if let Some(p) = &self.preview {
                    notes.push(format!(
                        "Snapshot: {} · captured {}",
                        p.name.as_deref().unwrap_or("Latest (mutable)"),
                        captured(p.snapshot.captured_at_ms)
                    ));
                    notes.push(format!("Digest: {}", p.digest));
                    notes.push(format!("Mode: {}", mode(self.layout_only)));
                    notes.push(p.counts());
                    let candidates = p
                        .panes
                        .iter()
                        .filter(|pane| {
                            matches!(pane.action.as_str(), "resumable" | "relaunch_command")
                        })
                        .count();
                    notes.push(if self.layout_only { "No agents or commands will start (layout only).".into() }
                        else { format!("Full-mode pane candidates: {candidates}; existing-session skips and live rechecks still apply.") });
                    notes.push("Restores only into the original tmux server. Existing sessions are skipped; nothing is overwritten, retried or rolled back. Live changes can still change what happens.".into());
                    if self.screen == Screen::Preview {
                        notes.extend(
                            self.details
                                .iter()
                                .skip(self.detail_page * PAGE_SIZE)
                                .take(PAGE_SIZE)
                                .cloned(),
                        );
                    } else {
                        notes.push("Confirm restores exactly the snapshot shown. If it has changed, the restore is refused.".into());
                    }
                }
            }
            Screen::Results => {
                if let Some(p) = &self.preview {
                    notes.push(format!("Showing {} as it is now (digest {}). The earlier restore result is unchanged.",p.counts(),p.digest));
                }
                if let Some(r) = &self.restored {
                    notes.push(format!(
                        "Restore {} · {} · digest {}",
                        r.status,
                        mode(r.layout_only),
                        r.digest
                    ));
                    notes.extend(
                        self.details
                            .iter()
                            .skip(self.detail_page * PAGE_SIZE)
                            .take(PAGE_SIZE)
                            .cloned(),
                    );
                }
                if let Some(unknown) = &self.unknown {
                    notes.push(format!(
                        "{} Unknown · {}",
                        Mark::Warning.symbol(),
                        safe(unknown)
                    ));
                }
                notes.push("Started means the agent was asked to start; check it before relying on it. Items marked Needs you say what to do. Nothing is retried automatically.".into());
            }
        }
        if matches!(self.screen, Screen::Preview | Screen::Results)
            && self.details.len() > PAGE_SIZE
        {
            notes.insert(1, format!("Detail page {} of {} — all recorded items are available through Previous/Next detail page.", self.detail_page + 1, self.details.len().div_ceil(PAGE_SIZE)));
        }
        notes
    }
    pub fn render(&self, frame: &mut Frame, look: Look, body: Rect) {
        static TEMPLATE: OnceLock<surface::Template<()>> = OnceLock::new();
        static REVIEW_TEMPLATE: OnceLock<surface::Template<()>> = OnceLock::new();
        static SHEET_TEMPLATE: OnceLock<surface::Template<()>> = OnceLock::new();
        // Center's 4/5 bound needs ten body rows to leave two reading
        // rows after borders, query, status, position and footer.
        let short = matches!(self.screen, Screen::Confirm | Screen::Delete) && body.height < 10;
        let template = if short {
            SHEET_TEMPLATE.get_or_init(|| {
                picker_surface::compile(
                    FILE,
                    &MARKUP
                        .replace(" h-38", "")
                        .replace("placement=\"center\"", "placement=\"body\""),
                    picker_surface::schema(&["cursor", "label"]),
                )
            })
        } else if matches!(self.screen, Screen::Confirm | Screen::Delete) {
            REVIEW_TEMPLATE.get_or_init(|| {
                picker_surface::compile(
                    FILE,
                    &MARKUP.replace(" h-38", ""),
                    picker_surface::schema(&["cursor", "label"]),
                )
            })
        } else {
            TEMPLATE.get_or_init(|| {
                picker_surface::compile(FILE, MARKUP, picker_surface::schema(&["cursor", "label"]))
            })
        };
        let rows = self.rows();
        let state = match self.screen {
            Screen::List => &self.list,
            Screen::Save => &self.form,
            _ => &self.panel,
        };
        let mut state = state.borrow_mut();
        state.reconcile(
            rows.iter()
                .map(|r| ListRow {
                    id: r.id.clone(),
                    disabled: r.disabled,
                })
                .collect(),
        );
        let selected = state.picker.list.selected();
        let rows: Vec<_> = rows.iter().map(|r| json!({"id":r.id,"disabled":r.disabled,"label":r.label,"cursor":if selected==Some(r.id.as_str()){"›"}else{""}})).collect();
        let notes = self.notes();
        let status = if let Some(task) = &self.pending {
            match task.job {
                Job::Restore(_) => {
                    "Restoring… submitted once; closing will retain the result.".into()
                }
                Job::Save { .. } => "Saving published capture…".into(),
                Job::Remove { .. } => "Deleting named snapshot…".into(),
                _ => "Reading workspace…".into(),
            }
        } else if let Some(error) = &self.failure {
            let message = if matches!(
                error.code.as_str(),
                "WORKSPACE_HOST_UNSUPPORTED" | "WORKSPACE_SELECTION_REQUIRED"
            ) {
                "Workspace snapshots support tmux only. Herdr and standalone are unsupported; no snapshot search or driver change was attempted.".into()
            } else {
                format!("{}: {}", safe(&error.code), safe(&error.message))
            };
            format!("{message} · {}", safe(&self.notice))
        } else {
            safe(&self.notice)
        };
        let query = if self.screen == Screen::Save {
            format!("Name: {}", safe(state.picker.query().unwrap_or_default()))
        } else if short {
            let (action, name) = if self.screen == Screen::Delete {
                (
                    "Delete snapshot",
                    self.selected_entry()
                        .and_then(|entry| entry.name.as_deref()),
                )
            } else {
                (
                    "Restore snapshot",
                    self.confirmation
                        .as_ref()
                        .and_then(|selection| selection.name.as_deref()),
                )
            };
            format!("{action}: {}", safe(name.unwrap_or("Latest")))
        } else {
            String::new()
        };
        let reading = matches!(self.screen, Screen::Preview | Screen::Results) && self.form_focus;
        let footer = if self.screen == Screen::Save {
            "Tab name/actions · type/paste · Enter selects · Esc cancels"
        } else if reading {
            "↑↓ / PgUp/PgDn read · Tab actions · Esc snapshots"
        } else if matches!(self.screen, Screen::Preview | Screen::Results) {
            "↑↓ actions · Enter selects · Tab read · Esc snapshots"
        } else {
            "↑↓ / PgUp/PgDn select · Enter opens/selects · Ctrl-R refresh · Esc back"
        };
        let footer = if matches!(self.screen, Screen::Confirm | Screen::Delete) {
            let width = Modal {
                title: String::new(),
                placement: if short {
                    Placement::Body
                } else {
                    Placement::Center
                },
            }
            .areas(
                if short { frame.area() } else { body },
                [120, 38],
                true,
                false,
            )
            .footer
            .width;
            let hints = [
                ("Esc", "cancel"),
                ("Enter", "select"),
                ("↑↓", "select"),
                ("Ctrl-R", "refresh"),
            ]
            .map(|(key, description)| KeyHint {
                key: key.into(),
                description: description.into(),
            });
            fit_hints(&hints, width, " · ")
        } else {
            footer.to_owned()
        };
        let value = json!({"rows":rows,"notes":notes.iter().map(|text|json!({"id":note_id(text),"text":text})).collect::<Vec<_>>(),"query":query,"status":status,"footer":footer});
        let body = if short {
            frame.area()
        } else if matches!(self.screen, Screen::Confirm | Screen::Delete) {
            // Intrinsic sizing retains the former 38-row cap. Center's 4/5
            // placement bound reaches 38 rows in a centered 48-row body.
            let height = body.height.min(48);
            Rect::new(
                body.x,
                body.y + (body.height - height) / 2,
                body.width,
                height,
            )
        } else {
            body
        };
        state.render(FILE, template, value, frame, look, body);
        // Do not activate a restore while its exact target/mode is clipped.
        // Low terminals can scroll the preview, then resize to review confirmation.
        self.readable.set(
            matches!(self.screen, Screen::Confirm | Screen::Delete)
                && state.frame.as_ref().is_some_and(|map| {
                    map.areas.content.width >= 38
                        && notes.iter().all(|text| {
                            let id = note_id(text);
                            let height = tmt_tui::text::measure(
                                text,
                                tmt_tui::style::TextFlow::Wrap,
                                tmt_tui::geometry::Space::Cells(map.areas.content.width),
                            )[1];
                            map.hits
                                .iter()
                                .any(|hit| hit.id.last() == Some(&id) && hit.rect.height == height)
                        })
                        && map.list.as_ref().is_some_and(|list| {
                            [
                                "cancel",
                                if self.screen == Screen::Delete {
                                    "delete-confirm"
                                } else {
                                    "confirm"
                                },
                            ]
                            .iter()
                            .all(|id| {
                                list.geometry.iter().any(|r| {
                                    r.id == *id && usize::from(r.visible.height) == r.lines.len()
                                })
                            })
                        })
                }),
        );
    }
}
fn status(value: &str) -> String {
    match value {
        "created" | "merged" | "started" | "relaunched" => {
            format!("{} Applied", Mark::Done.symbol())
        }
        "skip_existing" | "skipped" | "shell" => {
            format!("{} Not applied", Mark::Offline.symbol())
        }
        _ => format!("{} Unknown", Mark::Warning.symbol()),
    }
}
pub(super) fn topology(p: &Preview) -> Vec<String> {
    let s = &p.snapshot;
    let mut notes = vec![];
    for session in &s.sessions {
        let action = p
            .sessions
            .iter()
            .find(|p| p.session == session.id)
            .map_or("unknown", |p| p.action.as_str());
        notes.push(format!(
            "Session {} ({}) · {}",
            safe(&session.name),
            safe(&session.id),
            session_action(action)
        ));
        for link in &session.windows {
            notes.push(format!(
                "  session {} window link {} → {}{}",
                safe(&session.id),
                link.index,
                safe(&link.window),
                if link.active { " · active" } else { "" }
            ));
        }
    }
    for w in &s.windows {
        notes.push(format!(
            "Window {} ({}) · {}×{} · active pane {}",
            safe(&w.name),
            safe(&w.id),
            w.width,
            w.height,
            safe(&w.active_pane)
        ));
        notes.push(format!(
            "  window {} layout: {} · visible layout: {}",
            safe(&w.id),
            safe(&w.layout),
            safe(&w.visible_layout)
        ));
    }
    for pane in &s.panes {
        let plan = p.panes.iter().find(|p| p.pane == pane.id);
        let action = plan.map_or("unknown", |p| p.action.as_str());
        notes.push(format!(
            "Pane {} · window {} index {} · {}×{} at {},{} · full-mode action: {}",
            safe(&pane.id),
            safe(&pane.window),
            pane.index,
            pane.width,
            pane.height,
            pane.left,
            pane.top,
            pane_action(action),
        ));
        notes.push(format!(
            "  pane {} cwd: {}",
            safe(&pane.id),
            safe(&pane.cwd)
        ));
        if let Some(i) = &pane.identity {
            notes.push(format!(
                "  pane {} recorded identity: {} · {} · {}",
                safe(&pane.id),
                safe(&i.name),
                safe(&i.id),
                safe(&i.lifetime)
            ));
        }
        if let Some(i) = plan.and_then(|p| p.identity.as_ref()) {
            notes.push(format!(
                "  pane {} current identity: {} · {} · {}",
                safe(&pane.id),
                safe(&i.name),
                safe(&i.id),
                safe(&i.lifetime)
            ));
        }
        if let Some(command) = &pane.command {
            // JSON preserves each argument's boundary; this is display, never executable text.
            notes.push(format!(
                "  pane {} recorded TMT argv: {}",
                safe(&pane.id),
                safe(&serde_json::to_string(&command.argv).expect("string argv"))
            ));
        }
    }
    notes
}

fn note_id(text: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest: String = Sha256::digest(text.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!("context:{digest}")
}
pub(super) fn results(r: &Restore) -> Vec<String> {
    let mut notes = vec![];
    for s in &r.sessions {
        notes.push(format!(
            "{} · session {} ({}) · {}{}{}",
            status(&s.action),
            safe(&s.name),
            safe(&s.recorded),
            s.native
                .as_deref()
                .map(safe)
                .unwrap_or_else(|| "no native ID".into()),
            s.restored_as
                .as_deref()
                .map_or(String::new(), |name| format!(
                    " · restored as {}",
                    safe(name)
                )),
            s.retained_bootstrap
                .as_ref()
                .map_or(String::new(), |id| format!(
                    " · bootstrap retained: {}",
                    safe(id)
                ))
        ));
    }
    for (kind, mappings) in [("window", &r.windows), ("pane", &r.panes)] {
        for m in mappings {
            notes.push(format!(
                "{} Applied · {kind} {} → {}",
                Mark::Done.symbol(),
                safe(&m.recorded),
                safe(&m.native)
            ));
        }
    }
    for p in &r.revival {
        notes.push(format!(
            "{} · revival {}{}{}: {}",
            status(&p.status),
            safe(&p.recorded),
            p.native
                .as_ref()
                .map_or(String::new(), |n| format!(" → {}", safe(n))),
            p.name
                .as_ref()
                .map_or(String::new(), |n| format!(" ({})", safe(n))),
            safe(&p.message)
        ));
    }
    let mut failures = std::collections::BTreeMap::<&str, usize>::new();
    for failure in &r.failures {
        *failures.entry(failure).or_default() += 1;
    }
    for (failure, count) in failures {
        notes.push(format!(
            "{} Unknown · {} ({} occurrences)",
            Mark::Warning.symbol(),
            safe(failure),
            count
        ));
    }
    notes
}

fn session_action(action: &str) -> &'static str {
    match action {
        "create" => "Create session",
        "name_in_use" => "Name in use: restore adds to or beside it",
        _ => "Unknown live state",
    }
}
fn pane_action(action: &str) -> &'static str {
    match action {
        "shell" => "Shell",
        "relaunch_command" => "Recorded command eligible",
        "resumable" => "Remembered session eligible",
        "identity_missing" => "Needs you: original identity missing",
        "no_remembered_session" => "Needs you: no remembered session",
        "stale_requires_retry" => "Needs you: stale remembered session; no automatic retry",
        "floating" => "Floating pane: not restored",
        _ => "Unknown",
    }
}
