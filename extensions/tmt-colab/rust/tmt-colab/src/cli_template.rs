//! Page templates (#3928): a few starting pages for common kinds, so an agent starts from markup
//! that already follows the frame and says where each rule is taught. `template ls` lists them and
//! `page create --template NAME` creates a page from one, with the template's layout. Every block
//! of a template carries one HTML comment naming its rule and the skill section (the comments are
//! not rendered), and a template sets no width or gap: only the two declarations
//! (`data-width`, `data-gap`) and the layout inside its blocks.
use clap::{Arg, ArgMatches, Command, builder::PossibleValuesParser};
use serde_json::json;
use std::io::Write;
use tmt_cli_style::{CommandSpec, Example, OutputModes};
use tmt_colab::{
    Result,
    decoder::{LayoutKind, PageLayout},
};

use crate::cli_management::fresh_id;

/// One starting page: its name, what it is for, the layout it sets and its source.
pub struct Template {
    pub name: &'static str,
    pub summary: &'static str,
    kind: LayoutKind,
    source: &'static str,
}

pub const TEMPLATES: [Template; 4] = [
    Template {
        name: "report",
        summary: "Summary, findings table and next steps",
        kind: LayoutKind::Column,
        source: include_str!("../templates/report.html"),
    },
    Template {
        name: "review",
        summary: "Parts for the reader to confirm, and questions",
        kind: LayoutKind::Column,
        source: include_str!("../templates/review.html"),
    },
    Template {
        name: "compare",
        summary: "Now and after, side by side",
        kind: LayoutKind::Column,
        source: include_str!("../templates/compare.html"),
    },
    Template {
        name: "board",
        summary: "A full-bleed status board",
        kind: LayoutKind::Full,
        source: include_str!("../templates/board.html"),
    },
];

impl Template {
    pub fn layout(&self) -> PageLayout {
        PageLayout {
            kind: self.kind,
            reason: None,
        }
    }

    /// The page source, with each `{{uuid:NAME}}` replaced by a fresh canonical UUID, the same one
    /// for the same name within this page (a block's ID is unique on its page, never reused).
    pub fn source(&self) -> Result<String> {
        let mut names: Vec<(&str, String)> = Vec::new();
        let mut rendered = String::with_capacity(self.source.len() + 64);
        let mut rest = self.source;
        while let Some(start) = rest.find("{{uuid:") {
            rendered.push_str(&rest[..start]);
            let after = &rest[start + "{{uuid:".len()..];
            let end = after
                .find("}}")
                .expect("a template placeholder is closed (checked by the catalog test)");
            let name = &after[..end];
            let id = match names.iter().find(|(known, _)| *known == name) {
                Some((_, id)) => id.clone(),
                None => {
                    let id = fresh_id()?;
                    names.push((name, id.clone()));
                    id
                }
            };
            rendered.push_str(&id);
            rest = &after[end + 2..];
        }
        rendered.push_str(rest);
        Ok(rendered)
    }
}

pub fn find(name: &str) -> Option<&'static Template> {
    TEMPLATES.iter().find(|template| template.name == name)
}

/// The template `page create` was asked to start from, if any.
pub fn chosen(args: &ArgMatches) -> Option<&'static Template> {
    args.get_one::<String>("template")
        .map(|name| find(name).expect("clap admits only catalog names"))
}

/// `--template` on `page create`: it brings its own layout and source, so it excludes the layout
/// and source options rather than silently overriding them.
pub fn create_arg(command: Command) -> Command {
    command.arg(
        Arg::new("template")
            .long("template")
            .value_name("NAME")
            .value_parser(PossibleValuesParser::new(
                TEMPLATES.iter().map(|template| template.name),
            ))
            .conflicts_with_all(["layout", "reason", "file"])
            .help("Start from a template, which sets the layout and the source (tmt colab template ls)"),
    )
}

