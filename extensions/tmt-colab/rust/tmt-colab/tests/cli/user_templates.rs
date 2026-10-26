//! User templates (#5066): a page of the space saved as a template, chosen by `page create`, and
//! independent of the template from then on. Native command scenarios on the CLI suite's isolated
//! root and immutable Core fixtures.
use super::*;
const AGENT: &str = "50000000-0000-1000-8000-000000000001";
const QUESTION: &str = "00000000-0000-4000-8000-0000000000a1";
const TEMPLATE_SOURCE: &str = "<!-- Header: week number and one-line status -->\n<h1>Week</h1>\n<!-- Shipped: one row per merged item -->\n<p>Shipped</p>\n<!-- Keep the section order -->\n<p>Next</p>";

fn text(output: &std::process::Output) -> String {
    assert!(output.status.success(), "{output:?}");
    String::from_utf8(output.stdout.clone()).unwrap()
}
/// A command that must refuse: its JSON error, whose hint is the fix.
fn refused(pilot: &Pilot, args: &[&str], code: &str) -> Value {
    let output = pilot.command().args(args).output().unwrap();
    assert!(!output.status.success(), "{args:?}: {output:?}");
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["error"]["code"], code, "{value}");
    value
}
fn write(pilot: &Pilot, page: &str, source: &str) {
    let file = pilot.root.join(format!("{page}.html"));
    fs::write(&file, source).unwrap();
    pilot.patch_call(page, &["--file", file.to_str().unwrap(), "--json"]);
}
/// A column page holding `source`.
fn page(pilot: &Pilot, title: &str, source: &str) -> String {
    let id = pilot.call(&[
        "page", "create", "--title", title, "--layout", "column", "--json",
    ])["pageId"]
        .as_str()
        .unwrap()
        .to_owned();
    write(pilot, &id, source);
    id
}
fn save(pilot: &Pilot, page: &str, name: &str) -> Value {
    pilot.call(&[
        "template",
        "save",
        page,
        "--name",
        name,
        "--description",
        "Shipped, open, risks, next week",
        "--json",
    ])
}
fn source_of(pilot: &Pilot, page: &str) -> String {
    pilot.call(&["page", "read", page, "--json"])["source"]
        .as_str()
        .unwrap()
        .to_owned()
}
fn listed(pilot: &Pilot) -> Value {
    pilot.call(&["template", "ls", "--json"])
}
fn entry<'a>(listing: &'a Value, name: &str) -> &'a Value {
    listing["templates"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["name"] == name)
        .unwrap_or_else(|| panic!("no template {name} in {listing}"))
}
fn settings(pilot: &Pilot) -> Value {
    serde_json::from_str(
        &fs::read_to_string(pilot.root.join("selected/colab/settings.json")).unwrap(),
    )
    .unwrap()
}

