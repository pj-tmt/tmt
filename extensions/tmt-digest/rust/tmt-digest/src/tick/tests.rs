use super::*;
use std::{cell::Cell, rc::Rc};

struct Clock {
    time: Rc<Cell<u64>>,
    elapsed: u64,
}
impl TickClock for Clock {
    fn now_ms(&self) -> Result<u64, Error> {
        Ok(self.time.get())
    }
    fn elapsed_ms(&self) -> u64 {
        self.elapsed
    }
    fn sleep_ms(&mut self, ms: u64) {
        self.time.set(self.time.get() + ms);
        self.elapsed += ms;
    }
}
struct Port {
    time: Rc<Cell<u64>>,
    held: bool,
    threshold: bool,
    off_at: Option<u64>,
    delivered: Vec<u64>,
}
impl TickPort for Port {
    fn observe(&mut self) -> Result<Vec<TickObservation>, Error> {
        Ok(vec![TickObservation {
            identity_id: "member".into(),
            mode: if self.off_at.is_some_and(|at| self.time.get() >= at) {
                Mode::Off
            } else {
                Mode::parse("20s").unwrap()
            },
            flush_count: 10,
            held_count: if !self.held {
                0
            } else if self.threshold {
                10
            } else {
                1
            },
            oldest_held_at_ms: self.held.then_some(60_000),
        }])
    }
    fn deliver_if_current(&mut self, _: &str) -> Result<(), Error> {
        self.delivered.push(self.time.get());
        self.held = false;
        Ok(())
    }
}
fn fixture() -> (Clock, Port) {
    let time = Rc::new(Cell::new(60_000));
    (
        Clock {
            time: time.clone(),
            elapsed: 0,
        },
        Port {
            time,
            held: true,
            threshold: false,
            off_at: None,
            delivered: vec![],
        },
    )
}

#[test]
fn subminute_deadline_is_oldest_held_age_and_empty_work_has_no_deadline() {
    let (mut clock, mut port) = fixture();
    run(&mut clock, &mut port).unwrap();
    assert_eq!(port.delivered, [80_000]);
    assert_eq!(clock.time.get(), 120_000);
    assert!(
        port.observe().unwrap()[0]
            .deadline(clock.time.get())
            .is_none()
    );
}

#[test]
fn threshold_flushes_now_and_off_midinterval_prevents_delivery() {
    let (mut clock, mut port) = fixture();
    port.threshold = true;
    run(&mut clock, &mut port).unwrap();
    assert_eq!(port.delivered, [60_000]);
    let (mut clock, mut port) = fixture();
    port.off_at = Some(70_000);
    run(&mut clock, &mut port).unwrap();
    assert!(port.delivered.is_empty());
}

#[test]
fn restart_needs_no_last_sent_state_and_auto_has_no_interval_deadline() {
    let (mut clock, mut port) = fixture();
    clock.time.set(100_000);
    run(&mut clock, &mut port).unwrap();
    assert_eq!(port.delivered, [100_000]);
    let row = TickObservation {
        identity_id: "member".into(),
        mode: Mode::Auto,
        flush_count: 10,
        held_count: 50,
        oldest_held_at_ms: Some(60_000),
    };
    assert_eq!(row.deadline(100_000), None);
}

#[test]
fn thirty_due_members_are_serialized_and_no_delivery_starts_after_the_boundary() {
    struct Batch {
        time: Rc<Cell<u64>>,
        pending: bool,
        delivered: Vec<(String, u64)>,
    }
    impl TickPort for Batch {
        fn observe(&mut self) -> Result<Vec<TickObservation>, Error> {
            Ok(if self.pending {
                (0..30)
                    .map(|i| TickObservation {
                        identity_id: i.to_string(),
                        mode: Mode::parse("20s").unwrap(),
                        flush_count: 10,
                        held_count: 10,
                        oldest_held_at_ms: Some(60_000),
                    })
                    .collect()
            } else {
                vec![]
            })
        }
        fn deliver_if_current(&mut self, id: &str) -> Result<(), Error> {
            self.delivered.push((id.into(), self.time.get()));
            self.time.set(self.time.get() + 600);
            self.pending = false;
            Ok(())
        }
    }
    let (mut clock, _) = fixture();
    let mut port = Batch {
        time: clock.time.clone(),
        pending: true,
        delivered: vec![],
    };
    run(&mut clock, &mut port).unwrap();
    assert_eq!(port.delivered.len(), 30);
    assert_eq!(port.delivered.first().unwrap().1, 60_000);
    assert_eq!(port.delivered.last().unwrap().1, 77_400);
    let (mut clock, _) = fixture();
    clock.time.set(119_000);
    let mut port = Batch {
        time: clock.time.clone(),
        pending: true,
        delivered: vec![],
    };
    run(&mut clock, &mut port).unwrap();
    assert_eq!(port.delivered.len(), 2);
    assert!(port.delivered.iter().all(|(_, started)| *started < 120_000));
}

#[test]
fn rollback_does_not_extend_the_monotonic_budget_and_started_work_may_finish() {
    struct Crossing {
        time: Rc<Cell<u64>>,
        delivered: usize,
    }
    impl TickPort for Crossing {
        fn observe(&mut self) -> Result<Vec<TickObservation>, Error> {
            Ok(vec![
                TickObservation {
                    identity_id: "member".into(),
                    mode: Mode::parse("20s").unwrap(),
                    flush_count: 10,
                    held_count: 10,
                    oldest_held_at_ms: Some(60_000),
                },
                TickObservation {
                    identity_id: "next".into(),
                    mode: Mode::parse("20s").unwrap(),
                    flush_count: 10,
                    held_count: 10,
                    oldest_held_at_ms: Some(60_000),
                },
            ])
        }
        fn deliver_if_current(&mut self, _: &str) -> Result<(), Error> {
            self.delivered += 1;
            self.time.set(120_100); // A call admitted before the boundary finishes after it.
            Ok(())
        }
    }
    let (mut clock, _) = fixture();
    clock.time.set(119_900);
    let mut port = Crossing {
        time: clock.time.clone(),
        delivered: 0,
    };
    run(&mut clock, &mut port).unwrap();
    assert_eq!(port.delivered, 1);
    assert_eq!(clock.time.get(), 120_100);

    let (mut clock, mut port) = fixture();
    clock.time.set(0); // Wall-clock rollback after the fixed budget was captured.
    port.held = false;
    run_until(&mut clock, &mut port, 120_000, 60_000).unwrap();
    assert_eq!(clock.elapsed, 60_000);
    assert_eq!(clock.time.get(), 60_000);
}
