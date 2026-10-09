//! Advisory command refresh, after the command's own output has been flushed.

use crate::invocation::{ExchangeOperation, Invocation};
use std::{io::Write, time::Instant};
use tmt_adapters::{
    config::ConfigPaths,
    host::{CallerEnvironment, Host},
    workspace,
};

pub(crate) fn eligible(invocation: &Invocation) -> bool {
    matches!(
        invocation,
        Invocation::Talk { .. }
            | Invocation::Reply { .. }
            | Invocation::Answer { .. }
            | Invocation::Check { .. }
            | Invocation::List { .. }
            | Invocation::Result { .. }
            | Invocation::Inbox { .. }
            | Invocation::Whoami
            | Invocation::WhoamiContext
            | Invocation::Identity(_)
            | Invocation::Room(_)
            | Invocation::Focus { .. }
            | Invocation::FocusClient
            | Invocation::Preamble(_)
            | Invocation::Role { .. }
            | Invocation::Exchange {
                operation: ExchangeOperation::List { .. }
                    | ExchangeOperation::Show { .. }
                    | ExchangeOperation::Ack { .. }
                    | ExchangeOperation::Ackall { .. }
                    | ExchangeOperation::Withdraw { .. },
                ..
            }
    )
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

    #[test]
    fn finite_inspection_is_eligible_but_protocols_and_event_owners_are_not() {
        assert!(eligible(&Invocation::Whoami));
        assert!(eligible(&Invocation::Inbox {
            identity: None,
            from: None,
            limit: None
        }));
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