#[test]
fn a_page_is_saved_listed_shown_chosen_as_the_default_and_removed() {
    for serving in [false, true] {
        let mut pilot = Pilot::new(None);
        let id = page(&pilot, "Weekly review", TEMPLATE_SOURCE);
        if serving {
            pilot.start();
        }
        let saved = save(&pilot, &id, "weekly-review");
        assert_eq!(saved["pageId"], id.as_str());
        assert_eq!(saved["name"], "weekly-review");
        assert_eq!(saved["layout"], "column");
        assert_eq!(saved["rules"], 3, "{saved}");
        assert_eq!(saved["changed"], true);
        // The same name again changes nothing.
        assert_eq!(save(&pilot, &id, "weekly-review")["changed"], false);
        // The page stays an ordinary page, marked.
        let read = pilot.call(&["page", "read", &id, "--json"]);
        assert_eq!(read["template"]["name"], "weekly-review");
        assert_eq!(
            read["template"]["description"],
            "Shipped, open, risks, next week"
        );
        assert_eq!(read["source"], TEMPLATE_SOURCE);

        let listing = listed(&pilot);
        assert_eq!(listing["default"], "starter");
        let user = entry(&listing, "weekly-review");
        assert_eq!(user["id"], id.as_str());
        assert_eq!(user["source"], "space");
        assert_eq!(user["layout"], "column");
        assert_eq!(user["summary"], "Shipped, open, risks, next week");
        assert_eq!(user["default"], false);
        assert_eq!(user["usedBy"], 0);
        assert_eq!(entry(&listing, "starter")["default"], true);
        assert_eq!(entry(&listing, "starter")["source"], "builtIn");

        let human = text(&pilot.command().args(["template", "ls"]).output().unwrap());
        assert!(human.contains("BUILT-IN 6"), "{human}");
        assert!(human.contains("THIS SPACE 1"), "{human}");
        assert!(human.contains(&id[..8]), "{human}");
        assert!(human.contains("hint: tmt colab page create"), "{human}");

        let shown = pilot.call(&["template", "show", "weekly-review", "--json"]);
        assert_eq!(shown["id"], id.as_str());
        assert_eq!(shown["usedBy"], 0);
        assert_eq!(
            shown["rules"],
            json!([
                "Header: week number and one-line status",
                "Shipped: one row per merged item",
                "Keep the section order"
            ])
        );
        // A page ID, or a prefix of one, names the same template.
        assert_eq!(
            pilot.call(&["template", "show", &id[..8], "--json"])["name"],
            "weekly-review"
        );
        let human = text(
            &pilot
                .command()
                .args(["template", "show", "weekly-review"])
                .output()
                .unwrap(),
        );
        assert!(human.starts_with("weekly-review\n"), "{human}");
        let fields: Vec<Vec<&str>> = human
            .lines()
            .map(|line| line.split_whitespace().collect())
            .collect();
        assert!(fields.contains(&vec!["id", &id[..8]]), "{human}");
        assert!(fields.contains(&vec!["layout", "column"]), "{human}");
        assert!(
            fields.contains(&vec!["used", "by", "0", "pages"]),
            "{human}"
        );
        assert!(
            fields.contains(&vec![
                "rules", "1", "Header:", "week", "number", "and", "one-line", "status"
            ]),
            "{human}"
        );
        assert!(
            fields.contains(&vec!["3", "Keep", "the", "section", "order"]),
            "{human}"
        );
        assert!(human.contains("hint: tmt colab page read"), "{human}");

        // Choose it as the default, and go back.
        let set = pilot.call(&["template", "default", "weekly-review", "--json"]);
        assert_eq!(set["default"], "weekly-review");
        assert_eq!(settings(&pilot)["defaultTemplate"], id.as_str());
        let listing = listed(&pilot);
        assert_eq!(listing["default"], "weekly-review");
        assert_eq!(entry(&listing, "weekly-review")["default"], true);
        assert_eq!(entry(&listing, "starter")["default"], false);
        assert_eq!(
            pilot.call(&["template", "default", "--json"])["default"],
            "weekly-review"
        );
        let human = text(
            &pilot
                .command()
                .args(["template", "default", "--reset"])
                .output()
                .unwrap(),
        );
        assert!(
            human.contains("✓ Set the space default template to starter (TMT)"),
            "{human}"
        );
        assert!(settings(&pilot).get("defaultTemplate").is_none());
        assert_eq!(listed(&pilot)["default"], "starter");

        // Remove it: the page stays, now an ordinary page.
        let human = text(
            &pilot
                .command()
                .args(["template", "remove", "weekly-review"])
                .output()
                .unwrap(),
        );
        assert!(
            human.contains(&format!(
                "✓ Removed template weekly-review; page {} stays as a page",
                &id[..8]
            )),
            "{human}"
        );
        assert!(human.contains("pages created from it"), "{human}");
        assert!(human.contains("0, unchanged"), "{human}");
        assert!(
            pilot
                .call(&["page", "read", &id, "--json"])
                .get("template")
                .is_none()
        );
        assert!(
            listed(&pilot)["templates"]
                .as_array()
                .unwrap()
                .iter()
                .all(|entry| entry["name"] != "weekly-review")
        );
        assert_eq!(source_of(&pilot, &id), TEMPLATE_SOURCE);
    }
}

