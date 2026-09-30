//! The terminal hosts that hold agent panes, as pure data (#479). Each host
//! owns the syntax of its pane IDs and pane targets; core stores and compares
//! pane IDs as opaque strings and asks their host about syntax. Only this
//! module and the host adapters spell a host's name.

use crate::names::ecmascript_space;
use std::sync::OnceLock;
pub use tmt_host_grammar::{HostGrammar, HostName};

/// A terminal host. tmux and Herdr are built in; any other host is named by
/// the driver that serves it (#570). A host is only its name, so stored rows
/// and JSON read without knowing which drivers are installed; whether one is
/// belongs to the adapters that run drivers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum HostKind {
    Tmux,
    Herdr,
    /// A host an out-of-process driver serves. Its syntax is known only
    /// once that driver is registered; until then no pane ID or target is
    /// its own.
    External(HostName),
}

/// At most this many external hosts are registered for a process.
pub const MAX_EXTERNAL_HOSTS: usize = 16;

/// The approved drivers' syntax, written once at start and only read after.
/// Core stays free of other global state; this one is set before the first
/// name is validated, so every reader sees the same hosts.
static EXTERNAL: OnceLock<Vec<HostGrammar>> = OnceLock::new();

/// A second registration: the hosts are fixed for the process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlreadyRegistered;

/// Registers the approved drivers' hosts, once per process, before any name
/// is validated. A declaration that conflicts with a built-in host or an
/// earlier one is skipped, as is anything past [`MAX_EXTERNAL_HOSTS`].
/// Returns the names registered.
pub fn register_external_hosts(
    grammars: Vec<HostGrammar>,
) -> Result<Vec<HostName>, AlreadyRegistered> {
    let mut accepted: Vec<HostGrammar> = Vec::new();
    for grammar in grammars {
        if accepted.len() == MAX_EXTERNAL_HOSTS
            || builtin_conflict(&grammar).is_some()
            || accepted
                .iter()
                .any(|other| grammar.conflict(other).is_some())
        {
            continue;
        }
        accepted.push(grammar);
    }
    let names = accepted
        .iter()
        .filter_map(|grammar| HostName::new(grammar.name()))
        .collect();
    EXTERNAL.set(accepted).map_err(|_| AlreadyRegistered)?;
    Ok(names)
}

fn external_grammars() -> &'static [HostGrammar] {
    EXTERNAL.get().map_or(&[], Vec::as_slice)
}

fn external_grammar(name: HostName) -> Option<&'static HostGrammar> {
    external_grammars()
        .iter()
        .find(|grammar| grammar.name() == name.as_str())
}

/// Why a declared host would be mistaken for a built-in one, if it would: a
/// built-in host's name, or pane IDs or targets a built-in host reads as its
/// own.
pub fn builtin_conflict(grammar: &HostGrammar) -> Option<String> {
    let sample_id = format!("{}1", grammar.pane_id_prefix());
    HostKind::ALL.into_iter().find_map(|host| {
        if grammar.name() == host.as_str() {
            Some(format!("{} is a built-in host", host.as_str()))
        } else if host.is_pane_id(&sample_id) {
            Some(format!("its pane IDs look like {}'s", host.as_str()))
        } else if grammar
            .sample_target()
            .is_some_and(|target| host.is_target(&target))
        {
            Some(format!("its targets look like {}'s", host.as_str()))
        } else {
            None
        }
    })
}

impl HostKind {
    /// The built-in hosts.
    pub const ALL: [Self; 2] = [Self::Tmux, Self::Herdr];

    /// The built-in hosts, then the registered external ones.
    pub fn all() -> impl Iterator<Item = Self> {
        Self::ALL.into_iter().chain(
            external_grammars()
                .iter()
                .filter_map(|grammar| HostName::new(grammar.name()).map(Self::External)),
        )
    }

