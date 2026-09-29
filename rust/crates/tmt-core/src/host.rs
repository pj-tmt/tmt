//! The terminal hosts that hold agent panes, as pure data (#479). Each host
//! owns the syntax of its pane IDs and pane targets; core stores and compares
//! pane IDs as opaque strings and asks their host about syntax. Only this
//! module and the host adapters spell a host's name.

use crate::names::ecmascript_space;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum HostKind {
    Tmux,
    Herdr,
}

impl HostKind {
    pub const ALL: [Self; 2] = [Self::Tmux, Self::Herdr];

    /// The stored and JSON token.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Tmux => "tmux",
            Self::Herdr => "herdr",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|host| host.as_str() == value)
    }

    /// The host whose pane-ID syntax `id` uses; the syntaxes are disjoint.
    pub fn of_pane_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|host| host.is_pane_id(id))
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
        }
    }

    /// Whether names shaped like this host's targets were refused from the
    /// start. Herdr's `wN:pM` arrived later (#479): an identity created
    /// before may hold such a name, and keeps it.
    pub const fn targets_never_named_identities(self) -> bool {
        match self {
            Self::Tmux => true,
            Self::Herdr => false,
        }
    }

    /// How a user names a live pane of this host: its pane ID on tmux, its
    /// public `wN:pM` on Herdr, whose terminal IDs are opaque.
    pub fn pane_address<'a>(self, id: &'a str, target: Option<&'a str>) -> &'a str {
        match self {
            Self::Tmux => id,
            Self::Herdr => target.unwrap_or(id),
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
        for other in ["screen", "TMUX", "Herdr", "", "tmux "] {
            assert_eq!(HostKind::parse(other), None, "{other:?}");
        }
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
