mod common;
mod configuration;
mod extension;
mod guidance;
mod identity;
mod messaging;
mod naming;
mod native_install;
mod notes;
mod office;
mod profile;
mod room;
mod run;
mod setup;

use crate::invocation::IdentityStatusRequest;
use std::ffi::OsString;
use tmt_core::driver::descriptor::UsageHook;

use super::{
    ContentInput, ExchangeOperation, IdentityFilterRequest, IdentityMetadataRequest,
    IdentityRequest, Invocation, OutputMode, ParseError, Parsed, RoleOperation, TalkOptions, parse,
};

fn args(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}

fn parsed(values: &[&str]) -> Parsed {
    parse(&args(values)).unwrap_or_else(|error| {
        panic!("expected {:?} to parse, got {error:?}", values);
    })
}

fn parse_error(values: &[&str]) -> ParseError {
    parse(&args(values)).expect_err("expected arguments to be rejected")
}

fn assert_usage_error(values: &[&str], message: &str, mode: OutputMode) {
    let error = parse_error(values);
    assert_eq!(error.code, "USAGE_ERROR", "arguments: {values:?}");
    assert_eq!(error.mode, mode, "arguments: {values:?}");
    assert!(
        error.message.contains(message),
        "arguments: {values:?}; expected message containing {message:?}, got {:?}",
        error.message
    );
}
