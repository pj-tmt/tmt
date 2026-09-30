//! Durable requests, exchanges and their bounded observer grammar.

use super::{general, internal, operand, option, storage, with_options};
use clap::Command;

pub(super) fn talk() -> Command {
    with_options(
            general(spec!(
                "talk",
                "Send a request and wait for its durable reply",
                [
                    "Send a message and wait for the reply" => "tmt talk worker \"Run the tests\"",
                    "Send and return at once" => "tmt talk --detach worker \"Deploy when green\"",
                    "Queue for an identity with no pane" => "tmt talk --inbox worker \"Review when free\"",
                ]
            )),
            &[
                "force",
                "delay",
                "detach",
                "timeout",
                "no-preamble",
                "identity",
                "inbox",
                "room",
            ],
        )
        .visible_alias("send")
        .arg(operand("target", true))
        .arg(operand("message", true))
}

pub(super) fn exchanges() -> Command {
    with_options(
            storage(spec!(
                "x",
                "Inspect and acknowledge exchanges",
                [
                    "List unacknowledged exchanges" => "tmt x list",
                    "Wait for incoming inbox work" => "tmt x listen",
                ]
            )),
            &["identity", "limit", "after"],
        )
        .subcommand(with_options(
            storage(spec!(
                "list",
                "List unacknowledged exchanges",
                [
                    "List unacknowledged exchanges" => "tmt x list",
                    "Show at most 20" => "tmt x list --limit 20",
                ]
            )),
            &["identity", "limit", "after"],
        ))
        .subcommand(
            with_options(
                storage(spec!(
                    "show",
                    "Show retained exchange content",
                    [
                        "Show one exchange" => "tmt x show req_0f8e4b52-3c1d-4a6e-9b7f-2d5c8a1e6f30",
                    ]
                )),
                &["identity", "incoming"],
            )
            .arg(operand("request-id", true)),
        )
        .subcommand(
            with_options(
                storage(spec!(
                    "ack",
                    "Acknowledge an observed revision",
                    [
                        "Acknowledge the revision you read" => "tmt x ack --revision 3 req_0f8e4b52-3c1d-4a6e-9b7f-2d5c8a1e6f30",
                    ]
                )),
                &["identity", "incoming"],
            )
            .arg(option("revision").required(true))
            .arg(operand("request-id", true)),
        )
        .subcommand(with_options(
            storage(spec!(
                "ackall",
                "Acknowledge the current identity snapshot",
                [
                    "Acknowledge everything shown so far" => "tmt x ackall",
                ]
            )),
            &["identity", "incoming"],
        ))
        .subcommand(with_options(
            storage(spec!(
                "listen",
                "Wait for incoming inbox activity",
                [
                    "Wait for new inbox work" => "tmt x listen",
                    "Give up after ten minutes" => "tmt x listen --timeout 10m",
                ]
            )),
            &["identity", "room", "timeout", "debounce"],
        ))
}

pub(super) fn reply() -> Command {
    with_options(
            storage(spec!(
                "reply",
                "Submit an exact final response",
                [
                    "Reply with a message" => "tmt reply req_0f8e4b52-3c1d-4a6e-9b7f-2d5c8a1e6f30 --receipt v2_7OxG2uwfdAFFMd0qNWJDnA --message \"Done: tests pass\"",
                    "Reply with a file's content" => "tmt reply req_0f8e4b52-3c1d-4a6e-9b7f-2d5c8a1e6f30 --receipt v2_7OxG2uwfdAFFMd0qNWJDnA --file answer.md",
                ]
            )),
            &["file", "message", "stdin"],
        )
        .arg(option("receipt").required(true))
        .arg(operand("request-id", true))
}

pub(super) fn result() -> Command {
    storage(spec!(
            "result",
            "Retrieve a retained final response",
            [
                "Print a request's final response" => "tmt result req_0f8e4b52-3c1d-4a6e-9b7f-2d5c8a1e6f30",
            ]
        )).arg(operand("request-id", true))
}

pub(super) fn inbox() -> Command {
    with_options(
        storage(spec!(
            "inbox",
            "List requests waiting on you for a final response",
            details = "A request leaves only when it has a final response or its acceptance deadline passes; acknowledging it does not remove it.",
            [
                "What is waiting on you" => "tmt inbox",
                "Only from one identity" => "tmt inbox --from reviewer",
                "As a named identity outside its pane" => "tmt inbox --identity ben",
            ]
        )),
        &["identity", "limit", "from"],
    )
}

pub(super) fn answer() -> Command {
    with_options(
            storage(spec!(
                "answer",
                "Answer the request an identity is waiting on you for",
                details = "With several open requests from that identity, nothing is sent until you choose one with --request. With --request the sender may be omitted, as for an anonymous one. If a request gave you a receipt, use tmt reply.",
                [
                    "Answer with a message" => "tmt answer reviewer \"Yes, ship it\"",
                    "Choose one of several open requests" => "tmt answer reviewer \"Yes\" --request req_0f8e4b52-3c1d-4a6e-9b7f-2d5c8a1e6f30",
                    "Answer a request by ID, also one from an anonymous sender" => "tmt answer --request req_0f8e4b52-3c1d-4a6e-9b7f-2d5c8a1e6f30 \"Done\"",
                ]
            )),
            &["identity", "request", "file", "stdin"],
        )
        .arg(operand("from", false))
        .arg(operand("content", false))
}

pub(super) fn request_observer() -> Command {
    internal(
        "__request-observer",
        "Internal bounded request timeout observer",
    )
    .hide(true)
    .arg(operand("request-id", true))
}
