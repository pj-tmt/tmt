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
    List {
        target: Option<String>,
        room: Option<String>,
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
    Config(ConfigRequest),
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
    Install {
        target: Option<String>,
        directory: Option<String>,
        force: bool,
    },
    Setup {
        provider: Option<String>,
        remove: bool,
        yes: bool,
    },
    ProviderHook {
        worker: bool,
    },
    Upgrade {
        channel: Option<tmt_core::native_install::Channel>,
        exact: Option<String>,
        unpin: bool,
    },
    NativeRefreshSkills,
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

#[derive(Debug, Clone, PartialEq)]
pub enum ContentInput {
    Inline(String),
    File(String),
    Stdin,
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

#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct OutputMode {
    pub json: bool,
}

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
