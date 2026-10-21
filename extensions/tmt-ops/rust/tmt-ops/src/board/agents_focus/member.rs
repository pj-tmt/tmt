//! Member identity and observed statistics; conversation retains its session.
use super::*;

pub(super) fn render(
    frame: &mut Frame,
    app: &App,
    area: Rect,
    y: &mut u16,
    row: &serde_json::Value,
    target: &OpsTarget,
    squad: Option<&OpsSquad>,
) {
    let OpsTarget::Member {
        identity_id: id, ..
    } = target
    else {
        return;
    };
    let look = app.look();
    let observed = app.references.agents.get(id);
    let host = row["pane"]["location"]["host"].as_str().map(|host| {
        row["pane"]["target"]
            .as_str()
            .filter(|target| !target.is_empty())
            .map_or_else(|| host.to_owned(), |target| format!("{host} {target}"))
    });
    line(
        frame,
        area,
        y,
        identity_line::member(
            &identity_line::Member {
                squad: squad.map_or("Common pool", |squad| squad.name.as_str()),
                lead: squad.is_some_and(|squad| squad.lead.as_deref() == Some(id)),
                runtime: observed
                    .and_then(|agent| agent.runtime.as_deref())
                    .or_else(|| observed.filter(|agent| agent.basic).map(|_| "basic")),
                model: observed
                    .and_then(|agent| agent.model.as_deref())
                    .or_else(|| row["fields"]["model"].as_str()),
                host: host.as_deref(),
            },
            look,
        ),
    );
    blank(area, y);
    let link = conversation::link_preview(app, row, &app.workspace_bindings());
    line(
        frame,
        area,
        y,
        neutral(
            format!(
                " link {}",
                link.as_ref().map_or("—", |link| link.value.as_str())
            ),
            look,
        ),
    );
    blank(area, y);
}

