//! Acquisition for the cron projection; runs on the refresh worker only.

use super::{Cron, Places};
use crate::{
    config::Config,
    core::Core,
    cron_service::{self as service, JobKey},
};
use tmt_ops::cron::Clock;

/// What the worker needs to read; carries its own config so the read matches
/// the load that scheduled it.
pub struct Fetch {
    pub config: Config,
}

fn order(id: &str) -> u64 {
    id.trim_start_matches('c').parse().unwrap_or(u64::MAX)
}

pub fn fetch(core: &Core, job: Fetch, now_ms: i64, places: &mut Places) -> Result<Cron, String> {
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
        .unwrap_or(tmt_ops::cron::ClockStatus::Unknown);
    let place = match &clock {
        tmt_ops::cron::ClockStatus::Running(holder) => {
            holder.pane.as_deref().and_then(|pane| places.resolve(pane))
        }
        _ => None,
    };
    Ok(Cron {
        jobs,
        clock,
        place,
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
            &mut Places::new(None),
        )
        .unwrap();
        let ids: Vec<_> = read
            .jobs
            .iter()
            .map(|view| JobKey::of(&view.job).id)
            .collect();
        assert_eq!(ids[1], "c2");
        assert_eq!(ids[10], "c11", "ids order numerically, not as text");
        assert_eq!(read.clock, tmt_ops::cron::ClockStatus::NoClock);
        assert_eq!(read.actor.as_ref().unwrap().id, USER);
        assert!(read.next().is_some());
        assert_eq!(
            read.next_of(WORKER).map(|(at, _)| at),
            read.next().map(|(at, _)| at)
        );
    }

    #[test]
    fn the_holders_pane_resolves_through_places_and_unresolved_keeps_the_pane_id() {
        let f = Fixture::new();
        let root = service::root(&f.core).unwrap();
        let lease = Clock::new(&root)
            .unwrap()
            .acquire(0, 7, Some("%7".into()))
            .unwrap()
            .expect("lease");
        let read = |places: &mut Places| {
            fetch(
                &f.core,
                Fetch {
                    config: f.config.clone(),
                },
                1,
                places,
            )
            .unwrap()
        };
        let dir = std::env::temp_dir().join(format!("tmt-ops-load-place-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let program = dir.join("tmux");
        crate::test_support::write_ready_executable(&program, "#!/bin/sh\necho 'team:agents'\n");
        let read_in = read(&mut Places::with_program(program, Some("/sock".into())));
        assert_eq!(read_in.place.as_deref(), Some("team:agents"));
        // Outside tmux the read still shows the holder, with no place.
        let outside = read(&mut Places::new(None));
        assert!(matches!(
            outside.clock,
            tmt_ops::cron::ClockStatus::Running(_)
        ));
        assert_eq!(outside.place, None);
        drop(lease);
        let _ = std::fs::remove_dir_all(dir);
    }
}
