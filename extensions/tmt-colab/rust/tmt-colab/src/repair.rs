//! Inert formatting for runnable command repairs. Never executes the resulting text.
pub fn argument(value: &str) -> String {
    if !value.is_empty()
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
    {
        value.to_owned()
    } else {
        format!("'{}'", value.replace('\'', "'\\''"))
    }
}

/// A named CLI value beginning with a hyphen needs equals syntax as well as shell quoting.
pub fn option(name: &str, value: &str) -> String {
    format!(
        "--{name}{}{}",
        if value.starts_with('-') { "=" } else { " " },
        argument(value)
    )
}

#[cfg(test)]
mod tests {
    use super::argument;
    #[test]
    fn repairs_keep_untrusted_text_inside_one_shell_argument() {
        assert_eq!(argument("gui-3"), "gui-3");
        assert_eq!(super::option("agent", "-worker"), "--agent=-worker");
        assert_eq!(super::option("title", "-a b"), "--title='-a b'");
        assert_eq!(argument(""), "''");
        assert_eq!(argument("a'b"), "'a'\\''b'");
        assert_eq!(argument("$(touch x); `id`"), "'$(touch x); `id`'");
    }
}
