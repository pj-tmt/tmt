//! The #491 host guard: production code spells a terminal host's name only in
//! its core descriptor (`tmt-core/src/host.rs`), the host port
//! (`tmt-adapters/src/host.rs`), the host's own adapter module (`tmux/`,
//! `herdr/`) and its out-of-process driver crate (`tmt-driver-herdr`), which
//! reaches TMT only through the protocol and so has no `HostKind`. Human prose
//! that mentions a host is not a literal equal to its name and stays.
//!
//! Squad is outside this guard: its hotkeys and clipboard are tmux-only
//! extension features that it reaches through tmux itself, and the Herdr
//! slices revisit them (#479 H6).

use super::{driver_names::owned_literals, source::Source};

const OWNERS: [(&str, &str); 6] = [
    ("tmt-core", "host.rs"),
    ("tmt-adapters", "host.rs"),
    ("tmt-adapters", "tmux/"),
    ("tmt-adapters", "herdr/"),
    ("tmt-driver-herdr", ""),
    ("tmt-squad", ""),
];

pub fn violations(sources: &[Source], names: &[&str]) -> Vec<String> {
    owned_literals(sources, names, &OWNERS, |name| {
        format!("names the host {name:?}; use tmt_core::host::HostKind instead")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(package: &str, file: &str, text: &str) -> Source {
        Source {
            package: package.into(),
            file: file.into(),
            syntax: syn::parse_file(text).expect("fixture parses"),
        }
    }

    #[test]
    fn a_host_name_outside_its_owners_is_a_violation() {
        let text = r#"
            fn driver() -> &'static str { "hostx" }
            fn prose() -> &'static str { "No hostx client can be focused" }
            #[cfg(test)]
            mod tests { fn fixture() -> &'static str { "hostx" } }
        "#;
        assert_eq!(
            violations(
                &[source("tmt-cli", "binding_command/presentation.rs", text)],
                &["hostx"]
            ),
            [
                r#"tmt-cli/binding_command/presentation.rs: names the host "hostx"; use tmt_core::host::HostKind instead"#
            ]
        );
        for (package, file) in [
            ("tmt-core", "host.rs"),
            ("tmt-adapters", "host.rs"),
            ("tmt-adapters", "tmux/mod.rs"),
            ("tmt-adapters", "herdr/mod.rs"),
            ("tmt-squad", "effects.rs"),
        ] {
            assert!(
                violations(&[source(package, file, text)], &["hostx"]).is_empty(),
                "{package}/{file}"
            );
        }
        assert_eq!(
            violations(&[source("tmt-adapters", "delivery.rs", text)], &["hostx"]).len(),
            1
        );
    }
}
