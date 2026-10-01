#[derive(Debug, Clone, PartialEq)]
pub enum Invocation {
    Extension {
        name: String,
        args: Vec<std::ffi::OsString>,
        help: bool,
        prefix: Vec<std::ffi::OsString>,
    },
    Help(Vec<String>),
    Version,
    Api,
    Completion(Option<String>),
    Complete(Vec<std::ffi::OsString>),
    Learn {
        skill: Option<String>,
    },
    Init,
    Run {
        name: String,
        command: Vec<std::ffi::OsString>,
        resume: bool,
        save: bool,
    },
    Resume {
        name: String,
        forget: bool,
        retry: bool,
    },
    List {
        target: Option<String>,
        room: Option<String>,
        scope: ListScope,
    },
    Bind {
        pane: Option<String>,
        name: String,
        save: bool,
    },
    BindMarked {
        name: String,
        save: bool,
    },
    Remove {
        name: String,
        force: bool,
    },
    Rename {
        old: String,
        new: String,
    },
    Whoami,
    WhoamiContext,
    Unbind,
    Talk {
        target: String,
        message: String,
        originator: Option<String>,
        options: TalkOptions,
    },
    Check {
        target: String,
        lines: Option<u64>,
    },
    /// An identity name, or a pane target (for example a previous `from`).
    Focus {
        target: String,
    },
    /// Read-only: the invoker's tmux client and the pane it shows.
    FocusClient,
    Config(ConfigRequest),
    ExtensionHooks(ExtensionHooksRequest),
    ExtensionInstall(ExtensionInstallRequest),
    Identity(IdentityRequest),
    Room(RoomOperation),
    NotesPath {
        identity: Option<String>,
    },
    Preamble(PreambleRequest),
    Role {
        identity: Option<String>,
        operation: RoleOperation,
    },
    Exchange {
        identity: Option<String>,
        operation: ExchangeOperation,
    },
    Reply {
        request_id: String,
        receipt: String,
        input: ContentInput,
    },
    Result {
        request_id: String,
    },
    /// Requests waiting on the selected identity for a final.
    Inbox {
        identity: Option<String>,
        from: Option<String>,
        limit: Option<u64>,
    },
    /// Answers what `from` waits on the selected identity for, or the
    /// explicit `request`; no receipt.
    Answer {
        identity: Option<String>,
        from: Option<String>,
        request: Option<String>,
        input: ContentInput,
    },
    Install {
        target: Option<String>,
        directory: Option<String>,
        force: bool,
    },
    Setup {
        provider: Option<String>,
        remove: bool,
        usage: tmt_core::driver::descriptor::UsageHook,
        yes: bool,
    },
    ProviderHook {
        provider: String,
        worker: bool,
    },
    RequestObserver {
        request_id: String,
    },
    Upgrade {
        channel: Option<tmt_core::native_install::Channel>,
        exact: Option<String>,
        unpin: bool,
        yes: bool,
    },
    NativeRefreshSkills,
    Uninstall {
        purge: bool,
        yes: bool,
        prefix: Option<String>,
    },
    Office {
        prefix: Option<String>,
        operation: OfficeOperation,
    },
    NativeInstall {
        product: tmt_core::native_install::Product,
        archive: String,
        manifest: String,
        prefix: String,
        channel: tmt_core::native_install::Channel,
        pin: tmt_core::native_install::PinAction,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum RoomOperation {
    Create(String),
    List,
    Show(String),
    Retire(String),
    Membership {
        room: String,
        identity: Option<String>,
        change: tmt_core::room::MembershipChange,
    },
    Dispatch {
        room: String,
        message: String,
        identity: Option<String>,
        operation_id: Option<String>,
        kind: tmt_core::request::RequestKind,
    },
}

pub use crate::office_facade::invocation::OfficeOperation;
#[cfg(test)]
pub use crate::office_facade::invocation::{
    BoardActorSelection, BoardCategorySelection, OfficeAvatarOperation, OfficeBlockOperation,
    OfficeBlockTarget, OfficeBoardOperation, OfficeLayoutOperation, OfficeProfileOperation,
    OfficePropOperation,
};

#[derive(Debug, Clone, PartialEq)]
pub struct TalkOptions {
    pub room: Option<String>,
    pub inbox: bool,
    pub force: bool,
    pub detach: bool,
    pub delay_seconds: Option<f64>,
    pub timeout_seconds: Option<f64>,
    pub no_preamble: bool,
}

pub use tmt_command_output::ContentInput;

#[derive(Debug, Clone, PartialEq)]
pub enum ExtensionInstallRequest {
    Install {
        name: String,
        prefix: Option<String>,
        channel: Option<tmt_core::native_install::Channel>,
        archive: Option<String>,
        manifest: Option<String>,
        yes: bool,
        /// Restore the exact recorded artifact, retaining the damaged release.
        repair: bool,
        /// Publish the release's agent skills without asking.
        skills: bool,
    },
    Upgrade {
        name: String,
        prefix: Option<String>,
        channel: Option<tmt_core::native_install::Channel>,
        to: Option<String>,
        unpin: bool,
        yes: bool,
    },
    Uninstall {
        name: String,
        prefix: Option<String>,
        yes: bool,
    },
    List {
        prefix: Option<String>,
        check: bool,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum ExtensionHooksRequest {
    Enable(String),
    Disable(String),
    List,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ConfigRequest {
    Show,
    Set {
        key: String,
        value: String,
        global: bool,
    },
    Clear {
        key: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum IdentityRequest {
    Create(String),
    Show(Option<String>),
    List(Vec<IdentityFilterRequest>),
    Metadata {
        identity: Option<String>,
        operation: IdentityMetadataRequest,
    },
    Status {
        identity: Option<String>,
        operation: IdentityStatusRequest,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum IdentityStatusRequest {
    Show,
    Set {
        activity: String,
        mood: Option<String>,
        ttl_ms: u64,
    },
    Clear,
}

#[derive(Debug, Clone, PartialEq)]
pub enum IdentityFilterRequest {
    Equals { key: String, value: String },
    Has(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum IdentityMetadataRequest {
    Set { key: String, value: String },
    Get { key: String },
    List,
    Remove { key: String },
}

#[derive(Debug, Clone, PartialEq)]
pub enum PreambleRequest {
    Show(Option<String>),
    Set { name: String, content: String },
    Clear(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum RoleOperation {
    Show,
    Set(ContentInput),
    Clear,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ExchangeOperation {
    List {
        limit: Option<u64>,
        after: Option<u64>,
    },
    Show {
        request_id: String,
        incoming: bool,
    },
    Ack {
        request_id: String,
        revision: u64,
        incoming: bool,
    },
    Ackall {
        incoming: bool,
    },
    Listen {
        room: Option<String>,
        timeout_seconds: f64,
        debounce_seconds: f64,
    },
}

pub use tmt_command_output::OutputMode;

#[derive(Debug, PartialEq)]
pub struct Parsed {
    pub invocation: Invocation,
    pub mode: OutputMode,
}

#[derive(Debug, PartialEq)]
pub struct ParseError {
    pub code: &'static str,
    pub message: String,
    pub mode: OutputMode,
}

/// Which identities `tmt ls` shows: one lifetime or both, only the caller's
/// tmux session, and whether offline identities with nothing to resume are
/// folded into one line.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ListScope {
    pub lifetime: Option<tmt_core::identity::Lifetime>,
    pub here: bool,
    pub all: bool,
}
