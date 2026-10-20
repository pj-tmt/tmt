//! Panel preview and save lifetime. The lane is exclusive while this picker
//! saves, so its one result cannot be mistaken for another action's outcome.
use super::{App, Effect, Request};
use crate::{
    board::{
        panels_picker::{Input, Picker},
        shell::OpsTab,
    },
    config::{DigestPanels, OverviewPanels, PanelSetting},
};
use ratatui::crossterm::event::Event;

impl App {
    pub(in crate::board) fn overview_panels(&self) -> OverviewPanels {
        self.panels_picker
            .as_ref()
            .and_then(|picker| picker.draft.overview())
            .or_else(|| {
                self.workspace_snapshot()?
                    .preferences
                    .overview_panels
                    .as_ref()
                    .map(PanelSetting::effective)
            })
            .unwrap_or_default()
    }
    pub(in crate::board) fn digest_panels(&self) -> DigestPanels {
        self.panels_picker
            .as_ref()
            .and_then(|picker| picker.draft.digest())
            .or_else(|| {
                self.workspace_snapshot()?
                    .preferences
                    .digest_panels
                    .as_ref()
                    .map(PanelSetting::effective)
            })
            .unwrap_or_default()
    }
    pub(in crate::board) fn usage_panel_hidden(&self) -> bool {
        match self.shell.tab {
            OpsTab::Agents => !self
                .overview_panels()
                .shown(crate::config::OverviewPanel::Usage),
            OpsTab::Digest => !self
                .digest_panels()
                .shown(crate::config::DigestPanel::Chart),
        }
    }
    pub(in crate::board) fn digest_placement(
        &self,
        area: ratatui::layout::Rect,
    ) -> crate::board::conversation::layout::Placement {
        use crate::board::conversation::layout::{self, Placement, Preference};
        let policy = self.digest_panels();
        let placement = if !policy.shown(crate::config::DigestPanel::List) {
            Placement {
                list: None,
                conversation: Some(area),
            }
        } else if !policy.side_shown() {
            Placement {
                list: Some(area),
                conversation: None,
            }
        } else {
            let preference = self
                .workspace_snapshot()
                .map_or(Preference::Auto, |snapshot| {
                    match snapshot.preferences.layout {
                        crate::config::OpsLayout::Auto => Preference::Auto,
                        crate::config::OpsLayout::Side => Preference::Side,
                        crate::config::OpsLayout::Stack => Preference::Stack,
                    }
                });
            layout::agents(
                area,
                preference,
                self.focused() == crate::config::Pane::Replies,
            )
        };
        self.pane_resize.panes(
            placement,
            crate::action::ResizeBoundary::DigestColumns,
            crate::action::ResizeBoundary::DigestRows,
            crate::board::conversation::layout::LIST_MIN_COLUMNS,
        )
    }
    pub(in crate::board) fn overview_usage_band(
        &self,
        area: ratatui::layout::Rect,
    ) -> ratatui::layout::Rect {
        let height = if self.usage_panel_hidden() || self.workspace_snapshot().is_none() {
            0
        } else {
            crate::board::overview::chart::host::overview_demand(
                self,
                area.width.saturating_sub(2),
                area.height,
                crate::status::now_ms(),
            )
        };
        let height = if height > 1
            && self
                .overview_panels()
                .shown(crate::config::OverviewPanel::Monitor)
            && self
                .pane_resize
                .adjusted(crate::action::ResizeBoundary::AgentsUsage)
        {
            self.pane_resize.size(
                crate::action::ResizeBoundary::AgentsUsage,
                area.height,
                height,
                height,
                crate::board::overview::composition::minimum_height(),
            )
        } else {
            height
        };
        // The band yields to the two panes: first to its one-line summary, then
        // entirely, whenever the panes would no longer both fit below it.
        let panes = crate::board::conversation::layout::LIST_MIN_ROWS
            + crate::board::conversation::layout::CONVERSATION_MIN_ROWS;
        let fits = |band: u16| area.height < panes || area.height - band >= panes;
        let height = if fits(height) {
            height
        } else if fits(1) {
            1.min(height)
        } else {
            0
        };
        ratatui::layout::Rect::new(
            area.x.saturating_add(u16::from(area.width > 0)),
            area.y,
            area.width.saturating_sub(2),
            height,
        )
    }
    pub(in crate::board) fn open_panels(&mut self) -> Effect {
        if self.theme_picker.is_some() {
            return self.say("Close Theme before opening Panels.");
        }
        if self.loading() || self.in_flight > 0 || self.panels_save_pending {
            return self.say("Wait for the pending workspace action before changing panels.");
        }
        let Some(snapshot) = self.workspace_snapshot() else {
            return self.say("The workspace has not loaded.");
        };
        if !matches!(self.shell.tab, OpsTab::Agents | OpsTab::Digest) {
            return self.say("Panels are configurable in Agents and Digest.");
        }
        self.panels_picker = Some(Picker::open(
            snapshot.preferences.clone(),
            self.shell.tab == OpsTab::Digest,
        ));
        self.menu = None;
        self.invalidate_frame_geometry();
        Effect::None
    }
    pub(super) fn panels_event(&mut self, event: &Event) -> Option<Effect> {
        let Some(input) = self.panels_picker.as_mut()?.input(event) else {
            return Some(Effect::None);
        };
        Some(match input {
            Input::Preview => {
                self.invalidate_frame_geometry();
                Effect::None
            }
            Input::Cancel => {
                self.panels_picker = None;
                self.invalidate_frame_geometry();
                Effect::None
            }
            Input::Save => {
                if self.in_flight > 0 || self.panels_save_pending {
                    return Some(Effect::None);
                }
                let picker = self.panels_picker.as_mut().expect("open picker");
                picker.start_save();
                self.panels_save_pending = true;
                Effect::Act(Request::WorkspacePreferences {
                    expected: picker.expected.clone(),
                    next: picker.next(),
                    saved: Some("Panels saved.".into()),
                })
            }
        })
    }
    pub(super) fn panels_finished(&mut self, outcome: Result<String, String>) -> bool {
        if !self.panels_save_pending {
            return false;
        }
        self.panels_save_pending = false;
        // Escape removed the disposable preview. A late result must not reopen
        // it, apply a policy locally or produce feedback for that closed surface.
        let Some(picker) = &mut self.panels_picker else {
            return true;
        };
        match outcome {
            Ok(message) => {
                self.panels_picker = None;
                self.set_notice(Some(message), false);
            }
            Err(error) => picker.save_failed(error),
        }
        self.invalidate_frame_geometry();
        true
    }
}

#[cfg(test)]
mod tests;