#[test]
fn a_page_created_from_a_user_template_copies_its_current_content_and_then_stands_alone() {
    for serving in [false, true] {
        let mut pilot = Pilot::new(None);
        let id = page(&pilot, "Weekly review", TEMPLATE_SOURCE);
        if serving {
            pilot.start();
        }
        save(&pilot, &id, "weekly-review");
        let created = pilot.call(&[
            "page",
            "create",
            "--title",
            "Week 43",
            "--template",
            "weekly-review",
            "--json",
        ]);
        assert_eq!(created["template"], "weekly-review");
        assert_eq!(created["layout"], "column");
        assert_eq!(created["read"][0], "tmt colab template show weekly-review");
        assert!(created["read"].as_array().unwrap().len() > 1, "{created}");
        assert!(created.get("templateDefault").is_none());
        let first = created["pageId"].as_str().unwrap().to_owned();
        assert_eq!(source_of(&pilot, &first), TEMPLATE_SOURCE);
        // The new page records where it came from, with the template's revision, and is no template.
        let read = pilot.call(&["page", "read", &first, "--json"]);
        assert!(read.get("template").is_none(), "{read}");
        assert_eq!(read["fromTemplate"]["name"], "weekly-review");
        // The revision is the digest of the content that was copied, which `template show` prints
        // for the template as it is now.
        let at = pilot.call(&["template", "show", "weekly-review", "--json"])["revision"].clone();
        assert_eq!(read["fromTemplate"]["revision"], at);
        assert_eq!(
            pilot.call(&["template", "show", "weekly-review", "--json"])["usedBy"],
            1
        );
        // The ID names it too.
        let by_id = pilot.call(&[
            "page",
            "create",
            "--title",
            "Week 44",
            "--template",
            &id[..8],
            "--json",
        ]);
        assert_eq!(by_id["template"], "weekly-review");
        assert_eq!(
            pilot.call(&["template", "show", "weekly-review", "--json"])["usedBy"],
            2
        );

        // The template changes: the page already created does not, the next one does.
        write(
            &pilot,
            &id,
            &format!("{TEMPLATE_SOURCE}\n<p>New section</p>"),
        );
        assert_eq!(source_of(&pilot, &first), TEMPLATE_SOURCE);
        let next = pilot.call(&[
            "page",
            "create",
            "--title",
            "Week 45",
            "--template",
            "weekly-review",
            "--json",
        ]);
        assert!(
            source_of(&pilot, next["pageId"].as_str().unwrap()).ends_with("<p>New section</p>")
        );
        // Writing the page does not write the template; the page keeps its source and its mark.
        write(&pilot, &first, "<p>Mine</p>");
        assert!(source_of(&pilot, &id).ends_with("<p>New section</p>"));
        assert_eq!(
            pilot.call(&["page", "read", &first, "--json"])["fromTemplate"]["name"],
            "weekly-review"
        );
        // Removing and deleting the template change nothing about the pages created from it.
        pilot.call(&["template", "remove", "weekly-review", "--json"]);
        assert_eq!(source_of(&pilot, &first), "<p>Mine</p>");
        assert_eq!(
            pilot.call(&["page", "read", &first, "--json"])["fromTemplate"]["name"],
            "weekly-review"
        );
        let human = text(
            &pilot
                .command()
                .args([
                    "page",
                    "create",
                    "--title",
                    "Human",
                    "--template",
                    "starter",
                ])
                .output()
                .unwrap(),
        );
        assert!(
            human.contains("tmt colab template show starter · tmt colab skill layout"),
            "{human}"
        );
    }
}

