//! Compile the disjoint production models before the shared App integration.
//! These paths remain the production owners; no test replica implements them.
#[path = "../src/board/shell/mod.rs"]
mod shell;
#[path = "../src/board/workspace/mod.rs"]
mod workspace;
