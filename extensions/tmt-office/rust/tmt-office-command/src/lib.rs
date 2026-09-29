//! Public Office command grammar, handlers and core lookup port.

pub mod core_access;
pub mod grammar;
pub mod invocation;
pub mod parser;
pub mod process_core_access;
pub mod public;
pub mod service_control;

mod office_avatar_command;
mod office_block_command;
mod office_board_command;
mod office_command;
mod office_extension_command;
mod office_layout_command;
mod office_pairing_command;
mod office_profile_command;
mod office_prop_command;
mod office_storage_command;
mod office_whiteboard_command;

mod office_avatar;
mod office_block;
mod office_companion;
mod office_profile;
mod office_prop;
mod office_whiteboard;
mod office_world;

mod output {
    pub use tmt_command_output::{Failure, identity_missing};
    pub mod table {
        pub use tmt_command_output::table::write;
    }
}

pub use office_command::execute;
pub use office_companion::verify_release;

#[cfg(test)]
mod test_support;
