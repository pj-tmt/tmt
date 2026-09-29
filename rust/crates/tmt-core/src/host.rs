//! The terminal hosts that hold agent panes, as pure data (#479). Each host
//! owns the syntax of its pane IDs and pane targets; core stores and compares
//! pane IDs as opaque strings and asks their host about syntax. Only this
//! module and the host adapters spell a host's name.

use crate::names::ecmascript_space;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum HostKind {
    Tmux,
}

impl HostKind {
    pub const ALL: [Self; 1] = [Self::Tmux];

    /// The stored and JSON token.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Tmux => "tmux",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|host| host.as_str() == value)
    }

    /// A pane ID in this host's own syntax.
    pub fn is_pane_id(self, value: &str) -> bool {
        match self {
            Self::Tmux => value.strip_prefix('%').is_some_and(|digits| {
                !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())
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
        }
    }
}

/// tmux's `window.pane`.
fn window_pane(value: &str) -> bool {
    value.split_once('.').is_some_and(|(window, pane)| {
        [window, pane]
            .into_iter()
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
    })
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
    fn tmux_is_the_only_host_and_round_trips_its_token() {
        assert_eq!(HostKind::ALL, [HostKind::Tmux]);
        assert_eq!(HostKind::Tmux.as_str(), "tmux");
        assert_eq!(HostKind::parse("tmux"), Some(HostKind::Tmux));
        for other in ["herdr", "TMUX", "", "tmux "] {
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
}
