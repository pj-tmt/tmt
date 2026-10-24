//! Extension-only remote door. Core is reached only by fixed public commands.
#[cfg(test)]
extern crate self as tmt_remote;
pub mod admission;
pub mod approval;
pub mod audit;
pub mod authority;
pub mod budgets;
mod calendar;
pub mod canonical;
pub mod cloud_command;
pub mod cloud_hpke;
pub mod cloud_inbox;
pub mod cloud_keys;
pub mod cloud_pairing;
pub mod control;
pub mod core;
pub mod crypto;
pub mod declaration;
pub mod deploy_cli;
pub mod deploy_command;
pub mod deploy_discovery;
pub mod deploy_firestore;
pub mod deploy_plan;
pub mod deploy_record;
pub mod deploy_run;
pub mod deploy_tools;
pub mod devices;
pub mod error;
pub mod firebase_auth;
pub mod firestore_budget;
pub mod firestore_limits;
pub mod hosting;
pub mod http;
pub mod journal;
pub mod limits;
pub mod management;
pub mod message;
pub mod mount;
pub mod native_sync;
pub mod object_service;
pub mod objects;
pub mod open;
pub mod operations;
pub mod pages;
pub mod pairing;
pub mod readiness;
pub mod reads;
pub mod remote_catalog;
pub mod remote_link;
pub mod remote_selector;
pub mod routes;
pub mod rules;
pub mod sealed_sync;
pub mod session;
pub mod settings;
mod shell;
pub mod site;
pub mod state;
pub mod store;
pub mod transport;
pub mod wire;

#[cfg(test)]
#[allow(dead_code)]
#[path = "../tests/support/core.rs"]
pub(crate) mod core_fixture;

#[cfg(test)]
#[path = "serve/test_isolation.rs"]
pub(crate) mod test_isolation;