#[test]
fn the_questions_of_a_template_are_registered_and_open_for_every_copy() {
    let pilot = Pilot::new(None);
    let id = pilot.call(&[
        "page", "create", "--title", "Review", "--layout", "column", "--json",
    ])["pageId"]
        .as_str()
        .unwrap()
        .to_owned();
    pilot.call(&[
        "question", "add", &id, "--id", QUESTION, "--agent", AGENT, "--title", "Ship it?", "--json",
    ]);
    let source = format!(
        "<p>Intro</p>\n<tmt-question data-id=\"{QUESTION}\" data-kind=\"short\"><p data-question>Ship it?</p></tmt-question>"
    );
    write(&pilot, &id, &source);
    // Handled in the template: a copy starts open, with no task.
    pilot.call(&["question", "handle", &id, QUESTION, "--json"]);
    save(&pilot, &id, "release-review");
    let created = pilot.call(&[
        "page",
        "create",
        "--title",
        "Release 9",
        "--template",
        "release-review",
        "--json",
    ]);
    let copy = created["pageId"].as_str().unwrap();
    let questions = pilot.call(&["question", "ls", copy, "--json"]);
    let entries = questions["questions"].as_array().unwrap();
    assert_eq!(entries.len(), 1, "{questions}");
    assert_eq!(entries[0]["id"], QUESTION);
    assert_eq!(entries[0]["state"], "open", "{questions}");
    assert_eq!(entries[0]["title"], "Ship it?");
    assert_eq!(entries[0]["agent"]["name"], "Fixture agent");
    // The blocks name the creator, and nothing else of the source changed.
    let copied = source_of(&pilot, copy);
    assert!(copied.contains("data-agent=\"Fixture agent\""), "{copied}");
    assert!(copied.starts_with("<p>Intro</p>"), "{copied}");
    // The template's own registry is untouched.
    assert_eq!(
        pilot.call(&["question", "ls", &id, "--json"])["questions"][0]["state"],
        "handled"
    );
}

#[test]
fn an_anonymous_copy_registers_its_questions_with_no_agent_and_names_none_in_its_blocks() {
    let pilot = Pilot::new(None);
    let id = pilot.call(&[
        "page", "create", "--title", "Review", "--layout", "column", "--json",
    ])["pageId"]
        .as_str()
        .unwrap()
        .to_owned();
    pilot.call(&[
        "question", "add", &id, "--id", QUESTION, "--agent", AGENT, "--title", "Ship it?", "--json",
    ]);
    // The template's block names its author; a copy made by nobody names nobody.
    write(
        &pilot,
        &id,
        &format!(
            "<p>Intro</p>\n<tmt-question data-id=\"{QUESTION}\" data-agent=\"{AGENT}\" data-kind=\"short\"><p data-question>Ship it?</p></tmt-question>"
        ),
    );
    save(&pilot, &id, "release-review");
    fs::remove_file(pilot.root.join("publisher")).unwrap();
    let created = pilot.call(&[
        "page",
        "create",
        "--title",
        "Release 9",
        "--template",
        "release-review",
        "--json",
    ]);
    let copy = created["pageId"].as_str().unwrap();
    let read = pilot.call(&["page", "read", copy, "--json"]);
    assert!(read.get("creator").is_none(), "{read}");
    let entries = read["questions"].as_array().unwrap();
    assert_eq!(entries.len(), 1, "{read}");
    assert_eq!(entries[0]["id"], QUESTION);
    assert_eq!(entries[0]["state"], "open");
    assert_eq!(entries[0]["title"], "Ship it?");
    assert!(entries[0].get("agent").is_none(), "{read}");
    let source = read["source"].as_str().unwrap();
    assert!(!source.contains("data-agent"), "{source}");
    assert!(source.contains(QUESTION), "{source}");
    // The page records where it came from, as any copy does.
    assert_eq!(read["fromTemplate"]["name"], "release-review");
}