/// The four Measure cells of one agent: TOK 1h, STATE, DIGEST and TIMERS.
pub(in crate::board) fn tiles(
    frame: &mut Frame,
    app: &App,
    area: Rect,
    y: &mut u16,
    row: &serde_json::Value,
    target: &OpsTarget,
    squad: Option<&OpsSquad>,
) {
    let OpsTarget::Member {
        identity_id: id,
        room_id,
    } = target
    else {
        return;
    };
    let room = room_id.as_deref();
    let look = app.look();
    if area.bottom().saturating_sub(*y) >= stat_tiles::ROWS {
        let tile_area = Rect::new(area.x, *y, area.width, stat_tiles::ROWS);
        let tracks = stat_tiles::tracks(tile_area, 4);
        let reading = app.overview_member_usage(id);
        let trend = app
            .overview_member_trend::<8>(id)
            .map(|value| value.map(|n| n as u64));
        let scale = spark_row::scale(trend.into_iter()).unwrap_or(0);
        let spark = trend
            .into_iter()
            .map(|value| spark_row::glyph(value, scale))
            .collect::<String>();
        let token_value = if reading.is_some() {
            use unicode_width::UnicodeWidthStr;
            let token = token_label(reading);
            let width = tracks
                .first()
                .map_or(0, |rect| usize::from(rect.width.saturating_sub(2)));
            let gap = width.saturating_sub(token.width() + spark.width()).min(2);
            let spark = if gap > 0 { spark } else { String::new() };
            Line::from(vec![
                Span::styled(token, look.role(Role::Text)),
                Span::raw(" ".repeat(gap)),
                Span::styled(spark, look.role(Role::Muted)),
            ])
        } else {
            Line::styled("—", look.role(Role::Dim))
        };
        let detail = squad.map_or_else(
            || "—".into(),
            |squad| {
                use unicode_width::UnicodeWidthStr;
                let share = share(reading, app.overview_squad_usage(squad));
                let detail = format!("{share} of {}", squad.name);
                let width = tracks
                    .first()
                    .map_or(0, |rect| usize::from(rect.width.saturating_sub(2)));
                if detail.width() <= width {
                    detail
                } else {
                    format!("{share} {}", squad.name)
                }
            },
        );
        let mut labels = crate::board::workspace::columns::member_digest_labels(&app.labels, id);
        // Supplied modes win; otherwise keep Core's live Digest policy labels.
        let policy = labels.is_empty() && app.labels.of(id).next().is_none();
        if policy {
            labels = super::super::row_chips::of(&app.labels, row, crate::status::now_ms())
                .into_iter()
                .map(
                    |chip| crate::board::workspace::columns::WorkspaceColumnLabel {
                        text: chip.text,
                        actionable: false,
                    },
                )
                .collect();
        }
        let primary = labels.first();
        let digest = primary.map_or_else(
            || "—".into(),
            |label| {
                format!(
                    "{}{}",
                    crate::board::notes::single_line(&label.text),
                    if label.actionable { " ▾" } else { "" }
                )
            },
        );
        let detail_width = tracks
            .get(2)
            .map_or(0, |rect| usize::from(rect.width.saturating_sub(2)));
        let detail_chips: Vec<_> = labels
            .iter()
            .skip(1)
            .enumerate()
            .map(|(index, label)| crate::board::row_chips::Chip {
                text: format!(
                    "{}{}",
                    if index > 0 { " · " } else { "" },
                    crate::board::notes::single_line(&label.text)
                ),
                role: Role::Muted,
                head: false,
            })
            .collect();
        let detail_count = crate::board::row_chips::prefix(&detail_chips, detail_width);
        let savings = app.digest_savings([id.as_str()]);
        let estimate = app.compact_savings_caption(&savings, detail_width);
        let digest_detail = detail_chips[..detail_count]
            .iter()
            .map(|chip| chip.text.as_str())
            .collect::<String>();
        let timers = app.cron.counts.count(CronCountScope::Owner { id, room });
        let tiles = [
            Tile {
                label: app.tok_range().caption(),
                value: token_value,
                detail: neutral(detail, look),
            },
            Tile {
                label: "STATE",
                value: member_state(
                    app,
                    target,
                    tracks
                        .get(1)
                        .map_or(0, |rect| usize::from(rect.width.saturating_sub(2))),
                ),
                detail: Line::default(),
            },
            Tile {
                label: "DIGEST",
                value: Line::styled(
                    digest.clone(),
                    look.role(if primary.is_some() {
                        Role::Text
                    } else {
                        Role::Dim
                    }),
                ),
                detail: neutral(estimate.clone().unwrap_or(digest_detail), look),
            },
            Tile {
                label: "TIMERS",
                value: Line::styled(
                    timers.map_or_else(
                        || "—".into(),
                        |count| {
                            if count.total == 0 {
                                "none".into()
                            } else {
                                count.total.to_string()
                            }
                        },
                    ),
                    look.role(Role::Text),
                ),
                detail: timers
                    .filter(|count| count.total > 0)
                    .map_or_else(Line::default, |count| {
                        neutral(format!("{} on", count.active), look)
                    }),
            },
        ];
        let rects = stat_tiles::paint(frame.buffer_mut(), tile_area, &tiles, look);
        if let Some(rect) = rects.get(2) {
            if let Some(estimate) = &estimate
                && app.workspace_selection_visible()
            {
                app.savings_hit(
                    rect.x + 1,
                    rect.y + 2,
                    unicode_width::UnicodeWidthStr::width(estimate.as_str()),
                    rect.width.saturating_sub(2),
                );
            }
            if policy
                && super::super::row_chips::clock(&app.labels, row, crate::status::now_ms())
                    .is_some()
                && (primary.is_some_and(|label| {
                    unicode_width::UnicodeWidthStr::width(label.text.as_str())
                        <= usize::from(rect.width.saturating_sub(2))
                }))
            {
                let primary_width = primary.map_or(0, |label| {
                    unicode_width::UnicodeWidthStr::width(label.text.as_str())
                });
                let shown = primary_width
                    + if estimate.is_none() && detail_count > 0 {
                        detail_width
                    } else {
                        0
                    };
                app.frame_clocks
                    .borrow_mut()
                    .push(super::super::view::ClockSurface::Labels {
                        target: RowTarget::Workspace(target.clone()),
                        width: shown,
                    });
            }
        }
        if app.workspace_selection_visible()
            && primary.is_some_and(|label| label.actionable)
            && rects.get(2).is_some_and(|rect| {
                unicode_width::UnicodeWidthStr::width(digest.as_str())
                    <= usize::from(rect.width.saturating_sub(2))
            })
        {
            let rect = rects[2];
            app.chip_hits.borrow_mut().push((
                Rect::new(
                    rect.x + 1,
                    rect.y + 1,
                    unicode_width::UnicodeWidthStr::width(digest.as_str()) as u16,
                    1,
                ),
                app.selected,
            ));
        }
        *y += stat_tiles::ROWS;
    }
}
