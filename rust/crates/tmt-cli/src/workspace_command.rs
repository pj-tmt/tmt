//! Advisory command refresh, after the command's own output has been flushed.

use crate::invocation::{
    ExchangeOperation, IdentityMetadataRequest, IdentityRequest, IdentityStatusRequest, Invocation,
    PreambleRequest, RoleOperation, RoomOperation,
};
use std::{io::Write, time::Instant};
use tmt_adapters::{
    config::ConfigPaths,
    host::{CallerEnvironment, Host},
    workspace,
};

/// A command that changes state or talks to a pane keeps the snapshot fresh. A
/// pure read never makes its caller wait for the optional capture, whatever its
/// output mode: extensions call these on their hot paths. A new nested
/// operation must choose its side here.
pub(crate) fn eligible(invocation: &Invocation) -> bool {
    match invocation {
        Invocation::Talk { .. }
        | Invocation::Reply { .. }
        | Invocation::Answer { .. }
        | Invocation::Check { .. }
        | Invocation::Stop { .. }
        | Invocation::Focus { .. } => true,
        Invocation::Identity(request) => match request {
            IdentityRequest::Create(_) => true,
            IdentityRequest::Show(_) | IdentityRequest::List(_) => false,
            IdentityRequest::Metadata { operation, .. } => match operation {
                IdentityMetadataRequest::Apply
                | IdentityMetadataRequest::Set { .. }
                | IdentityMetadataRequest::Remove { .. } => true,
                IdentityMetadataRequest::Get { .. } | IdentityMetadataRequest::List => false,
            },
            IdentityRequest::Status { operation, .. } => match operation {
                IdentityStatusRequest::Set { .. } | IdentityStatusRequest::Clear => true,
                IdentityStatusRequest::Show => false,
            },
        },
        Invocation::Room(operation) => match operation {
            RoomOperation::Create(_)
            | RoomOperation::Retire(_)
            | RoomOperation::Membership { .. }
            | RoomOperation::Dispatch { .. } => true,
            RoomOperation::List | RoomOperation::Show(_) => false,
        },
        Invocation::Preamble(request) => match request {
            PreambleRequest::Set { .. } | PreambleRequest::Clear(_) => true,
            PreambleRequest::Show(_) => false,
        },
        Invocation::Role { operation, .. } => match operation {
            RoleOperation::Set(_) | RoleOperation::Clear => true,
            RoleOperation::Show => false,
        },
        Invocation::Exchange { operation, .. } => match operation {
            ExchangeOperation::Ack { .. }
            | ExchangeOperation::Ackall { .. }
            | ExchangeOperation::Withdraw { .. } => true,
            ExchangeOperation::List { .. }
            | ExchangeOperation::Show { .. }
            | ExchangeOperation::Listen { .. } => false,
        },
        _ => false,
    }
}

pub(crate) fn refresh(output: &mut impl Write) {
    after_output(output, || {
        let caller = CallerEnvironment::current();
        // No configuration or filesystem effects outside a candidate tmux pane.
        if !caller
            .pane_locators()
            .iter()
            .any(|(kind, _, socket)| *kind == tmt_core::host::HostKind::Tmux && socket.is_some())
        {
            return;
        }
        if let Ok(paths) = ConfigPaths::discover() {
            let host = Host::for_caller(&caller);
            let _ = workspace::refresh_command(
                &paths,
                &host,
                &caller,
                Instant::now() + workspace::CAPTURE_BUDGET,
            );
        }
    });
}

