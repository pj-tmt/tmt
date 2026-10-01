//! The only core access: fixed public API calls through the invoking executable.
use crate::Result;
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::atomic::AtomicBool,
    time::{Duration, Instant},
};
use tmt_invoke::Request;

pub fn data_root(stop: &AtomicBool) -> Result<PathBuf> {
    let executable = tmt_invoke::invoking_tmt()?;
    let args = ["api".into()];
    let input = serde_json::to_vec(&json!({"version":1,"operation":"storage.root","input":{}}))?;
    let output = tmt_invoke::invoke(
        Request {
            program: &executable,
            args: &args,
            input: &input,
            deadline: Instant::now() + Duration::from_secs(5),
            max_stream_bytes: 64 * 1024,
            launch: Default::default(),
        },
        Some(stop),
    )?;
    if !output.status.success() {
        return Err(
            "Core storage.root is unavailable; use a core supporting that operation.".into(),
        );
    }
    root_from_reply(&output.stdout)
}

pub fn root_from_reply(bytes: &[u8]) -> Result<PathBuf> {
    let value: Value = serde_json::from_slice(bytes)?;
    let root = value["dataRoot"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or("Core storage.root did not report dataRoot.")?;
    let root = PathBuf::from(root);
    if !root.is_absolute() {
        return Err(crate::keyring::StateFault::RootNotAbsolute.into());
    }
    Ok(root)
}
