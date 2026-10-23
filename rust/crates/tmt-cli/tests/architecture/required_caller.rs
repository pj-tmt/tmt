//! The #4260 budget guard: the caller-pane check that waits five seconds instead of one runs
//! only for the commands that cannot do without the pane. Everything else (hooks, the digest
//! tick, delivery, identity inference) keeps `caller_pane` and its one second, so a slow tmux can
//! never stall them. The list is exact: a new caller, or one that stops calling, must be reviewed.

use super::source::{Source, production, production_impl, production_trait};
use std::collections::BTreeSet;
use syn::visit::{self, Visit};

/// Production files that call `caller_pane_required`: the host port forwarding it to tmux, and
/// the binding command (`name`, `whoami`, `unbind`, `ls --here` through `labeled_caller_pane`).
pub const CALLERS: &[(&str, &str)] = &[
    ("tmt-adapters", "host.rs"),
    ("tmt-cli", "binding_command.rs"),
];

pub fn violations(sources: &[Source], allowed: &[(&str, &str)]) -> Vec<String> {
    let mut found = BTreeSet::new();
    for source in sources {
        let mut visitor = Visitor { hit: false };
        visitor.visit_file(&source.syntax);
        if visitor.hit {
            found.insert((source.package.clone(), source.file.clone()));
        }
    }
    let listed: BTreeSet<(String, String)> = allowed
        .iter()
        .map(|(package, file)| ((*package).to_owned(), (*file).to_owned()))
        .collect();
    let mut violations: Vec<String> = found
        .difference(&listed)
        .map(|(package, file)| {
            format!(
                "{package}/{file}: calls caller_pane_required (five seconds); only commands that cannot do without the caller's pane may, otherwise use caller_pane"
            )
        })
        .collect();
    violations.extend(listed.difference(&found).map(|(package, file)| {
        format!("{package}/{file}: no longer calls caller_pane_required; remove it from the list")
    }));
    violations
}

struct Visitor {
    hit: bool,
}

impl<'ast> Visit<'ast> for Visitor {
    fn visit_item(&mut self, item: &'ast syn::Item) {
        if production(item) {
            visit::visit_item(self, item);
        }
    }

    fn visit_impl_item(&mut self, item: &'ast syn::ImplItem) {
        if production_impl(item) {
            visit::visit_impl_item(self, item);
        }
    }

    fn visit_trait_item(&mut self, item: &'ast syn::TraitItem) {
        if production_trait(item) {
            visit::visit_trait_item(self, item);
        }
    }

    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        if call.method == "caller_pane_required" {
            self.hit = true;
        }
        visit::visit_expr_method_call(self, call);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(package: &str, file: &str, text: &str) -> Source {
        Source {
            package: package.into(),
            file: file.into(),
            syntax: syn::parse_file(text).unwrap(),
        }
    }

    #[test]
    fn a_new_caller_is_found_and_the_list_stays_exact() {
        let call = "fn f(h: Host) { let _ = h.caller_pane_required(&e); }";
        let sources = [
            source("tmt-cli", "binding_command.rs", call),
            source("tmt-adapters", "host.rs", call),
            source("tmt-cli", "provider_hook_command.rs", call),
            source(
                "tmt-cli",
                "tests_only.rs",
                "#[cfg(test)] fn f(h: Host) { let _ = h.caller_pane_required(&e); }",
            ),
            source(
                "tmt-cli",
                "context_command.rs",
                "fn f(h: Host) { let _ = h.caller_pane(&e); }",
            ),
        ];
        assert_eq!(
            violations(&sources, CALLERS),
            [
                "tmt-cli/provider_hook_command.rs: calls caller_pane_required (five seconds); only commands that cannot do without the caller's pane may, otherwise use caller_pane"
            ]
        );
        assert_eq!(
            violations(&sources[..1], CALLERS),
            ["tmt-adapters/host.rs: no longer calls caller_pane_required; remove it from the list"]
        );
    }
}
