//! Inert issue references share prose styling; they never create an action.
use super::*;

fn issue_label(text: &str) -> Option<String> {
    let path = text
        .strip_prefix("https://github.com/")
        .or_else(|| text.strip_prefix("http://github.com/"))?;
    let path = path.split(['?', '#']).next()?;
    let mut parts = path.trim_end_matches('/').split('/');
    let (owner, repo, kind, number) = (parts.next()?, parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some()
        || !matches!(kind, "issues" | "pull")
        || number.is_empty()
        || !number.bytes().all(|byte| byte.is_ascii_digit())
        || [owner, repo].iter().any(|part| {
            part.is_empty()
                || !part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        })
    {
        return None;
    }
    Some(format!("{owner}/{repo}#{number}"))
}

impl Renderer<'_> {
    pub(super) fn prose(&mut self, text: &str) {
        let style = self.style();
        let link = style
            .patch(self.look.role(Role::Text))
            .add_modifier(Modifier::UNDERLINED);
        let mut emitted = 0;
        let mut offset = 0;
        for word in text.split_inclusive(char::is_whitespace) {
            let token = word
                .trim_end_matches(char::is_whitespace)
                .trim_end_matches(['.', ',', '!', ')', ']', ':', ';']);
            let label = issue_label(token);
            let numbered = token.strip_prefix('#').is_some_and(|number| {
                !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit())
            });
            if label.is_some() || numbered {
                if offset > emitted {
                    self.push(&text[emitted..offset], style);
                }
                self.push(label.as_deref().unwrap_or(token), link);
                emitted = offset + token.len();
            }
            offset += word.len();
        }
        if emitted < text.len() || text.is_empty() {
            self.push(&text[emitted..], style);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn issue_urls_shorten_only_for_the_exact_github_host_and_issue_paths() {
        assert_eq!(
            issue_label("https://github.com/owner/repo/pull/42?view=split"),
            Some("owner/repo#42".into())
        );
        for value in [
            "https://github.com.evil/o/r/pull/42",
            "https://github.com/o/r/blob/42",
            "https://github.com/o/r/issues/no",
            "javascript:x",
        ] {
            assert_eq!(issue_label(value), None, "{value}");
        }
    }
    #[test]
    fn prose_issue_labels_are_neutral_underlined_and_code_is_not_shortened() {
        let look = crate::look::Look::default();
        let mapped = render_mapped(
            "See #42, <https://github.com/o/r/issues/7>. `https://github.com/o/r/pull/9`",
            100,
            look,
        );
        let all: String = mapped.lines.iter().map(|line| line.to_string()).collect();
        assert!(all.contains("#42, o/r#7."), "{all}");
        assert!(all.contains("https://github.com/o/r/pull/9"));
        for text in ["#42", "o/r#7"] {
            let span = mapped
                .lines
                .iter()
                .flat_map(|line| &line.spans)
                .find(|span| span.content.contains(text))
                .unwrap();
            assert_eq!(span.style.fg, look.role(Role::Text).fg);
            assert!(span.style.add_modifier.contains(Modifier::UNDERLINED));
        }
    }
}
