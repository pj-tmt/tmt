//! The Measure column of the Agents tab: Overview's lower modules for the selected squad, or for
//! all agents when the Agents chip is active. Each section is painted only when it fits whole.
use crate::board::{
    app::App, digest_matrix, overview::digest, overview::matrix, section_rule, workspace::OpsTarget,
};
use ratatui::{Frame, layout::Rect, text::Line};
use tmt_cli_style::Role;

const DIGEST_ROWS: u16 = 5;

pub(in crate::board) fn render(frame: &mut Frame, app: &App, area: Rect) {
    let Some(snapshot) = app.workspace_snapshot() else {
        return;
    };
    let look = app.look();
    let scene = super::super::monitor::scene(app, snapshot);
    let selected_squad = match app.workspace.selected.as_ref() {
        Some(OpsTarget::Squad(room)) => snapshot
            .roster
            .squads
            .iter()
            .find(|squad| &squad.room_id == room),
        _ => None,
    };
    let mut y = area.y;
    let bottom = area.bottom();
    // The top rule carries the Talk/Measure control.
    if area.height >= 2 {
        let top = crate::board::talk_measure::paint(
            frame.buffer_mut(),
            app,
            Rect::new(area.x, y, area.width, 1),
        );
        // The target the cells describe: the selected agent or squad, else every agent.
        let title = match app.workspace.selected.as_ref() {
            Some(_) if app.flat_agents() => "All agents".to_owned(),
            Some(target @ OpsTarget::Member { .. }) => app
                .target_row(&crate::board::app::RowTarget::Workspace(target.clone()))
                .and_then(|row| row["name"].as_str().map(str::to_owned))
                .unwrap_or_else(|| "Measure".to_owned()),
            Some(OpsTarget::Squad(_)) => {
                selected_squad.map_or_else(|| "Measure".to_owned(), |squad| squad.name.clone())
            }
            _ => "All agents".to_owned(),
        };
        section_rule::paint(
            frame.buffer_mut(),
            top,
            &crate::board::workspace::render::safe(&title),
            Line::default(),
            app.focused() == crate::config::Pane::Replies,
            look,
        );
        y += 1;
    }
    let rule = |frame: &mut Frame, y: u16, title: &str, note: Line<'static>| {
        section_rule::paint(
            frame.buffer_mut(),
            Rect::new(area.x, y, area.width, 1),
            title,
            note,
            app.focused() == crate::config::Pane::Replies,
            look,
        );
    };
    // The four summary cells: TOK 1h, STATE, DIGEST and TIMERS, label above value.
    let timers = super::lower::Band::timers(app);
    // An agent selected in the tree gets its own cells; everything else gets the aggregate.
    let member = match app.workspace.selected.as_ref() {
        Some(target @ OpsTarget::Member { room_id, .. }) if !app.flat_agents() => app
            .target_row(&crate::board::app::RowTarget::Workspace(target.clone()))
            .map(|row| {
                let squad = snapshot
                    .roster
                    .squads
                    .iter()
                    .find(|squad| Some(&squad.room_id) == room_id.as_ref());
                (target, row, squad)
            }),
        _ => None,
    };
    if let Some((target, row, squad)) = member
        && bottom.saturating_sub(y) >= crate::board::stat_tiles::ROWS
        && area.width >= 40
    {
        crate::board::agents_focus::member::tiles(
            frame,
            app,
            Rect::new(area.x, y, area.width, bottom - y),
            &mut y,
            row,
            target,
            squad,
        );
        y += 1;
    } else if bottom.saturating_sub(y) >= crate::board::stat_tiles::ROWS && area.width >= 40 {
        let tokens = match selected_squad {
            Some(squad) => app.overview_squad_usage(squad),
            None => snapshot
                .roster
                .squads
                .iter()
                .filter(|squad| {
                    app.workspace
                        .visibility
                        .shows_squad(&squad.room_id, &snapshot.preferences)
                })
                .try_fold((0u128, true), |(sum, exact), squad| {
                    app.overview_squad_usage(squad)
                        .map(|(tokens, e)| (sum + tokens, exact && e))
                }),
        };
        let tokens = tokens.map_or_else(
            || "—".to_owned(),
            |(tokens, exact)| {
                format!(
                    "{}{}",
                    if exact { "" } else { "~" },
                    crate::source::tokens(tokens)
                )
            },
        );
        let summary = &scene.summary;
        // The same tile row as an agent's cells.
        let text = |value: String| Line::styled(value, look.role(Role::Text));
        let tiles = [
            crate::board::stat_tiles::Tile {
                label: "TOK 1h",
                value: text(tokens),
                detail: Line::default(),
            },
            crate::board::stat_tiles::Tile {
                label: "STATE",
                value: text(format!(
                    "{} working · {} wait",
                    summary.working, summary.needs_you
                )),
                detail: Line::default(),
            },
            crate::board::stat_tiles::Tile {
                label: "DIGEST",
                value: text(digest::rows(app, summary)[2].key.clone()),
                detail: Line::default(),
            },
            crate::board::stat_tiles::Tile {
                label: "TIMERS",
                value: text(format!("{} jobs", timers.total())),
                detail: Line::default(),
            },
        ];
        crate::board::stat_tiles::paint(
            frame.buffer_mut(),
            Rect::new(area.x, y, area.width, crate::board::stat_tiles::ROWS),
            &tiles,
            look,
        );
        y += crate::board::stat_tiles::ROWS + 1;
    }
    // Timers: the three soonest, `c` opens all of them.
    if bottom.saturating_sub(y) >= timers.height as u16 {
        for row in 0..timers.height {
            timers.paint(
                frame,
                app,
                Rect::new(area.x, y + row as u16, area.width, 1),
                row,
                timers.height as u16,
            );
        }
        y += timers.height as u16 + 1;
    }
    // The Digest table: its five metric rows under a rule.
    if app
        .overview_panels()
        .shown(crate::config::OverviewPanel::Digest)
        && bottom.saturating_sub(y) > DIGEST_ROWS
    {
        rule(frame, y, "Digest", Line::default());
        for (row, metric) in digest::rows(app, &scene.summary).iter().enumerate() {
            metric.paint(
                frame,
                app,
                Rect::new(area.x, y + 1 + row as u16, area.width, 1),
            );
        }
        y += DIGEST_ROWS + 2;
    }
    // The Digest table by agent takes what is left; #4016 adds the real matrix later.
    if bottom.saturating_sub(y) >= 8 {
        let height = bottom.saturating_sub(y + 1);
        let matrix = matrix::Matrix::project(app, height);
        let drawn = digest_matrix::paint(
            frame.buffer_mut(),
            Rect::new(area.x, y + 1, area.width, height),
            &matrix.rows,
            look,
        );
        rule(
            frame,
            y,
            "Digest by agent",
            Line::styled(
                if drawn {
                    matrix.note
                } else {
                    "widen pane".into()
                },
                look.role(Role::Muted),
            ),
        );
    }
}
