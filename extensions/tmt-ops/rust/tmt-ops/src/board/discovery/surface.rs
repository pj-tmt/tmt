//! One shared modal/list/scroll surface for explicit candidate discovery.
use super::*;
use crate::look::Look;
use ratatui::{Frame, layout::Rect};
use serde_json::json;
use std::sync::OnceLock;
use tmt_cli_style::mark::Mark;
use tmt_tui::components::{ListRow, surface};

const FILE: &str = "ops.discovery.xml";
const MARKUP: &str = r#"<tmt-view version="1"><tmt-modal id="discovery" title="Add an agent already running" placement="center" class="w-120 h-38"><tmt-scroll id="body"><tmt-repeat each="$.notes" as="note"><tmt-text id-bind="note.id" bind="note.text" wrap="true"/></tmt-repeat><tmt-list id="choices" bind="$.rows" empty=""><tmt-row class="flex-row gap-1"><tmt-text bind="row.cursor" class="w-1 shrink-0"/><tmt-text bind="row.label" wrap="true"/></tmt-row></tmt-list></tmt-scroll><tmt-text slot="query" bind="$.query" token="accent" wrap="true"/><tmt-text slot="status" bind="$.status" token="waiting" wrap="true"/><tmt-text slot="footer" bind="$.footer" token="muted" wrap="true"/></tmt-modal></tmt-view>"#;
pub(super) struct Row {
    pub id: String,
    pub label: String,
    pub disabled: bool,
}
impl Row {
    fn new(id: impl Into<String>, label: impl Into<String>, disabled: bool) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            disabled,
        }
    }
}
fn safe(text: &str) -> String {
    super::super::notes::sanitize(&tmt_cli_style::table::escape(text))
}
fn selector(selection: &BindSelection) -> String {
    match &selection.selector {
        AgentSelector::NewName(name) => format!("New agent name: {}", safe(&format!("{name:?}"))),
        AgentSelector::ExistingUuid(id) => format!("Existing identity UUID: {}", safe(id)),
    }
}
fn selection_notes(selection: &BindSelection) -> Vec<String> {
    vec![
        format!(
            "Target: {} on {}",
            safe(&selection.key.pane),
            safe(selection.key.socket.as_deref().unwrap_or("unknown"))
        ),
        selector(selection),
        format!(
            "Keep identity: {}",
            if selection.save {
                "Saved"
            } else {
                "Temporary; an already saved identity stays saved"
            }
        ),
        format!("Reviewed scan: {}", selection.expect),
    ]
}
impl Controller {
    pub(super) fn rows(&self) -> Vec<Row> {
        let pending = self.pending.is_some();
        let mut rows = match self.screen {
            Screen::Intro => vec![Row::new("scan", "Scan this environment", pending)],
            Screen::Candidates => {
                let mut rows: Vec<_> = self
                    .observation
                    .as_ref()
                    .map(|s| {
                        s.panes
                            .iter()
                            .enumerate()
                            .map(|(i, p)| {
                                Row::new(
                                    format!("candidate:{i}"),
                                    format!(
                                        "{} · {} · {}{}",
                                        safe(&p.key.pane),
                                        safe(p.session.as_deref().unwrap_or("session unknown")),
                                        safe(
                                            p.identity
                                                .as_ref()
                                                .map_or(p.title.as_str(), |a| a.name.as_str())
                                        ),
                                        if p.adoptable {
                                            " · review"
                                        } else {
                                            " · unavailable"
                                        }
                                    ),
                                    false,
                                )
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                rows.push(Row::new(
                    "scan",
                    "Scan again",
                    self.pending.as_ref().is_some_and(|t| t.job.writes()),
                ));
                rows
            }
            Screen::Details => vec![
                Row::new(
                    "bind",
                    "Choose identity and review binding",
                    pending || self.selected_pane().is_none_or(|p| !p.adoptable),
                ),
                Row::new("back", "Back to candidates", false),
            ],
            Screen::Form => {
                let mut rows = vec![
                    Row::new(
                        "selector",
                        if self.existing {
                            "Use a new name instead"
                        } else {
                            "Link an existing identity instead"
                        },
                        pending,
                    ),
                    Row::new(
                        "lifetime",
                        if self.save {
                            "Keep identity: Saved (change)"
                        } else {
                            "Keep identity: Temporary (change)"
                        },
                        pending,
                    ),
                    Row::new("cancel", "Cancel", false),
                    Row::new("review", "Review binding", pending),
                ];
                if self.existing {
                    rows.extend(self.known.iter().take(32).map(|a| {
                        Row::new(
                            format!("known:{}", a.id),
                            format!("Choose {} · {}", safe(&a.name), a.id),
                            pending,
                        )
                    }));
                }
                rows
            }
            Screen::Confirm => vec![
                Row::new("cancel", "Cancel", false),
                Row::new("confirm", "Confirm binding", pending),
            ],
            Screen::Results => vec![
                Row::new("previous-result", "Previous result", self.result_index == 0),
                Row::new(
                    "next-result",
                    "Next result",
                    self.result_index + 1 >= self.attempts.len(),
                ),
                Row::new("scan", "Scan again", pending),
            ],
        };
        if !self.attempts.is_empty() && self.screen != Screen::Results {
            rows.push(Row::new("results", "View binding results", false));
        }
        rows.push(Row::new("close", "Close", false));
        if self.pending.as_ref().is_some_and(|t| t.job.writes()) {
            for row in &mut rows {
                row.disabled = row.id != "close";
            }
        }
        rows
    }
    pub(super) fn notes(&self) -> Vec<String> {
        let mut notes = vec![];
        match self.screen {
            Screen::Intro => {
                notes.push("Find an agent you have already started in a tmux pane. Scan first, review a candidate, then choose a name or existing identity.".into());
                notes.push("Nothing is bound until you confirm. Scanning does not install or start programs, extensions or hooks.".into());
                notes.push("Discovery supports tmux. Herdr and standalone environments show an unsupported explanation.".into());
                notes.push("tmux guide: https://github.com/pj-tmt/tmt/blob/main/site/src/chapters/drv-tmux.mdx".into());
                notes.push("Runtime and optional setup guide: https://github.com/pj-tmt/tmt/blob/main/site/src/chapters/drivers.mdx".into());
            }
            Screen::Candidates => {
                if let Some(scan) = &self.observation {
                    notes.push(format!(
                        "{} candidate panes · observed at {} ms",
                        scan.panes.len(),
                        scan.observed_at_ms
                    ));
                    for source in &scan.sources {
                        notes.push(format!(
                            "{}: {} · {}{}",
                            safe(&source.host),
                            safe(source.socket.as_deref().unwrap_or("no server selected")),
                            safe(&source.status),
                            source
                                .reason
                                .as_ref()
                                .map_or(String::new(), |r| format!(" · {}", safe(r)))
                        ));
                    }
                    if scan.truncated {
                        notes.push("Scan incomplete. Review the source explanation, then scan again; these rows cannot be bound.".into());
                    }
                    if scan.panes.is_empty() {
                        notes.push("No candidate panes were observed. Start an agent in a tmux pane, check the selected server and permissions, then scan again.".into());
                    }
                    notes.push("Runtime hints do not prove an agent is ready. Open a candidate to inspect its evidence.".into());
                }
            }
            Screen::Details => {
                if let Some(p) = self.selected_pane() {
                    notes.push(format!(
                        "Pane {} · host {} · socket {}",
                        safe(&p.key.pane),
                        safe(&p.key.host),
                        safe(p.key.socket.as_deref().unwrap_or("unknown"))
                    ));
                    notes.push(format!(
                        "Session {} · window {} · target {}",
                        safe(p.session.as_deref().unwrap_or("unknown")),
                        safe(p.window.as_deref().unwrap_or("unknown")),
                        safe(p.target.as_deref().unwrap_or("unknown"))
                    ));
                    notes.push(format!(
                        "Observation: {}{}{}",
                        safe(&p.state),
                        if p.duplicate {
                            " · duplicate occurrence"
                        } else {
                            ""
                        },
                        p.reason
                            .as_ref()
                            .map_or(String::new(), |r| format!(" · {}", safe(r)))
                    ));
                    for (label, native) in [("Server", &p.server), ("Pane process", &p.native)] {
                        notes.push(native.as_ref().map_or_else(
                            || format!("{label}: native evidence unknown"),
                            |n| format!("{label}: PID {} · start {}", n.pid, safe(&n.start)),
                        ));
                    }
                    if let Some(id) = &p.server_identity {
                        notes.push(format!("Server identity UUID: {}", safe(id)));
                    }
                    if let Some(agent) = &p.identity {
                        notes.push(format!(
                            "Verified identity: {} · UUID {} · binding {}",
                            safe(&agent.name),
                            agent.id,
                            safe(agent.binding.as_deref().unwrap_or("not reported"))
                        ));
                    }
                    notes.push(format!(
                        "Runtime: {} · hint {} · {}",
                        safe(&p.runtime_state),
                        safe(p.runtime.as_deref().unwrap_or("none")),
                        safe(p.runtime_reason.as_deref().unwrap_or("not reported"))
                    ));
                    if let Some(state) = &p.recorded_state {
                        notes.push(format!(
                            "Recorded state: {} (not current readiness)",
                            safe(state)
                        ));
                    }
                    notes.push(format!("Title: {}", safe(&p.title)));
                    notes.push(format!("Command: {}", safe(&p.command)));
                    notes.push(format!("Working directory: {}", safe(&p.cwd)));
                    if p.text_truncated {
                        notes.push("Some display text was shortened by the scan.".into());
                    }
                    notes.push(if p.adoptable {"This pane can be reviewed for binding. Binding does not start the runtime or confirm it is ready."}else{"This pane cannot be bound from this scan. Check its explanation, then scan again."}.into());
                }
            }
            Screen::Form => {
                notes.push(if self.existing {"Enter the exact lowercase UUID of an existing identity, or choose known evidence below. A name cannot link an identity."}else{"Choose a new name for this agent. A name already in use is refused, including an offline saved identity."}.into());
                notes.push("Temporary identities expire with their pane. Choose Saved to keep the identity after the pane closes.".into());
                if self.existing {
                    notes.push("Known examples below come from already observed evidence. You can enter another or offline identity UUID directly; its current state is checked when you confirm.".into());
                }
            }
            Screen::Confirm => {
                if let Some(selection) = &self.confirmation {
                    notes.extend(selection_notes(selection));
                    notes.push("Bind only this reviewed pane. If the pane or its ownership has changed, binding is refused. No program, extension or hook is installed or started.".into());
                }
            }
            Screen::Results => {
                if let Some(attempt) = self.attempts.get(self.result_index) {
                    notes.push(format!(
                        "Binding result {} of {}",
                        self.result_index + 1,
                        self.attempts.len()
                    ));
                    notes.extend(selection_notes(&attempt.selection));
                    notes.push(match &attempt.result {
                    Ok(BindReport::Bound{id,name,lifetime,already_bound,..})=>format!("{} {}: {} · UUID {} · {}",Mark::Done.symbol(),if *already_bound{"Already bound"}else{"Bound"},safe(name),id,lifetime),
                    Ok(BindReport::Refused{message,..})=>format!("{} Not applied: {}",Mark::Offline.symbol(),safe(message)),
                    Ok(BindReport::Partial{message,..})=>format!("{} Needs you: {}. Binding may have changed; check this pane before trying again.",Mark::Warning.symbol(),safe(message)),
                    Err(error)=>format!("{} Unknown: {}. Check this pane before deciding what to do next.",Mark::Warning.symbol(),safe(&error.message)),
                });
                    notes.push("Nothing is retried automatically. Binding does not confirm that the agent is ready.".into());
                }
            }
        }
        notes
    }
    pub fn render(&self, frame: &mut Frame, look: Look, body: Rect) {
        static TEMPLATE: OnceLock<surface::Template<()>> = OnceLock::new();
        let template = TEMPLATE.get_or_init(|| {
            picker_surface::compile(FILE, MARKUP, picker_surface::schema(&["label", "cursor"]))
        });
        let rows = self.rows();
        let mut ui = self.ui.borrow_mut();
        ui.reconcile(
            rows.iter()
                .map(|r| ListRow {
                    id: r.id.clone(),
                    disabled: r.disabled,
                })
                .collect(),
        );
        let selected = ui.picker.list.selected();
        let projected:Vec<_>=rows.iter().map(|r|json!({"id":r.id,"label":r.label,"disabled":r.disabled,"cursor":if selected==Some(r.id.as_str()){"›"}else{""}})).collect();
        let notes = self.notes();
        let projected_notes: Vec<_> = notes
            .iter()
            .enumerate()
            .map(|(i, text)| json!({"id":format!("note:{i}"),"text":text}))
            .collect();
        // The shared modal reserves one field row; the shared editor owns its
        // escaped cells, wrapping, cursor and pointer-to-source projection.
        let query = if self.screen == Screen::Form {
            "▏"
        } else {
            ""
        };
        let status = if let Some(task) = &self.pending {
            if task.job.writes() {
                "Binding… closing will retain the result.".into()
            } else {
                "Scanning… you can keep using the controls or cancel.".into()
            }
        } else {
            self.failure
                .as_ref()
                .map_or(String::new(), |e| safe(&e.message))
        };
        let footer = if self.screen == Screen::Form {
            "Tab input/actions · type/paste · Enter selects · Esc cancels"
        } else if matches!(self.screen, Screen::Details | Screen::Results) && self.form_focus {
            "↑↓ / PgUp/PgDn read · Tab actions · Esc candidates"
        } else {
            "↑↓ select · Enter opens/selects · Ctrl-R scan · Esc back"
        };
        ui.render(FILE,template,json!({"rows":projected,"notes":projected_notes,"query":query,"status":status,"footer":footer}),frame,look,body);
        self.editor_area.set(None);
        if self.screen == Screen::Form
            && let Some(map) = &ui.frame
        {
            let area = map.query;
            if area.width > 0 && area.height > 0 {
                let layout = self.editor.layout(area.width);
                let offset = layout.reveal(self.editor_offset.get(), area.height);
                self.editor_offset.set(offset);
                self.editor_area.set(Some(area));
                if let Some(cursor) = layout.paint(
                    frame.buffer_mut(),
                    area,
                    offset,
                    self.editor.selection(),
                    look.role(tmt_cli_style::Role::Accent),
                    look.selection(),
                ) && self.form_focus
                {
                    frame.set_cursor_position(cursor);
                }
            }
        }
        self.readable.set(
            self.screen == Screen::Confirm
                && ui.frame.as_ref().is_some_and(|map| {
                    map.areas.content.width >= 38
                        && notes.iter().enumerate().all(|(i, text)| {
                            let id = format!("note:{i}");
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
                            ["cancel", "confirm"].iter().all(|id| {
                                list.geometry.iter().any(|r| {
                                    r.id == *id && usize::from(r.visible.height) == r.lines.len()
                                })
                            })
                        })
                }),
        );
    }
}
