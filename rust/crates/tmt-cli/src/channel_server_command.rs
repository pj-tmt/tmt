//! The hidden stdio server a provider starts for a `tmt run --channel` launch.
//! The provider's driver owns the protocol; this only dispatches to it. Stdout
//! carries protocol messages only, so failures go through the shared error
//! publisher on stderr, which the provider surfaces in its own diagnostics.

use crate::{invocation::OutputMode, output::Failure};
use std::{io, path::Path};
use tmt_adapters::runtime::{RuntimeRegistry, channel::ServeRequest};
use tmt_core::binding::session::HarnessId;

pub fn execute(
    harness: &str,
    binding_id: &str,
    generation: &str,
    directory: &Path,
) -> io::Result<u8> {
    let registry = RuntimeRegistry::first_party();
    let Some(channel) = HarnessId::new(harness)
        .ok()
        .and_then(|harness| registry.channel(&harness))
    else {
        return Failure::new(
            "CHANNEL_UNSUPPORTED",
            "No message channel is registered for that agent driver.",
            1,
        )
        .publish(OutputMode::default());
    };
    let request = ServeRequest {
        binding_id,
        generation,
        directory,
    };
    let input = Box::new(io::BufReader::new(io::stdin()));
    let mut output = tmt_cli_style::stream::stdout(true);
    match channel.serve(&request, input, &mut output) {
        Ok(()) => Ok(0),
        Err(error) => Failure::new(
            "CHANNEL_SERVER_FAILED",
            "The message channel server stopped.",
            1,
        )
        .caused_by(error)
        .publish(OutputMode::default()),
    }
}
