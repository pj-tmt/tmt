use crate::grammar::{operand, storage};
use clap::{Arg, ArgAction, Command};

fn consent(c: Command) -> Command {
    c.arg(
        Arg::new("yes")
            .long("yes")
            .action(ArgAction::SetTrue)
            .help("Save after the effect preview without asking"),
    )
}
fn file(c: Command) -> Command {
    c.arg(
        Arg::new("file")
            .long("file")
            .required(true)
            .help("Read a bounded regular plan JSON document without following a symlink"),
    )
}

fn selection(c: Command) -> Command {
    c.arg(
        Arg::new("range")
            .long("range")
            .help("Show 1d, 7d or 30d of local calendar slots")
            .value_parser(["1d", "7d", "30d"])
            .default_value("1d"),
    )
    .arg(
        Arg::new("at")
            .long("at")
            .help("Evaluate an RFC3339 instant or local YYYY-MM-DD HH:MM"),
    )
    .arg(
        Arg::new("plan")
            .long("plan")
            .help("Filter the timeline by exact plan name or UUID"),
    )
    .arg(
        Arg::new("member")
            .long("member")
            .help("Filter canonical names case-insensitively, prefix matches first"),
    )
}
pub(in crate::grammar) fn orch() -> Command {
    let add = consent(file(storage(
        spec!("add","Preview a new plan and save only with consent",["Preview a new plan"=>"tmt orch plan add --file plan.json"]),
    )));
    let set=consent(file(storage(spec!("set","Preview replacement of a selected plan",["Replace a plan after preview"=>"tmt orch plan set Quiet --file plan.json --yes"]))).arg(operand("plan-name",true)));
    let remove=consent(storage(spec!("rm","Preview removal of a selected plan",["Preview plan removal"=>"tmt orch plan rm Quiet"])).arg(operand("plan-name",true)));
    let plans = storage(spec!(
        "plan", "Read and change orchestration plans with an effect preview",
        ["List stored plans" => "tmt orch plan ls"]
    ))
    .subcommand_required(true)
    .subcommand(
        storage(spec!(
            "ls", "List stored plans (Base always applies)",
            ["List plans as JSON" => "tmt orch plan ls --json"]
        ))
        .alias("list"),
    )
    .subcommand(
        storage(spec!(
            "show", "Read one plan by UUID or unique exact name",
            ["Read a named plan" => "tmt orch plan show Quiet"]
        ))
        .arg(operand("plan-name", true)),
    )
    .subcommand(add)
    .subcommand(set)
    .subcommand(remove);
    let preview = selection(storage(spec!(
        "preview", "Preview a candidate plan without saving or delivering",
        ["Preview a plan document" => "tmt orch preview --file plan.json"]
    )))
    .arg(
        Arg::new("file")
            .long("file")
            .required(true)
            .help("Read a bounded regular plan JSON file without following a symlink"),
    );
    let why = storage(spec!(
        "why", "Read retained receiving decision facts",
        ["Read a request's recorded decisions" => "tmt orch why req_11111111-1111-4111-8111-111111111111"]
    ))
    .arg(operand("subject", true))
    .arg(
        Arg::new("before")
            .long("before")
            .help("Read event IDs below this cursor")
            .value_parser(clap::value_parser!(u64).range(1..=tmt_core::limits::MAX_JS_SAFE_INTEGER)),
    )
    .arg(
        Arg::new("limit")
            .long("limit")
            .help("Read 1 through 100 journal entries")
            .default_value("50")
            .value_parser(clap::value_parser!(u16).range(1..=100)),
    );
    selection(storage(spec!(
        "orch", "Read orchestration slots and effective member policies",
        ["Read the current local day" => "tmt orch", "Inspect a week as JSON" => "tmt orch --range 7d --json"]
    )))
    .subcommand(plans)
    .subcommand(preview)
    .subcommand(why)
    .subcommand(consent(storage(spec!("focus","Preview personal focus above every plan, or clear it",["Focus for two hours"=>"tmt orch focus worker-1 2h --yes","End focus early"=>"tmt orch focus worker-1 off --yes"])).arg(operand("member-name",true)).arg(operand("focus-duration",true))))
}
