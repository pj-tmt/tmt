use super::*;

#[cfg(test)]
mod tests;

pub(super) trait ObserverRuntime {
    fn now(&self) -> Instant;
    fn interrupted(&self) -> bool;
    fn wait_until(&self, deadline: Instant) -> io::Result<()>;
}

impl ObserverRuntime for Interrupt {
    fn now(&self) -> Instant {
        Instant::now()
    }
    fn interrupted(&self) -> bool {
        self.is_interrupted()
    }
    fn wait_until(&self, deadline: Instant) -> io::Result<()> {
        Interrupt::wait_until(self, deadline)
    }
}

pub(super) fn observe(
    mut read: impl FnMut() -> Result<Option<FinalResponse>, Failure>,
    correlation: &Correlation,
    deadline: Instant,
    timeout: f64,
    poll_seconds: f64,
    runtime: &impl ObserverRuntime,
) -> Result<FinalResponse, Failure> {
    let timed_out = || {
        correlation
            .error(
                "TIMEOUT",
                format!(
                    "Timed out waiting for {} after {timeout}s{}",
                    correlation.target,
                    if correlation.delivery_uncertain {
                        "; delivery was uncertain (the channel gave no receipt), so do not resend"
                    } else {
                        ""
                    }
                ),
                4,
            )
            .with_request(correlation.request_id.clone(), Some("timeout"))
            .suggestion(correlation.inspection())
    };
    loop {
        if runtime.interrupted() {
            return Err(correlation.interrupted());
        }
        if runtime.now() >= deadline {
            return Err(timed_out());
        }
        let response = read()?;
        if runtime.interrupted() {
            return Err(correlation.interrupted());
        }
        if runtime.now() >= deadline {
            return Err(timed_out());
        }
        if let Some(response) = response {
            return Ok(response);
        }
        let now = runtime.now();
        let remaining = deadline.saturating_duration_since(now);
        // The reference timer has millisecond granularity. Positive sub-ms
        // intervals must not become a zero-duration CPU spin in native code.
        let poll_ms = (poll_seconds.min(timeout) * 1000.0).ceil().max(1.0) as u64;
        let wake = now + Duration::from_millis(poll_ms).min(remaining);
        runtime.wait_until(wake).map_err(|error| {
            correlation
                .error("ERROR", "Could not wait for a durable reply.", 1)
                .caused_by(error)
        })?;
    }
}
