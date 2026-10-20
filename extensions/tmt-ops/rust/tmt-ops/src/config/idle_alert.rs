//! `[squad.<name>.idle_alert]`: the opt-in notice to a squad's lead about a member that
//! sits idle too long. This module owns the two settings; `crate::idle_alert` decides.
use super::sourced::Sources;
use super::*;

/// Per-squad idle-alert policy. Off in every layout: the alert sends messages to an agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IdleAlert {
    pub enabled: bool,
    pub after: Duration,
}

impl Default for IdleAlert {
    fn default() -> Self {
        Self {
            enabled: false,
            after: Duration::from_secs(1800),
        }
    }
}

impl Config {
    /// `[squad.<name>.idle_alert]`, off unless the squad switches it on.
    pub fn idle_alert(&self, squad: &str) -> Result<IdleAlert, SquadError> {
        let place = format!("squad.{squad}.idle_alert");
        let mut setting = IdleAlert::default();
        let Some(item) = self
            .squad_table(squad)?
            .and_then(|table| table.get("idle_alert"))
        else {
            return Ok(setting);
        };
        let table = item
            .as_table_like()
            .ok_or_else(|| invalid(format!("`{place}` must be a table.")))?;
        for (key, item) in table.iter() {
            match key {
                "enabled" => {
                    setting.enabled = item.as_bool().ok_or_else(|| {
                        invalid(format!("`{place}.enabled` must be true or false."))
                    })?
                }
                "after" => {
                    let wrong = || {
                        invalid(format!(
                            "`{place}.after` must be 5m-24h in whole s/m/h units, such as \"30m\"."
                        ))
                    };
                    let text = item.as_str().ok_or_else(wrong)?;
                    setting.after = duration(text, &['s', 'm', 'h'])
                        .filter(|_| text.as_bytes().first().is_some_and(u8::is_ascii_digit))
                        .filter(|after| (300..=86400).contains(&after.as_secs()))
                        .ok_or_else(wrong)?;
                }
                _ => {
                    return Err(invalid(format!(
                        "`{place}.{key}` is not an idle alert setting; use enabled and after."
                    )));
                }
            }
        }
        Ok(setting)
    }

    pub(super) fn idle_alert_settings(
        &self,
        squad: &str,
    ) -> Result<(IdleAlert, Sources), SquadError> {
        let source =
            |key: &str, fallback: &str| self.source(&["squad", squad, "idle_alert", key], fallback);
        Ok((
            self.idle_alert(squad)?,
            Sources::from([
                ("enabled".into(), source("enabled", "default:off")),
                ("after".into(), source("after", "default:30m")),
            ]),
        ))
    }

    /// Squads that switched the alert on, so a squad with it off costs no Core read.
    /// An invalid table still lists its squad; `idle_alert` then reports why.
    pub fn idle_alert_squads(&self) -> Vec<String> {
        self.document
            .get("squad")
            .and_then(Item::as_table_like)
            .into_iter()
            .flat_map(|squads| squads.iter())
            .filter(|(_, squad)| {
                squad
                    .get("idle_alert")
                    .and_then(|alert| alert.get("enabled"))
                    .and_then(Item::as_bool)
                    == Some(true)
            })
            .map(|(name, _)| name.to_owned())
            .collect()
    }
}

#[cfg(test)]
mod tests;
