//! Typed public Office command requests.

pub use tmt_command_output::ContentInput;
pub use tmt_command_output::OutputMode;

#[derive(Debug, Clone, PartialEq)]
pub enum OfficeOperation {
    Open,
    Start {
        port: Option<u16>,
    },
    Stop,
    Status,
    Sync,
    Layout(OfficeLayoutOperation),
    Block {
        target: OfficeBlockTarget,
        identity: Option<String>,
        operation: OfficeBlockOperation,
    },
    Profile {
        identity: Option<String>,
        operation: OfficeProfileOperation,
    },
    Prop(OfficePropOperation),
    Avatar(OfficeAvatarOperation),
    ExtensionValidate {
        file: String,
        instance: String,
    },
    Board(OfficeBoardOperation),
    WhiteboardSnapshot {
        reference: String,
        output: Option<String>,
    },
    Unpair {
        world: String,
        identity: Option<String>,
        emulator: bool,
    },
    Inspect {
        world: String,
        identity: Option<String>,
        emulator: bool,
    },
    Pair {
        world: String,
        identity: Option<String>,
        emulator: bool,
        read_only: bool,
        timeout_seconds: u64,
    },
    PairStatus {
        world: String,
        identity: Option<String>,
        emulator: bool,
    },
    Install {
        yes: bool,
        force: bool,
        archive: Option<String>,
        manifest: Option<String>,
        channel: Option<tmt_core::native_install::Channel>,
    },
    Upgrade {
        force: bool,
        channel: Option<tmt_core::native_install::Channel>,
    },
    Uninstall {
        yes: bool,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum BoardActorSelection {
    Owner,
    Identity(Option<String>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum BoardCategorySelection {
    General,
    Repository(String),
    Room(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum OfficeBoardOperation {
    Post {
        category: BoardCategorySelection,
        actor: BoardActorSelection,
        title: String,
        body: ContentInput,
        operation_id: Option<String>,
    },
    List {
        category: BoardCategorySelection,
        view: String,
        author_id: Option<String>,
        owner: bool,
        since: Option<String>,
        limit: u32,
        cursor: Option<String>,
    },
    Show {
        thread_id: String,
        reply_limit: u32,
        reply_cursor: Option<String>,
    },
    Reply {
        thread_id: String,
        actor: BoardActorSelection,
        body: ContentInput,
        operation_id: Option<String>,
    },
    Edit {
        entry_id: String,
        actor: BoardActorSelection,
        title: Option<String>,
        body: Option<ContentInput>,
        if_revision: u64,
        operation_id: Option<String>,
    },
    Delete {
        entry_id: String,
        actor: BoardActorSelection,
        moderate: bool,
        if_revision: u64,
        operation_id: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct OfficeBlockTarget {
    pub world: String,
    pub emulator: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum OfficeLayoutOperation {
    Show,
    Apply {
        file: String,
        if_revision: u64,
        legacy_basis: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum OfficeBlockOperation {
    Show {
        block_id: Option<String>,
    },
    Apply {
        block_id: Option<String>,
        file: String,
        if_revision: u64,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum OfficeProfileOperation {
    Show,
    Apply { file: String, if_revision: u64 },
}

#[derive(Debug, Clone, PartialEq)]
pub enum OfficePropOperation {
    Validate { file: String },
    Preview { file: String },
    Install { file: String, if_revision: u64 },
    Remove { digest: String, if_revision: u64 },
    List { limit: u64, cursor: Option<String> },
    Show { digest: String },
}

#[derive(Debug, Clone, PartialEq)]
pub enum OfficeAvatarOperation {
    Validate { file: String },
    Preview { file: String },
    Install { file: String, if_revision: u64 },
    Remove { digest: String, if_revision: u64 },
    List { limit: u64, cursor: Option<String> },
    Show { digest: String },
}
