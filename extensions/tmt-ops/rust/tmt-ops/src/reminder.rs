//! Optional next-turn context. The cache-only gate precedes all core calls;
//! current config, room and leadership are revalidated before claiming.

use crate::{
    config::Config,
    core::Core,
    observe,
    squad::{Member, Squad},
    staleness, status,
};
use nix::{
    sys::signal::{Signal, killpg},
    unistd::{getpgrp, getpid},
};
use std::{
    sync::{Arc, Condvar, Mutex},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const BUDGET: Duration = Duration::from_millis(300);

/// Core starts this hook in its own process group. Keep every nested call in
/// it, so core's earlier deadline also kills descendants. The local deadline
/// covers input, cache reads, publication and stdout, not just subprocesses.
pub struct Budget {
    pub deadline: Instant,
    done: Arc<(Mutex<bool>, Condvar)>,
    timer: Option<JoinHandle<()>>,
}

impl Budget {
    pub fn start() -> Option<Self> {
        let owner = getpid();
        if getpgrp() != owner {
            return None; // Never signal a direct caller's interactive group.
        }
        let deadline = Instant::now() + BUDGET;
        let done = Arc::new((Mutex::new(false), Condvar::new()));
        let waiting = Arc::clone(&done);
        let timer = thread::Builder::new()
            .name("squad-context-budget".into())
            .spawn(move || {
                let (lock, wake) = &*waiting;
                let (done, _) = wake
                    .wait_timeout_while(
                        lock.lock().expect("budget lock"),
                        deadline.saturating_duration_since(Instant::now()),
                        |done| !*done,
                    )
                    .expect("budget wait");
                if !*done {
                    // The owner is still alive and cannot have a recycled PID.
                    // This group contains only this hook and its nested calls.
                    let _ = killpg(owner, Signal::SIGKILL);
                }
            })
            .ok()?;
        Some(Self {
            deadline,
            done,
            timer: Some(timer),
        })
    }
}

impl Drop for Budget {
    fn drop(&mut self) {
        let (lock, wake) = &*self.done;
        *lock.lock().expect("budget lock") = true;
        wake.notify_one();
        if let Some(timer) = self.timer.take() {
            let _ = timer.join();
        }
    }
}

pub fn summary(lead: &str, deadline: Instant) -> Option<String> {
    let candidates = staleness::candidates(lead, status::now_ms(), deadline);
    if candidates.is_empty() || Instant::now() >= deadline {
        return None;
    }
    let core = Core::discover().ok()?.until(deadline);
    let config = Config::load(&core).ok()?;
    for candidate in candidates {
        if Instant::now() >= deadline {
            return None;
        }
        if candidate.config != config.path() {
            continue;
        }
        let settings = config.reminders(&candidate.squad.name).ok()?;
        if !settings.enabled {
            continue;
        }
        let squad = Squad::resolve(&core, Some(&candidate.squad.name)).ok()?;
        if squad.room_id != candidate.squad.room_id {
            continue;
        }
        let providers = config.providers(&squad.name).ok()?;
        let observed = observe::observe(
            &core,
            config.path(),
            &squad,
            settings,
            &providers,
            observe::Mode::Reminder { lead },
        )
        .ok()?;
        if let Some(claim) = observed.reminder {
            // Every claimed item is represented by a name or the remainder
            // count. Never truncate a line after committing its claims.
            return (Instant::now() < deadline)
                .then(|| line(&squad.name, &claim, &observed.members));
        }
    }
    None
}

fn line(squad: &str, claim: &staleness::Reminder, members: &[Member]) -> String {
    let mut parts = Vec::new();
    if claim.notes {
        parts.push("stale lead notes".to_owned());
    }
    if !claim.members.is_empty() {
        let names = claim
            .members
            .iter()
            .take(2)
            .map(|id| {
                let name = members
                    .iter()
                    .find(|member| &member.id == id)
                    .map_or(id.as_str(), |member| member.name.as_str());
                sanitized(name)
            })
            .collect::<Vec<_>>()
            .join(", ");
        let more = claim.members.len().saturating_sub(2);
        parts.push(format!(
            "{} stale member rows: {names}{}",
            claim.members.len(),
            if more == 0 {
                String::new()
            } else {
                format!(" (+{more} others)")
            }
        ));
    }
    format!(
        "Squad {squad}: {}. Review notes/task/state.",
        parts.join("; ")
    )
}

fn sanitized(text: &str) -> String {
    let text: String = text.chars().map(|ch| {
        if ch.is_control() || matches!(ch, '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{206f}' | '\u{feff}') {
            ' '
        } else { ch }
    }).collect();
    let visible = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let name: String = visible.chars().take(40).collect();
    if name.is_empty() {
        "unnamed member".into()
    } else {
        name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_represents_every_claim_and_sanitizes_unicode_names_within_the_bound() {
        let members = Vec::new();
        let claim = staleness::Reminder {
            members: vec![
                "悪\n\u{1b}[0m\u{202e}".repeat(80),
                "β".repeat(80),
                "third".into(),
            ],
            notes: true,
        };
        let result = line(&"s".repeat(24), &claim, &members);
        assert!(result.chars().count() <= 240);
        assert!(!result.chars().any(char::is_control));
        assert!(!result.contains('\u{202e}'));
        assert!(result.contains("stale lead notes"));
        assert!(result.contains("3 stale member rows"));
        assert!(result.contains("+1 others"));
        assert_eq!(sanitized("\n\u{202e}"), "unnamed member");
    }
}
