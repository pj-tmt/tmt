//! The binding marker as pane tokens under source `tmt`. Herdr caps a token
//! value at 80 characters, so a longer name spans numbered continuation keys.
//! Keys are not owned by their source: a marker proves nothing unless core
//! matches its IDs against storage. The token layout is the built-in Herdr
//! host's (tmt-adapters `herdr::marker`), byte for byte, so a pane bound by
//! either reads the same.

use serde_json::{Map, Value};
use tmt_driver_protocol::Marker;

pub const SOURCE: &str = "tmt";
const VALUE_CHARACTERS: usize = 80;
const NAME_PARTS: usize = 4;
const IDS: [&str; 4] = ["tmt_identity", "tmt_binding", "tmt_server", "tmt_pid"];
const NAMES: [&str; 2] = ["tmt_name", "tmt_cname"];
/// The largest pid a JSON number carries exactly, as core requires.
const MAX_PID: u64 = (1 << 53) - 1;

fn part_key(name: &str, part: usize) -> String {
    if part == 0 {
        name.into()
    } else {
        format!("{name}_{}", part + 1)
    }
}

/// Every key a marker may use, for clearing.
pub fn keys() -> Vec<String> {
    let mut keys: Vec<String> = IDS.iter().map(|key| (*key).into()).collect();
    for name in NAMES {
        keys.extend((0..NAME_PARTS).map(|part| part_key(name, part)));
    }
    keys
}

/// The marker keys a report of `tokens` does not set. Herdr merges a report
/// into the source's tokens key by key, so a shorter name must clear the
/// continuation parts a longer one left.
pub fn unset_keys(tokens: &[String]) -> Vec<String> {
    keys()
        .into_iter()
        .filter(|key| {
            !tokens
                .iter()
                .any(|token| token.split_once('=').is_some_and(|(set, _)| set == key))
        })
        .collect()
}

/// `KEY=VALUE` tokens for one report, or `None` when a name is too long for
/// the parts Herdr allows.
pub fn encode(marker: &Marker) -> Option<Vec<String>> {
    let mut tokens = vec![
        format!("tmt_identity={}", marker.identity_id),
        format!("tmt_binding={}", marker.binding_id),
        format!("tmt_server={}", marker.server_id),
        format!("tmt_pid={}", marker.pane_pid),
    ];
    for (name, value) in NAMES
        .into_iter()
        .zip([&marker.name, &marker.canonical_name])
    {
        let characters: Vec<char> = value.chars().collect();
        let parts: Vec<String> = characters
            .chunks(VALUE_CHARACTERS)
            .map(|chunk| chunk.iter().collect())
            .collect();
        if parts.is_empty() || parts.len() > NAME_PARTS {
            return None;
        }
        for (part, text) in parts.iter().enumerate() {
            tokens.push(format!("{}={text}", part_key(name, part)));
        }
    }
    Some(tokens)
}

pub fn decode(tokens: Option<&Map<String, Value>>) -> Option<Marker> {
    let tokens = tokens?;
    let text = |key: &str| {
        tokens
            .get(key)?
            .as_str()
            .filter(|text| !text.is_empty())
            .map(str::to_owned)
    };
    let name = |key: &str| {
        let mut value = text(key)?;
        for part in 1..NAME_PARTS {
            match text(&part_key(key, part)) {
                Some(next) => value.push_str(&next),
                None => break,
            }
        }
        Some(value)
    };
    let pane_pid = text("tmt_pid")?
        .parse::<u64>()
        .ok()
        .filter(|pid| (1..=MAX_PID).contains(pid))?;
    Some(Marker {
        name: name("tmt_name")?,
        canonical_name: name("tmt_cname")?,
        identity_id: text("tmt_identity")?,
        binding_id: text("tmt_binding")?,
        server_id: text("tmt_server")?,
        pane_pid,
    })
}

/// The binding ID a pane's tokens name, if any.
pub fn binding_id(tokens: Option<&Map<String, Value>>) -> Option<&str> {
    tokens?.get("tmt_binding")?.as_str()
}
