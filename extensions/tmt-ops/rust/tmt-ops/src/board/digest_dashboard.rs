//! Existing-data Digest view. Acquisition, effects, scrolls and terminal diffing
//! remain with their existing owners; this module projects already-held values.
mod model;
mod render;

pub(super) use model::{labels, rows, summary};
pub(super) use render::render;

#[cfg(test)]
mod tests;
