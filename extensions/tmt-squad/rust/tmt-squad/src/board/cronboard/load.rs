//! Acquisition for the cron projection; runs on the refresh worker only.

use super::Cron;
use crate::{
    config::Config,
    core::Core,
    cron_service::{self as service, JobKey},
};
use tmt_squad::cron::Clock;

/// What the worker needs to read; carries its own config so the read matches
/// the load that scheduled it.
pub struct Fetch {
    pub config: Config,
}

fn order(id: &str) -> u64 {
    id.trim_start_matches('c').parse().unwrap_or(u64::MAX)
}

pub fn fetch(core: &Core, job: Fetch, now_ms: i64) -> Result<Cron, String> {
    let listed =
        service::list_jobs(core, &job.config, None, now_ms).map_err(|error| error.to_string())?;
    let mut jobs = listed.jobs;
    jobs.sort_by_key(|view| {
        let key = JobKey::of(&view.job);
        (key.squad, order(&key.id))
    });
    let clock = service::root(core)
        .and_then(|root| Ok(Clock::new(&root)?))
        .map(|clock| clock.status(now_ms))
        .unwrap_or(tmt_squad::cron::ClockStatus::Unknown);
    Ok(Cron {
        jobs,
        clock,
        actor: service::actor(core, &job.config, None).map_err(|error| error.to_string()),
        read_ms: now_ms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cron_service::test_support::*;

    #[test]
    fn reads_every_squad_in_stable_order_with_clock_and_actor() {
        let f = Fixture::new();
        let lead = f.actor(LEAD);
        for _ in 0..11 {
            f.add(&lead, WORKER).unwrap();
        }
        let read = fetch(
            &f.core,
            Fetch {
                config: f.config.clone(),
            },
            0,
        )
        .unwrap();
        let ids: Vec<_> = read
            .jobs
            .iter()
            .map(|view| JobKey::of(&view.job).id)
            .collect();
        assert_eq!(ids[1], "c2");
        assert_eq!(ids[10], "c11", "ids order numerically, not as text");
        assert_eq!(read.clock, tmt_squad::cron::ClockStatus::NoClock);
        assert_eq!(read.actor.as_ref().unwrap().id, USER);
        assert!(read.next().is_some());
        assert_eq!(
            read.next_of(WORKER).map(|(at, _)| at),
            read.next().map(|(at, _)| at)
        );
    }
}
