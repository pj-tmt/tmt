//! Squad commands that do not yet follow the help style, with the rules each
//! still breaks. #436 PR 2b migrates them; the list only shrinks and is empty
//! at the end. Every entry breaks `HelpForms` because Squad has no
//! `help <command>` route yet; 2b adds one through the shared registration,
//! not a Squad-specific subcommand.

use tmt_cli_style::audit::Rule::{self, *};

pub const MIGRATING: &[(&str, &[Rule])] = &[
    ("tmt squad", &[HelpForms, Template, Examples]),
    ("tmt squad add", &[HelpForms, Template, Examples]),
    ("tmt squad annotate", &[HelpForms, Template, Examples]),
    ("tmt squad back", &[HelpForms, Template, Examples]),
    ("tmt squad board", &[HelpForms, Template, Examples]),
    ("tmt squad copy", &[HelpForms, Template, Examples]),
    ("tmt squad hotkeys", &[HelpForms, Template, Examples]),
    (
        "tmt squad hotkeys install",
        &[HelpForms, Template, Examples],
    ),
    ("tmt squad hotkeys remove", &[HelpForms, Template, Examples]),
    ("tmt squad hotkeys show", &[HelpForms, Template, Examples]),
    ("tmt squad init", &[HelpForms, Template, Examples]),
    ("tmt squad jump", &[HelpForms, Template, Examples]),
    ("tmt squad lead", &[HelpForms, Template, Examples]),
    ("tmt squad open", &[HelpForms, Template, Examples]),
    ("tmt squad remove", &[HelpForms, Template, Examples]),
    ("tmt squad replies", &[HelpForms, Template, Examples]),
    ("tmt squad reply", &[HelpForms, Template, Examples]),
    ("tmt squad set", &[HelpForms, Template, Examples]),
    ("tmt squad skill", &[HelpForms, Template, Examples]),
    ("tmt squad skill show", &[HelpForms, Template, Examples]),
    ("tmt squad status", &[HelpForms, Template, Examples]),
    ("tmt squad talk", &[HelpForms, Template, Examples]),
];
