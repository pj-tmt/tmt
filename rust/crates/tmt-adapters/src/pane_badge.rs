//! Post-commit cosmetic projection. Never an input to identity or routing.

use crate::{
    config::{ConfigFiles, ConfigPaths},
    process::CommandRunner,
    storage::Storage,
    tmux::{PaneCosmetics, Tmux},
};
use std::time::{Duration, Instant};
use tmt_core::{binding::Binding, settings::PaneBadge};

pub fn refresh<R: CommandRunner>(
    paths: &ConfigPaths,
    tmux: &Tmux<R>,
    expected: &Binding,
    deadline: Instant,
) {
    // Leave the hook supervisor time to publish its already-rendered context.
    let Some(deadline) = deadline.checked_sub(Duration::from_millis(100)) else {
        return;
    };
    if Instant::now() >= deadline {
        return;
    }
    let Ok(settings) = (ConfigFiles {
        paths: paths.clone(),
    })
    .load() else {
        return;
    };
    let Ok(Some(context)) = Storage::context_by_pane(
        &paths.database,
        &expected.pane_id,
        &expected.server.server_id,
        crate::request_runtime::wall_time_ms(),
    ) else {
        return;
    };
    let Some(binding) = context
        .entry
        .binding
        .as_ref()
        .filter(|binding| binding.id == expected.id)
    else {
        return;
    };
    let cosmetics = PaneCosmetics::Bound {
        identity: &context.entry.identity,
        badge: settings.settings.pane_badge == PaneBadge::On,
    };
    let _ = tmux.update_binding_cosmetics_until(binding, cosmetics, deadline);
}
