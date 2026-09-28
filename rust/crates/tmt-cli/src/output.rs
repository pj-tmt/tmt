//! Shared command failure and table presentation.

pub use tmt_command_output::{Failure, after_cleanup, identity_document, identity_missing};

pub mod table {
    pub use tmt_command_output::table::write;
}
