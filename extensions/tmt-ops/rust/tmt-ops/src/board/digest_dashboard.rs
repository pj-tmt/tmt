//! Existing-data Digest view. Acquisition, effects, scrolls and terminal diffing
//! remain with their existing owners; this module projects already-held values.
mod model;
mod render;

/// Session scope uses exactly the admitted Digest identities, including its
/// current search, while saved hiding remains an Agents/Overview preference.
pub(super) fn summary(app: &crate::board::app::App) -> String {
    if app.workspace.visibility.scope.is_empty() {
        return model::summary(&app.labels, None);
    }
    let rows = app.workspace_rows();
    let identities = rows
        .iter()
        .filter_map(|row| match &row.target {
            crate::board::workspace::OpsTarget::Member { identity_id, .. } => {
                Some(identity_id.as_str())
            }
            _ => None,
        })
        .collect();
    model::summary(&app.labels, Some(&identities))
}
pub(super) use model::{percent_text, savings_caption, savings_start, savings_value, usd_text};
pub(super) use render::render;

#[cfg(test)]
pub(in crate::board) mod tests;
