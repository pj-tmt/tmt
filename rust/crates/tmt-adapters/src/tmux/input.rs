//! Recent real client keys, independent of pane output or application buffers.
use super::{OperationOptions, Tmux, TmuxError, TmuxFailure, evidence, socket_args};
use crate::process::CommandRunner;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tmt_core::driver::InputActivity;

impl<R: CommandRunner> Tmux<R> {
    pub(super) fn input_activity(
        &self,
        socket: &str,
        pane: &str,
        options: OperationOptions<'_>,
    ) -> Result<InputActivity, TmuxError> {
        let mut args = socket_args(Some(socket));
        args.extend([
            "list-clients".into(),
            "-F".into(),
            format!("#{{pane_id}}{}#{{client_activity}}", evidence::SEPARATOR),
        ]);
        let output = self.execute(args, options, TmuxFailure::Command)?;
        Ok(SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(InputActivity::Unknown, |now| parse(&output, pane, now)))
    }
}

fn parse(output: &str, pane: &str, now: Duration) -> InputActivity {
    let mut latest = None;
    for line in output.lines().filter(|line| !line.trim().is_empty()) {
        let Some((current, activity)) = line.split_once(evidence::SEPARATOR) else {
            return InputActivity::Unknown;
        };
        if !super::valid_pane_id(current) {
            return InputActivity::Unknown;
        }
        if current != pane {
            continue;
        }
        let Some(activity) = evidence::wire_integer(activity)
            .filter(|activity| *activity > 0 && *activity <= now.as_secs())
        else {
            // Clock rollback, unsupported formats and malformed evidence do not
            // prove ongoing typing or authorize an indefinite deferral.
            return InputActivity::Unknown;
        };
        latest = Some(latest.map_or(activity, |previous: u64| previous.max(activity)));
    }
    let Some(latest) = latest else {
        return InputActivity::Unknown;
    };
    let Ok(now_ms) = u64::try_from(now.as_millis()) else {
        return InputActivity::Unknown;
    };
    // tmux records whole seconds. The last key could have been at the very end
    // of that second: use its latest possible time so a quiet period is never
    // shortened by rounding, at the cost of at most one extra second.
    let Some(latest_ms) = latest
        .checked_mul(1000)
        .and_then(|value| value.checked_add(999))
    else {
        return InputActivity::Unknown;
    };
    InputActivity::ElapsedMs(now_ms.saturating_sub(latest_ms))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn row(pane: &str, activity: &str) -> String {
        format!("{pane}{}{activity}\n", evidence::SEPARATOR)
    }
    #[test]
    fn client_evidence_uses_the_binding_socket_and_existing_budget_without_input() {
        use crate::scripted_runner::ScriptedRunner;
        use std::time::Instant;
        let runner = ScriptedRunner::default();
        runner.push_output(row("%1", "1").into_bytes(), Vec::new());
        let tmux = Tmux::new(runner);
        let deadline = Instant::now() + Duration::from_millis(200);
        assert!(matches!(
            tmux.input_activity(
                "/tmp/reply-notice-owned.sock",
                "%1",
                OperationOptions {
                    deadline: Some(deadline),
                    pane_ids: None,
                }
            )
            .unwrap(),
            InputActivity::ElapsedMs(_)
        ));
        let calls = tmux.runner.calls.borrow();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].program, "tmux");
        assert_eq!(
            calls[0].args,
            [
                "-S",
                "/tmp/reply-notice-owned.sock",
                "list-clients",
                "-F",
                &format!("#{{pane_id}}{}#{{client_activity}}", evidence::SEPARATOR)
            ]
        );
        assert_eq!(calls[0].deadline, deadline);
        assert!(calls[0].input.is_empty());
    }

    #[test]
    fn only_viewers_of_the_target_pane_supply_key_activity() {
        let now = Duration::from_millis(10_500);
        assert_eq!(parse("", "%1", now), InputActivity::Unknown);
        assert_eq!(parse(&row("%2", "10"), "%1", now), InputActivity::Unknown);
        assert_eq!(
            parse(&(row("%1", "5") + &row("%2", "10")), "%1", now),
            InputActivity::ElapsedMs(4501)
        );
        assert_eq!(
            parse(&(row("%1", "5") + &row("%1", "9")), "%1", now),
            InputActivity::ElapsedMs(501)
        );
        assert_eq!(
            parse(&row("%1", "10"), "%1", now),
            InputActivity::ElapsedMs(0)
        );
    }
    #[test]
    fn unknown_or_future_activity_never_claims_typing() {
        for text in [
            row("%1", ""),
            row("%1", "junk"),
            row("%1", "0"),
            row("%1", "11"),
            "missing fields".into(),
        ] {
            assert_eq!(
                parse(&text, "%1", Duration::from_secs(10)),
                InputActivity::Unknown
            );
        }
    }
}