#[test]
fn the_default_is_what_a_bare_create_starts_from_and_returns_to_starter_when_it_is_gone() {
    for deleted in [false, true] {
        let pilot = Pilot::new(None);
        let id = page(&pilot, "Weekly review", TEMPLATE_SOURCE);
        save(&pilot, &id, "weekly-review");
        // No default is set: a create naming nothing starts from starter.
        let first = pilot.call(&["page", "create", "--title", "Notes", "--json"]);
        assert_eq!(first["template"], "starter");
        assert_eq!(first["templateDefault"], true);
        assert_eq!(first["layout"], "column");
        pilot.call(&["template", "default", "weekly-review", "--json"]);
        let second = pilot.call(&["page", "create", "--title", "Week", "--json"]);
        assert_eq!(second["template"], "weekly-review");
        assert_eq!(second["templateDefault"], true);
        assert_eq!(
            source_of(&pilot, second["pageId"].as_str().unwrap()),
            TEMPLATE_SOURCE
        );
        let human = text(
            &pilot
                .command()
                .args(["page", "create", "--title", "Human"])
                .output()
                .unwrap(),
        );
        assert!(human.contains("weekly-review (space default)"), "{human}");
        // A layout or a file keeps its own meaning: the default is not consulted.
        let plain = pilot.call(&[
            "page", "create", "--title", "Plain", "--layout", "full", "--json",
        ]);
        assert!(plain.get("template").is_none());
        assert_eq!(plain["layout"], "full");

        if deleted {
            // Deleting the page returns the default to starter, which the CLI does itself.
            let deleted = pilot.call(&["delete", &id, "--yes", "--json"]);
            assert_eq!(deleted["templateDefaultReset"], true, "{deleted}");
            assert!(settings(&pilot).get("defaultTemplate").is_none());
        } else {
            let human = text(
                &pilot
                    .command()
                    .args(["template", "remove", "weekly-review"])
                    .output()
                    .unwrap(),
            );
            assert!(
                human.contains(
                    "warning: weekly-review was the space default; the default is now starter"
                ),
                "{human}"
            );
            assert!(settings(&pilot).get("defaultTemplate").is_none());
        }
        let after = pilot.call(&["page", "create", "--title", "After", "--json"]);
        assert_eq!(after["template"], "starter");
        assert_eq!(listed(&pilot)["default"], "starter");
    }
}