const TEMPLATE: CommandSpec = CommandSpec {
    name: "template",
    summary: "List the page templates",
    examples: &[Example {
        command: "tmt colab template ls",
        note: "See which templates exist and the layout each sets",
    }],
    outputs: OutputModes::HumanAndJson,
    details: "A template is a starting page for a common kind. page create --template NAME creates a page from it with the template's layout, so choose it instead of --layout and --file. Each block of a template has an HTML comment that names its rule and the skill section that teaches it (tmt colab skill layout, tmt colab skill components); the comments are not shown on the page. Replace the placeholder text and keep the structure. Reads only; needs no running serve and no local state.",
};

const LS: CommandSpec = CommandSpec {
    name: "ls",
    summary: "List every template with its layout",
    examples: &[Example {
        command: "tmt colab template ls --json",
        note: "Each template's name, layout and summary",
    }],
    outputs: OutputModes::HumanAndJson,
    details: "Prints each template's name, the layout it sets and what it is for.",
};

pub fn command() -> Command {
    tmt_cli_style::command(&TEMPLATE)
        .mut_arg("json", |arg| arg.global(true))
        .subcommand_required(true)
        .subcommand(tmt_cli_style::command(&LS).alias("list"))
}

pub fn run(args: &ArgMatches) -> Result<()> {
    let (_, args) = args.subcommand().expect("required template command");
    let json_output = args.get_flag("json");
    let mut output = tmt_cli_style::stream::stdout(json_output);
    if json_output {
        let templates: Vec<_> = TEMPLATES
            .iter()
            .map(|template| {
                json!({
                    "name": template.name,
                    "layout": template.kind.as_str(),
                    "summary": template.summary,
                })
            })
            .collect();
        writeln!(output, "{}", json!({ "templates": templates }))?;
        return Ok(());
    }
    let terminal = output.terminal();
    let rows: Vec<(&str, String)> = TEMPLATES
        .iter()
        .map(|template| {
            (
                template.name,
                format!("{} · {}", template.kind.as_str(), template.summary),
            )
        })
        .collect();
    tmt_cli_style::detail::write(&mut output, terminal, "TEMPLATES", &rows)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        SKILL_REFERENCES,
        cli_page_check::{Attachments, inspect},
    };

    fn reference(topic: &str) -> &'static str {
        SKILL_REFERENCES
            .iter()
            .find(|(name, _)| *name == topic)
            .unwrap_or_else(|| panic!("no skill topic {topic}"))
            .1
    }

    #[test]
    fn the_catalog_is_the_four_documented_templates_with_their_layouts() {
        let listed: Vec<(&str, &str)> = TEMPLATES
            .iter()
            .map(|template| (template.name, template.kind.as_str()))
            .collect();
        assert_eq!(
            listed,
            [
                ("report", "column"),
                ("review", "column"),
                ("compare", "column"),
                ("board", "full")
            ]
        );
        for template in &TEMPLATES {
            assert!(find(template.name).is_some());
            assert!(template.layout().reason.is_none());
        }
        assert!(find("freeform").is_none());
    }

    #[test]
    fn every_template_renders_checks_clean_and_sets_no_width_or_gap() {
        for template in &TEMPLATES {
            let source = template.source().unwrap();
            assert!(
                !source.contains("{{"),
                "{} has an open placeholder",
                template.name
            );
            let report = inspect(&source, Attachments::NewPage).unwrap();
            assert!(
                report.findings.is_empty(),
                "{} fails page check: {:?}",
                template.name,
                report.findings
            );
            assert!(
                source.len() < 8 * 1024,
                "{} is a starting point",
                template.name
            );
            // Only the two declarations change the frame; the CSS inside a block is the author's.
            // The comments may name these properties to say they are not for the author to set.
            let markup: String = source
                .lines()
                .filter(|line| !line.starts_with("<!--"))
                .collect();
            for forbidden in ["max-width", "min-width", "margin", "width:", "gap:"] {
                assert!(
                    !markup.contains(forbidden),
                    "{} sets {forbidden}",
                    template.name
                );
            }
        }
    }

    #[test]
    fn a_page_gets_fresh_ids_each_time_and_one_id_per_placeholder_name() {
        let review = find("review").unwrap();
        let ids = |source: &str| -> Vec<String> {
            source
                .split("data-id=\"")
                .skip(1)
                .map(|rest| rest.split('"').next().unwrap().to_owned())
                .collect()
        };
        let first = ids(&review.source().unwrap());
        let second = ids(&review.source().unwrap());
        assert_eq!(first.len(), 4);
        for id in &first {
            tmt_colab_model::values::generated_id(id).unwrap();
        }
        let mut unique = first.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), first.len(), "an ID is unique on its page");
        assert!(first.iter().all(|id| !second.contains(id)), "IDs are fresh");
        // A name that appears twice would render as one ID; the catalog test above forbids open
        // placeholders, and this checks the pairing directly.
        let twice = Template {
            name: "x",
            summary: "x",
            kind: LayoutKind::Column,
            source: "{{uuid:a}} {{uuid:b}} {{uuid:a}}",
        };
        let parts: Vec<String> = twice
            .source()
            .unwrap()
            .split(' ')
            .map(str::to_owned)
            .collect();
        assert_eq!(parts[0], parts[2]);
        assert_ne!(parts[0], parts[1]);
    }

    #[test]
    fn every_block_has_one_comment_that_points_at_real_skill_sections() {
        for template in &TEMPLATES {
            let source = template.source().unwrap();
            let mut lines = source.lines();
            // The first comment is the page's: its layout, which has no block of its own.
            let header = lines.next().unwrap();
            // The e2e spec reads the layout from this line, so it must be the catalog's.
            assert!(
                header.starts_with(&format!("<!-- layout: {}.", template.kind.as_str())),
                "{}: {header}",
                template.name
            );
            let mut comments = vec![header];
            let mut previous_was_comment = false;
            let mut blocks = 0;
            for line in lines {
                // A block, or a style rule, starts at column 0; indented lines are inside one.
                if !line.starts_with('<') || line.starts_with("</") {
                    continue;
                }
                if line.starts_with("<!--") {
                    assert!(
                        !previous_was_comment,
                        "{}: two comments in a row before {line}",
                        template.name
                    );
                    assert!(line.ends_with("-->"), "{}: one-line comment", template.name);
                    previous_was_comment = true;
                    comments.push(line);
                } else {
                    assert!(
                        previous_was_comment,
                        "{}: a block without its comment: {line}",
                        template.name
                    );
                    previous_was_comment = false;
                    blocks += 1;
                }
            }
            assert!(blocks >= 3, "{} is a starting page", template.name);
            assert!(
                !previous_was_comment,
                "{}: a comment with no block",
                template.name
            );
            for comment in comments {
                let comment = comment
                    .trim_start_matches("<!--")
                    .trim_end_matches("-->")
                    .trim();
                assert!(
                    comment.contains("tmt colab skill "),
                    "{}: {comment} names no skill topic",
                    template.name
                );
                for pointer in comment.split("tmt colab skill ").skip(1) {
                    // `topic`, or `topic, section Name`, or `topic, sections A and B`, then a stop.
                    let pointer = pointer.split(". ").next().unwrap().trim();
                    let (topic, sections) =
                        pointer.split_once(", section").unwrap_or((pointer, ""));
                    let text = reference(topic);
                    for heading in sections.trim_start_matches('s').split(" and ") {
                        let heading = heading.trim();
                        assert!(
                            heading.is_empty()
                                || text.lines().any(|line| line == format!("## {heading}")),
                            "{}: no section `{heading}` in tmt colab skill {topic}",
                            template.name
                        );
                    }
                }
            }
        }
    }
}
