//! The two-line summary an author writes on a structured page value (`shell`, `work`):
//! `{line1,line2,attribution}`. Not `summary.rs`, which derives the owner list's display counts.
//! The admitted writer supplies attribution; an action body never does.
use crate::closed::{Invalid, Result, id, integer, keys, plain, require, text};
use serde_json::Value;
use tmt_colab_model::values;

pub use crate::limits::{SUMMARY_LINE1_POINTS, SUMMARY_LINE2_POINTS};

/// The two lines of an action body or a stored summary, each within its code-point limit.
pub fn lines(value: &Value) -> Result<()> {
    for (key, limit) in [
        ("line1", SUMMARY_LINE1_POINTS),
        ("line2", SUMMARY_LINE2_POINTS),
    ] {
        let line = plain(&value[key], limit * 4)?;
        require(line.chars().count() <= limit)?;
    }
    Ok(())
}
/// Agent details are display-only Core metadata, never proof of agent authority.
pub fn validate_attribution(value: &Value) -> Result<()> {
    keys(value, &["writerId", "timeMs"], &["displayAgent"])?;
    id(&value["writerId"])?;
    integer(&value["timeMs"])?;
    if let Some(agent) = value.get("displayAgent") {
        keys(agent, &["name"], &["id"])?;
        require(crate::decoder::valid_publisher_agent(text(&agent["name"])?))?;
        if let Some(agent_id) = agent.get("id") {
            values::core_id(text(agent_id)?).map_err(|_| Invalid)?;
        }
    }
    Ok(())
}
/// A stored summary: both lines and the system attribution.
pub fn validate(value: &Value) -> Result<()> {
    keys(value, &["line1", "line2", "attribution"], &[])?;
    lines(value)?;
    validate_attribution(&value["attribution"])
}
/// The summary an admitted `summary.set` action stores: its two lines plus the write owner's
/// attribution.
pub fn set(body: &Value, attribution: &Value) -> Result<Value> {
    keys(body, &["line1", "line2"], &[])?;
    lines(body)?;
    validate_attribution(attribution)?;
    Ok(serde_json::json!({"line1":body["line1"],"line2":body["line2"],"attribution":attribution}))
}