fn after_output(output: &mut impl Write, refresh: impl FnOnce()) {
    if output.flush().is_ok() {
        refresh();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;

    struct Output {
        events: std::rc::Rc<std::cell::RefCell<Vec<&'static str>>>,
        fail: bool,
    }
    impl Write for Output {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            self.events.borrow_mut().push("flush");
            if self.fail {
                Err(io::Error::other("closed output"))
            } else {
                Ok(())
            }
        }
    }

    #[test]
    fn refresh_follows_flush_and_closed_output_skips_optional_work() {
        let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let mut output = Output {
            events: events.clone(),
            fail: false,
        };
        after_output(&mut output, || {
            assert_eq!(*events.borrow(), ["flush"]);
            events.borrow_mut().push("capture");
        });
        assert_eq!(*events.borrow(), ["flush", "capture"]);
        output.fail = true;
        after_output(&mut output, || {
            panic!("failed output cannot trigger refresh")
        });
        assert_eq!(*events.borrow(), ["flush", "capture", "flush"]);
    }

    fn text(value: &str) -> String {
        value.to_owned()
    }

    #[test]
    fn only_state_changes_and_pane_talk_refresh_and_pure_reads_never_wait() {
        use crate::invocation::{ContentInput, ListScope, TalkOptions};
        let talk = TalkOptions {
            urgent: false,
            passthrough: false,
            digest_kind: tmt_core::request::digest::DigestKind::Fyi,
            room: None,
            force: false,
            detach: false,
            delay_seconds: None,
            timeout_seconds: None,
            no_preamble: false,
        };
        let input = || ContentInput::Inline(text("body"));
        let metadata = |operation| {
            Invocation::Identity(IdentityRequest::Metadata {
                identity: None,
                operation,
            })
        };
        let status = |operation| {
            Invocation::Identity(IdentityRequest::Status {
                identity: None,
                operation,
            })
        };
        let exchange = |operation| Invocation::Exchange {
            identity: None,
            operation,
        };
        let refreshing = [
            Invocation::Talk {
                target: text("seat"),
                message: text("hello"),
                originator: None,
                options: talk,
            },
            Invocation::Reply {
                request_id: text("req"),
                receipt: text("receipt"),
                input: input(),
            },
            Invocation::Answer {
                identity: None,
                from: None,
                request: None,
                input: input(),
            },
            Invocation::Check {
                target: text("seat"),
                lines: None,
                capture_only: false,
            },
            Invocation::Focus {
                target: text("seat"),
                into: None,
            },
            Invocation::Identity(IdentityRequest::Create(text("seat"))),
            metadata(IdentityMetadataRequest::Apply),
            metadata(IdentityMetadataRequest::Set {
                key: text("k"),
                value: text("v"),
            }),
            metadata(IdentityMetadataRequest::Remove { key: text("k") }),
            status(IdentityStatusRequest::Set {
                activity: text("working"),
                mood: None,
                ttl_ms: 1,
            }),
            status(IdentityStatusRequest::Clear),
            Invocation::Room(RoomOperation::Create(text("room"))),
            Invocation::Room(RoomOperation::Retire(text("room"))),
            Invocation::Room(RoomOperation::Membership {
                room: text("room"),
                identity: None,
                change: tmt_core::room::MembershipChange::Join,
            }),
            Invocation::Room(RoomOperation::Dispatch {
                room: text("room"),
                message: text("hello"),
                identity: None,
                operation_id: None,
                kind: tmt_core::request::RequestKind::Request,
            }),
            Invocation::Preamble(PreambleRequest::Set {
                name: text("p"),
                content: text("c"),
            }),
            Invocation::Preamble(PreambleRequest::Clear(text("p"))),
            Invocation::Role {
                identity: None,
                operation: RoleOperation::Set(input()),
            },
            Invocation::Role {
                identity: None,
                operation: RoleOperation::Clear,
            },
            exchange(ExchangeOperation::Ack {
                request_id: text("req"),
                revision: 1,
                incoming: false,
            }),
            exchange(ExchangeOperation::Ackall { incoming: false }),
            exchange(ExchangeOperation::Withdraw {
                request_id: text("req"),
                reason: text("why"),
            }),
        ];
        for invocation in &refreshing {
            assert!(
                eligible(invocation),
                "{invocation:?} keeps the snapshot fresh"
            );
        }
        let reads = [
            Invocation::Whoami,
            Invocation::WhoamiContext,
            Invocation::FocusClient,
            Invocation::List {
                target: None,
                room: None,
                scope: ListScope::default(),
            },
            Invocation::Result {
                request_id: text("req"),
            },
            Invocation::Inbox {
                identity: None,
                from: None,
                limit: None,
            },
            Invocation::Identity(IdentityRequest::Show(None)),
            Invocation::Identity(IdentityRequest::List(Vec::new())),
            metadata(IdentityMetadataRequest::Get { key: text("k") }),
            metadata(IdentityMetadataRequest::List),
            status(IdentityStatusRequest::Show),
            Invocation::Room(RoomOperation::List),
            Invocation::Room(RoomOperation::Show(text("room"))),
            Invocation::Preamble(PreambleRequest::Show(None)),
            Invocation::Role {
                identity: None,
                operation: RoleOperation::Show,
            },
            exchange(ExchangeOperation::List {
                limit: None,
                after: None,
            }),
            exchange(ExchangeOperation::Show {
                request_id: text("req"),
                incoming: false,
            }),
        ];
        for invocation in &reads {
            assert!(
                !eligible(invocation),
                "{invocation:?} is a read and must not wait for the capture"
            );
        }
    }

    #[test]
    fn protocols_and_event_owners_never_refresh() {
        assert!(!eligible(&Invocation::Api));
        assert!(!eligible(&Invocation::Mcp {
            identity: "seat".into()
        }));
        assert!(!eligible(&Invocation::Bind {
            pane: None,
            name: "seat".into(),
            save: true
        }));
        assert!(!eligible(&Invocation::Help(Vec::new())));
        assert!(!eligible(&Invocation::Exchange {
            identity: None,
            operation: ExchangeOperation::Listen {
                room: None,
                timeout_seconds: 1.0,
                debounce_seconds: 0.0
            }
        }));
    }
}
