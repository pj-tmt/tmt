//! Durable requests, exchanges and their bounded observer grammar.

use crate::grammar::{general, internal, operand, option, storage, with_options};
use clap::Command;

pub(in crate::grammar) fn talk() -> Command {
    with_options(
            general(spec!(
                "talk",
                "Send a request and wait for its durable reply",
                details = "An identity with no binding receives through its inbox: talk waits for its reply (180 seconds unless configured; --timeout overrides it), and the recipient must pull with tmt inbox or tmt x listen. Use --detach to return at once. For a bound recipient, plain talk attempts live notification; --inbox suppresses that notification and requires inbox pull. Digest queues non-owner, non-urgent automatic delivery for one checklist and returns immediately with remaining time; --urgent bypasses only Digest.",
                [
                    "Send a message and wait for the reply" => "tmt talk worker \"Run the tests\"",
                    "Send and return at once" => "tmt talk --detach worker \"Deploy when green\"",
                    "Wait for a reply without a host" => "tmt talk worker \"Review this\" --identity coordinator --timeout 30 --json",
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
        .arg(clap::Arg::new("urgent").long("urgent").action(clap::ArgAction::SetTrue).help("Bypass Digest only; all delivery guards still apply"))
        .arg(clap::Arg::new("kind").long("kind").value_parser(["decision", "review", "fyi"]).default_value("fyi").help("Checklist purpose (default: fyi)"))
        .arg(operand("target", true))
        .arg(operand("message", true))
}

pub(in crate::grammar) fn exchanges() -> Command {
    with_options(
            storage(spec!(
                "x",
                "Inspect and acknowledge exchanges",
                [
                    "List unacknowledged exchanges" => "tmt x ls",
                    "Wait for incoming inbox work" => "tmt x listen",
                ]
            )),
            &["identity", "limit", "after"],
        )
        .subcommand(with_options(
            storage(spec!(
                "ls",
                "List unacknowledged exchanges",
                [
                    "List unacknowledged exchanges" => "tmt x ls",
                    "Show at most 20" => "tmt x ls --limit 20",
                ]
            )).alias("list"),
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
        .subcommand(
            with_options(
                storage(spec!(
                    "withdraw",
                    "Withdraw your obsolete unanswered request",
                    details = "Only the recorded originator can withdraw. A reason of 1–1024 UTF-8 bytes is required. Identical retries preserve the original reason and time; a final response or different reason conflicts. Withdrawal sends no recipient notification and does not cancel work.",
                    [
                        "Withdraw an obsolete request" => "tmt x withdraw req_0f8e4b52-3c1d-4a6e-9b7f-2d5c8a1e6f30 --reason \"Already resolved\" --identity lead",
                    ]
                )),
                &["identity"],
            )
            .arg(clap::Arg::new("reason").long("reason").required(true).help("Why the request is obsolete (1–1024 UTF-8 bytes)"))
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

pub(in crate::grammar) fn reply_command() -> Command {
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

pub(in crate::grammar) fn result() -> Command {
    storage(spec!(
            "result",
            "Retrieve a retained final response",
            details = "Use the full request ID or a unique UUID prefix of at least 8 hex characters, with or without req_. Ambiguous prefixes list up to five retained candidate IDs.",
            [
                "Print a request's final response" => "tmt result req_0f8e4b52-3c1d-4a6e-9b7f-2d5c8a1e6f30",
                "Use a unique short request ID" => "tmt result 0f8e4b52",
            ]
        )).arg(operand("request-id", true))
}

pub(in crate::grammar) fn inbox() -> Command {
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

pub(in crate::grammar) fn answer() -> Command {
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

pub(in crate::grammar) fn request_observer() -> Command {
    internal(
        "__request-observer",
        "Internal bounded request timeout observer",
    )
    .hide(true)
    .arg(operand("request-id", true))
}

pub(in crate::grammar) fn reply_notice_worker() -> Command {
    internal(
        "__reply-notice-worker",
        "Internal bounded reply notice worker",
    )
    .hide(true)
    .arg(operand("batch-id", true))
    .arg(operand("log-id", true))
}
