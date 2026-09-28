//! Office data contracts and pure codecs, below persistence and runtime adapters.

pub mod codec;
pub mod indexed_art;
pub mod json_integer;

pub mod office_art_reference;
pub mod office_block;
pub mod office_board;
pub mod office_extension;
pub mod office_map;
pub mod office_profile;
pub mod office_protocol;
pub mod office_whiteboard;
pub mod office_world;

#[cfg(test)]
mod office_block_tests;