    /// The stored and JSON token.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Tmux => "tmux",
            Self::Herdr => "herdr",
            Self::External(name) => name.as_str(),
        }
    }

    /// A stored token: a built-in host, or any other valid host name,
    /// whether or not its driver is installed.
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|host| host.as_str() == value)
            .or_else(|| HostName::new(value).map(Self::External))
    }

    /// The host whose pane-ID syntax `id` uses; the syntaxes are disjoint.
    pub fn of_pane_id(id: &str) -> Option<Self> {
        Self::all().find(|host| host.is_pane_id(id))
    }

    /// How a user names a pane seen only by its ID and optional target.
    pub fn label<'a>(id: &'a str, target: Option<&'a str>) -> &'a str {
        Self::of_pane_id(id).map_or(id, |host| host.pane_address(id, target))
    }

    /// A pane ID in this host's own syntax. Herdr's is the terminal ID
    /// (`term_…`): it follows a pane through moves, and a server restart
    /// replaces it, while the public `wN:pM` is reused after a restart.
    pub fn is_pane_id(self, value: &str) -> bool {
        match self {
            Self::Tmux => value.strip_prefix('%').is_some_and(all_digits),
            Self::Herdr => value.strip_prefix("term_").is_some_and(|rest| {
                (1..=MAX_TERMINAL_SUFFIX).contains(&rest.len())
                    && rest
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || byte.is_ascii_lowercase())
            }),
            Self::External(name) => {
                external_grammar(name).is_some_and(|grammar| grammar.is_pane_id(value))
            }
        }
    }

    /// Whether canonical text addresses a pane of this host rather than
    /// naming an identity.
    pub fn is_target(self, canonical: &str) -> bool {
        match self {
            Self::Tmux => {
                if self.is_pane_id(canonical) || window_pane(canonical) {
                    return true;
                }
                canonical.split_once(':').is_some_and(|(session, pane)| {
                    !session.is_empty()
                        && !session.chars().any(ecmascript_space)
                        && window_pane(pane)
                })
            }
            // Herdr's public pane ID; a terminal ID is never a target.
            Self::Herdr => canonical
                .strip_prefix('w')
                .and_then(|rest| rest.split_once(":p"))
                .is_some_and(|(workspace, pane)| all_digits(workspace) && all_digits(pane)),
            Self::External(name) => {
                external_grammar(name).is_some_and(|grammar| grammar.is_target(canonical))
            }
        }
    }

    /// Whether names shaped like this host's targets were refused from the
    /// start. Herdr's `wN:pM` arrived later (#479), as does every external
    /// host: an identity created before may hold such a name, and keeps it.
    pub const fn targets_never_named_identities(self) -> bool {
        matches!(self, Self::Tmux)
    }

    /// How a user names a live pane of this host: its pane ID on tmux, its
    /// public target on Herdr and external hosts, whose IDs are opaque.
    pub fn pane_address<'a>(self, id: &'a str, target: Option<&'a str>) -> &'a str {
        match self {
            Self::Tmux => id,
            Self::Herdr | Self::External(_) => target.unwrap_or(id),
        }
    }
}

const MAX_TERMINAL_SUFFIX: usize = 64;

fn all_digits(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit())
}

/// tmux's `window.pane`.
fn window_pane(value: &str) -> bool {
    value
        .split_once('.')
        .is_some_and(|(window, pane)| all_digits(window) && all_digits(pane))
}

/// One running server of a host that keeps no server-level store (Herdr),
/// named by its socket and its server process's incarnation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostServerIncarnation<'a> {
    pub host: HostKind,
    pub socket_path: &'a str,
    pub server_pid: u64,
    pub server_start_time: &'a str,
}

/// TMT's UUID for a server incarnation, created the first time it is seen.
/// Storage implements it; a host adapter asks before any binding
/// transaction, so the transaction only sees resolved server evidence.
pub trait HostServerIds {
    type Error: std::error::Error + Send + Sync + 'static;

