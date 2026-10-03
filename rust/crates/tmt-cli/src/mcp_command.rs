//! Same-user stdio tools compose the existing command and API owners.

use crate::{
    answer_command, exchange_command, identity_command,
    identity_context::{self, Selector},
    invocation::OutputMode,
    output::{Failure, after_cleanup},
    response_command,
};
use serde_json::Value;
use std::io;
use tmt_adapters::{
    api,
    config::ConfigPaths,
    mcp::{self, ToolCall},
    storage::Storage,
};

fn unavailable(error: impl std::error::Error + 'static) -> Failure {
    Failure::new(
        "MCP_UNAVAILABLE",
        "The local MCP server could not continue.",
        1,
    )
    .caused_by(error)
}

fn identity(paths: &ConfigPaths, selector: Selector) -> Result<String, Failure> {
    let mut storage = Storage::open(&paths.database).map_err(unavailable)?;
    let pending = identity_context::resolve(&mut storage, selector).and_then(|identity| {
        if identity.lifetime != tmt_core::identity::Lifetime::Saved {
            Err(Failure::new(
                "MCP_SAVED_IDENTITY_REQUIRED",
                "MCP requires an existing saved identity.",
                1,
            ))
        } else {
            Ok(identity.id)
        }
    });
    after_cleanup(pending, || storage.close())
}

fn call(paths: &ConfigPaths, bound: &str, tool: ToolCall) -> Result<Value, Value> {
    identity(paths, Selector::SavedId(bound.into())).map_err(|error| error.document())?;
    match tool {
        ToolCall::List => identity_command::list_document(paths).map_err(|error| error.document()),
        ToolCall::Inbox { from, limit } => {
            answer_command::inbox_document(paths, bound, from, limit)
                .map_err(|error| error.document())
        }
        ToolCall::Request(id) => {
            exchange_command::request_document(paths, bound, id).map_err(|error| error.document())
        }
        ToolCall::Result(id) => {
            response_command::result_document(paths, id).map_err(|error| error.document())
        }
        ToolCall::Operation(id) => api::execute(paths, api::Request::Receipt(id))
            .map_err(|error| {
                serde_json::from_slice::<Value>(&error.encode()).expect("API error is JSON")
            })
            .map(|bytes| serde_json::from_slice(&bytes).expect("API receipt is JSON")),
    }
}

pub fn execute(selected: &str) -> io::Result<u8> {
    let ready = ConfigPaths::discover()
        .map_err(unavailable)
        .and_then(|paths| {
            // Named local selection happens once; every later call uses exact UUID lookup.
            identity(&paths, Selector::Explicit(selected.into())).map(|bound| (paths, bound))
        });
    let (paths, bound) = match ready {
        Ok(ready) => ready,
        Err(error) => return error.publish(OutputMode::default()),
    };
    match mcp::serve(|tool| call(&paths, &bound, tool)) {
        Ok(()) => Ok(0),
        Err(error) => unavailable(error).publish(OutputMode::default()),
    }
}
