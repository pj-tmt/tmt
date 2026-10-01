//! Child-only enrollment locators; the private record remains the authority.
use super::record::Record;
use std::ffi::OsString;

pub const BINDING_ENV: &str = "TMT_CODEX_CHANNEL_BINDING";
pub const GENERATION_ENV: &str = "TMT_CODEX_CHANNEL_GENERATION";
pub fn environment(record: &Record) -> Vec<(OsString, OsString)> {
    vec![
        (BINDING_ENV.into(), record.binding_id.clone().into()),
        (GENERATION_ENV.into(), record.generation.clone().into()),
    ]
}