    fn server_id(&mut self, server: &HostServerIncarnation<'_>) -> Result<String, Self::Error>;
}

/// A server as a caller's environment selects it, for presentation priority.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServerSelector<'a> {
    pub host: HostKind,
    pub socket: &'a str,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_host_round_trips_its_token() {
        assert_eq!(HostKind::ALL, [HostKind::Tmux, HostKind::Herdr]);
        for host in HostKind::ALL {
            assert_eq!(HostKind::parse(host.as_str()), Some(host));
        }
        assert_eq!(HostKind::Tmux.as_str(), "tmux");
        assert_eq!(HostKind::Herdr.as_str(), "herdr");
        for other in ["TMUX", "Herdr", "", "tmux ", "9host", "a_b"] {
            assert_eq!(HostKind::parse(other), None, "{other:?}");
        }
    }

    #[test]
    fn any_valid_host_name_reads_as_an_external_host() {
        // No driver is registered in this process: the host still reads and
        // lists, and no pane ID or target is its own.
        let host = HostKind::parse("screen").unwrap();
        assert_eq!(host, HostKind::External(HostName::new("screen").unwrap()));
        assert_eq!(host.as_str(), "screen");
        assert!(!host.is_pane_id("screen-1") && !host.is_target("s1"));
        assert!(!host.targets_never_named_identities());
        assert_eq!(HostKind::all().collect::<Vec<_>>(), HostKind::ALL);
    }

    #[test]
    fn hosts_order_by_name_with_built_ins_first() {
        let mut hosts =
            ["zeta", "herdr", "alpha", "tmux"].map(|name| HostKind::parse(name).unwrap());
        hosts.sort();
        assert_eq!(
            hosts.iter().map(HostKind::as_str).collect::<Vec<_>>(),
            ["tmux", "herdr", "alpha", "zeta"]
        );
    }

    #[test]
    fn a_declaration_like_a_built_in_host_is_refused() {
        let conflict = |name, prefix, target| {
            builtin_conflict(&HostGrammar::new(name, prefix, target).unwrap())
        };
        assert!(conflict("tmux", "tm-", None).is_some());
        assert!(conflict("other", "term_", None).is_some(), "Herdr's IDs");
        assert!(
            conflict("other", "ot-", Some("w{n}:p{n}")).is_some(),
            "Herdr's targets"
        );
        assert!(conflict("other", "ot-", Some("s{n}")).is_none());
    }

    #[test]
    fn tmux_owns_its_pane_id_and_target_syntax() {
        for id in ["%0", "%14"] {
            assert!(HostKind::Tmux.is_pane_id(id), "{id}");
        }
        for not_id in ["%", "14", "%1a", "%-1", "w1:p1", ""] {
            assert!(!HostKind::Tmux.is_pane_id(not_id), "{not_id}");
        }
        for target in ["%3", "1.2", "work:1.2", "my-session:0.0"] {
            assert!(HostKind::Tmux.is_target(target), "{target}");
        }
        for not_target in ["worker", "1.", ".2", ":1.2", "a b:1.2", "w1:p1", "1.2.3"] {
            assert!(!HostKind::Tmux.is_target(not_target), "{not_target}");
        }
    }

    #[test]
    fn herdr_owns_its_terminal_id_and_public_pane_syntax() {
        for id in [
            "term_65ca1161edc141",
            "term_0",
            &format!("term_{}", "a".repeat(64)),
        ] {
            assert!(HostKind::Herdr.is_pane_id(id), "{id}");
        }
        for not_id in [
            "term_",
            "term_65CA",
            "term_6-5",
            "w1:p1",
            "%1",
            "",
            &format!("term_{}", "a".repeat(65)),
        ] {
            assert!(!HostKind::Herdr.is_pane_id(not_id), "{not_id}");
        }
        for target in ["w1:p1", "w12:p340"] {
            assert!(HostKind::Herdr.is_target(target), "{target}");
        }
        for not_target in [
            "w1:p",
            "w:p1",
            "w1p1",
            "1:p1",
            "w1:p1:",
            "w+1:p1",
            "worker",
            "term_65ca1161edc141",
            "%3",
            "1.2",
        ] {
            assert!(!HostKind::Herdr.is_target(not_target), "{not_target}");
        }
        // No text is a target of both hosts.
        for text in ["%3", "1.2", "s:1.2", "w1:p1"] {
            let hosts = HostKind::ALL
                .into_iter()
                .filter(|host| host.is_target(text));
            assert_eq!(hosts.count(), 1, "{text}");
        }
    }

    #[test]
    fn a_herdr_pane_is_addressed_by_its_public_id() {
        assert_eq!(HostKind::Tmux.pane_address("%3", Some("s:1.2")), "%3");
        assert_eq!(
            HostKind::Herdr.pane_address("term_1", Some("w1:p2")),
            "w1:p2"
        );
        assert_eq!(HostKind::Herdr.pane_address("term_1", None), "term_1");
        assert_eq!(HostKind::of_pane_id("%3"), Some(HostKind::Tmux));
        assert_eq!(HostKind::of_pane_id("term_1"), Some(HostKind::Herdr));
        assert_eq!(HostKind::of_pane_id("w1:p2"), None);
        assert_eq!(HostKind::label("term_1", Some("w1:p2")), "w1:p2");
        assert_eq!(HostKind::label("%3", Some("s:1.2")), "%3");
    }
}