#[test]
fn a_default_that_names_a_template_that_is_gone_falls_back_to_starter_and_says_so() {
    let pilot = Pilot::new(None);
    let id = page(&pilot, "Weekly review", TEMPLATE_SOURCE);
    save(&pilot, &id, "weekly-review");
    pilot.call(&["template", "default", "weekly-review", "--json"]);
    // The mark goes through a path that does not know the setting: the page's template mark is
    // cleared by the owner while the setting stays.
    let settings_file = pilot.root.join("selected/colab/settings.json");
    pilot.call(&["template", "remove", "weekly-review", "--json"]);
    fs::write(&settings_file, json!({"defaultTemplate": id}).to_string()).unwrap();
    let output = pilot
        .command()
        .args(["page", "create", "--title", "Notes", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let created: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(created["template"], "starter");
    let warning = String::from_utf8_lossy(&output.stderr);
    assert!(warning.contains("warning:"), "{warning}");
    assert!(warning.contains("no longer exists"), "{warning}");
    assert!(warning.contains("page create uses starter"), "{warning}");
}

#[test]
fn every_refusal_names_its_fix() {
    let pilot = Pilot::new(None);
    // The seeded page is written into a space with no other page.
    let legacy = seed_legacy_page(&pilot);
    let id = page(&pilot, "Weekly review", TEMPLATE_SOURCE);
    let other = page(&pilot, "Other", "<p>Other</p>");
    save(&pilot, &id, "weekly-review");
    let short = &other[..8];

    let unknown = refused(
        &pilot,
        &[
            "page",
            "create",
            "--title",
            "Notes",
            "--template",
            "weekly",
            "--json",
        ],
        "COLAB_TEMPLATE_UNKNOWN",
    );
    assert_eq!(unknown["error"]["message"], "No template named weekly");
    assert_eq!(unknown["error"]["hint"], "tmt colab template ls");
    let human = pilot
        .command()
        .args(["page", "create", "--title", "Notes", "--template", "weekly"])
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&human.stderr),
        "error: No template named weekly\nhint: tmt colab template ls\n"
    );

    let reserved = refused(
        &pilot,
        &["template", "save", &other, "--name", "report", "--json"],
        "COLAB_TEMPLATE_NAME_RESERVED",
    );
    assert_eq!(
        reserved["error"]["message"],
        "report is a built-in template name"
    );
    assert_eq!(
        reserved["error"]["hint"],
        format!("tmt colab template save {short} --name report-ops")
    );

    let taken = refused(
        &pilot,
        &[
            "template",
            "save",
            &other,
            "--name",
            "weekly-review",
            "--json",
        ],
        "COLAB_TEMPLATE_NAME_TAKEN",
    );
    assert_eq!(
        taken["error"]["message"],
        format!("weekly-review is already page {}'s template", &id[..8])
    );
    assert_eq!(
        taken["error"]["hint"],
        "tmt colab template remove weekly-review, or choose another --name"
    );

    for name in ["Weekly", "", &"x".repeat(41), "has space"] {
        refused(
            &pilot,
            &["template", "save", &other, "--name", name, "--json"],
            "COLAB_TEMPLATE_NAME_INVALID",
        );
    }
    refused(
        &pilot,
        &[
            "template",
            "save",
            &other,
            "--name",
            "other",
            "--description",
            " padded ",
            "--json",
        ],
        "COLAB_TEMPLATE_DESCRIPTION_INVALID",
    );
    refused(
        &pilot,
        &["template", "remove", "starter", "--json"],
        "COLAB_TEMPLATE_BUILT_IN",
    );
    refused(
        &pilot,
        &["template", "remove", short, "--json"],
        "COLAB_TEMPLATE_UNKNOWN",
    );
    refused(
        &pilot,
        &["template", "show", "nothing", "--json"],
        "COLAB_TEMPLATE_UNKNOWN",
    );
    refused(
        &pilot,
        &["template", "default", "nothing", "--json"],
        "COLAB_TEMPLATE_UNKNOWN",
    );
    // A dashboard is not a page to copy.
    let dashboard = pilot.call(&["dashboard", "create", "--title", "Board", "--json"])["pageId"]
        .as_str()
        .unwrap()
        .to_owned();
    refused(
        &pilot,
        &[
            "template", "save", &dashboard, "--name", "board-2", "--json",
        ],
        "COLAB_TEMPLATE_UNSUPPORTED",
    );
    // A page written before templates existed has neither mark and is no template.
    let read = pilot.call(&["page", "read", &legacy, "--json"]);
    assert!(
        read.get("template").is_none() && read.get("fromTemplate").is_none(),
        "{read}"
    );
    assert!(
        listed(&pilot)["templates"]
            .as_array()
            .unwrap()
            .iter()
            .all(|entry| entry["id"] != legacy.as_str())
    );
    // A page that never named a layout would create pages without one.
    let layout = refused(
        &pilot,
        &["template", "save", &legacy, "--name", "legacy", "--json"],
        "COLAB_TEMPLATE_NEEDS_LAYOUT",
    );
    assert!(
        layout["error"]["hint"]
            .as_str()
            .unwrap()
            .contains("tmt colab page layout"),
        "{layout}"
    );
    // Nothing above made another template.
    assert_eq!(
        listed(&pilot)["templates"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|entry| entry["source"] == "space")
            .count(),
        1
    );
}

#[test]
fn a_space_that_does_not_exist_yet_lists_only_the_compiled_templates() {
    let pilot = Pilot::new(None);
    let listing = listed(&pilot);
    assert_eq!(listing["templates"].as_array().unwrap().len(), 6);
    assert_eq!(listing["default"], "starter");
    assert!(!pilot.root.join("selected/colab").exists());
    let shown = pilot.call(&["template", "show", "board", "--json"]);
    assert_eq!(shown["layout"], "full");
    assert_eq!(shown["source"], "builtIn");
    assert!(!shown["rules"].as_array().unwrap().is_empty());
    assert!(!pilot.root.join("selected/colab").exists());
}

/// A page written before layouts existed: no layout in its metadata.
fn seed_legacy_page(pilot: &Pilot) -> String {
    seed_page(pilot);
    PAGE.to_owned()
}
