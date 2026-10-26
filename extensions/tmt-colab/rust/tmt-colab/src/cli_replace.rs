//! The `--replace` gate of `page write` (#5025). A write that replaces more than half of an
//! existing source of at least 2 KiB (`EditScale::is_large_rewrite`, the measure the editing hint
//! uses) is refused unless the caller says it means to, so a small change is not paid for with a
//! whole-page rewrite that discards concurrent edits. Only `page write` is gated: `page patch`
//! is anchored to the text it changes, and page creation, templates, the browser's Save and an
//! export restore never replace existing source through this command.
use std::{error::Error, path::Path};
use tmt_colab::edit_scale::EditScale;

use crate::cli_management::fail;

/// The code a gated write is refused with.
pub const CODE: &str = "COLAB_REWRITE_REQUIRES_REPLACE";

/// A `page write` that has not passed `--replace`, with what it needs to name its own rerun.
pub struct Gate<'a> {
    pub page: &'a str,
    pub file: &'a Path,
    pub expected_revision: Option<&'a str>,
}

impl Gate<'_> {
    /// The refusal for a write whose measure is `scale`; nothing has been published.
    pub fn refusal(&self, scale: EditScale) -> Box<dyn Error + Send + Sync> {
        let percent = scale.replaced_bytes * 100 / scale.base_bytes.max(1);
        fail(
            CODE,
            &format!(
                "This write would replace {percent}% of the page ({} of {} bytes). To change part of \
                 a page, read only the lines you need and use `tmt colab page patch` (see \
                 `tmt colab skill editing`). To replace the page on purpose, run `{}`. Nothing was \
                 written.",
                scale.replaced_bytes,
                scale.base_bytes,
                self.rerun()
            ),
        )
    }

    /// The command that repeats this write with `--replace`, keeping its fence.
    fn rerun(&self) -> String {
        let mut command = format!(
            "tmt colab page write {} --file {}",
            self.page,
            shell_word(&self.file.to_string_lossy())
        );
        if let Some(revision) = self.expected_revision {
            command.push_str(" --expected-revision ");
            command.push_str(&shell_word(revision));
        }
        command.push_str(" --replace");
        command
    }
}

/// `text` as one shell word: unchanged when it holds only characters a shell leaves alone,
/// otherwise single-quoted.
fn shell_word(text: &str) -> String {
    let plain = !text.is_empty()
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_-./:@%+=,".contains(c));
    if plain {
        text.to_owned()
    } else {
        format!("'{}'", text.replace('\'', "'\\''"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = "10000000-0000-4000-8000-000000000001";

    fn message(gate: &Gate<'_>) -> String {
        gate.refusal(EditScale {
            base_bytes: 4096,
            replaced_bytes: 3000,
        })
        .to_string()
    }

    #[test]
    fn the_refusal_names_the_exact_rerun_with_its_fence_and_the_way_to_patch() {
        let plain = message(&Gate {
            page: PAGE,
            file: Path::new("/tmp/page.html"),
            expected_revision: None,
        });
        assert!(
            plain.contains(&format!(
                "`tmt colab page write {PAGE} --file /tmp/page.html --replace`"
            )),
            "{plain}"
        );
        assert!(
            plain.contains("73%") && plain.contains("3000 of 4096 bytes"),
            "{plain}"
        );
        assert!(plain.contains("tmt colab page patch"), "{plain}");
        assert!(plain.ends_with("Nothing was written."), "{plain}");
        let fenced = message(&Gate {
            page: PAGE,
            file: Path::new("-"),
            expected_revision: Some("v1:abc"),
        });
        assert!(
            fenced.contains(&format!(
                "tmt colab page write {PAGE} --file - --expected-revision v1:abc --replace"
            )),
            "{fenced}"
        );
    }

    #[test]
    fn a_path_a_shell_would_split_or_expand_is_quoted_in_the_rerun() {
        assert_eq!(shell_word("a b.html"), "'a b.html'");
        assert_eq!(shell_word("it's.html"), "'it'\\''s.html'");
        assert_eq!(shell_word("$HOME/x"), "'$HOME/x'");
        assert_eq!(shell_word("plan-1.2_a/b.html"), "plan-1.2_a/b.html");
        assert_eq!(shell_word(""), "''");
    }
}
