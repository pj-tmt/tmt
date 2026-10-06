//! Object backend conformance. The same scenarios run against `LocalFs` and an
//! independently written in-memory adapter that shares no code with it, so a
//! consumer written only against `ObjectBackend` cannot depend on either.
//! `local` adds the filesystem, ledger and lease checks that only `LocalFs` has.
#[path = "objects/conformance.rs"]
mod conformance;
#[path = "objects/local.rs"]
mod local;
#[path = "objects/memory.rs"]
mod memory;
