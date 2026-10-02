//! Opt-in local batch benchmark. Owns its children; reads no TMT state.
use std::{
    process::{Child, Command},
    time::{Duration, Instant},
};
use tmt_adapters::process::{
    CommandError, CommandOutput, CommandRequest, CommandRunner, UnixCommandRunner, ps::query_ps,
    runtime::observe_starts,
};

struct Children(Vec<Child>);
impl Drop for Children {
    fn drop(&mut self) {
        for child in &mut self.0 {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
struct Ps;
impl CommandRunner for Ps {
    fn execute(&self, request: CommandRequest<'_>) -> Result<CommandOutput, CommandError> {
        UnixCommandRunner.execute(request)
    }
}
fn measure(mut call: impl FnMut()) -> f64 {
    let mut samples = Vec::new();
    for _ in 0..20 {
        let start = Instant::now();
        call();
        samples.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    samples.sort_by(f64::total_cmp);
    (samples[9] + samples[10]) / 2.0
}
fn main() {
    let mut children = Children(Vec::new());
    for _ in 0..10 {
        children
            .0
            .push(Command::new("/bin/sleep").arg("60").spawn().unwrap());
    }
    let pids: Vec<u64> = children
        .0
        .iter()
        .map(|child| u64::from(child.id()))
        .collect();
    for n in [1, 2, 10] {
        let pids = &pids[..n];
        let deadline = || Instant::now() + Duration::from_secs(5);
        assert_eq!(
            observe_starts(&Ps, pids, deadline()).unwrap(),
            observe_starts(&UnixCommandRunner, pids, deadline()).unwrap()
        );
        let before = measure(|| {
            assert_eq!(observe_starts(&Ps, pids, deadline()).unwrap().len(), n);
        });
        let after = measure(|| {
            assert_eq!(
                observe_starts(&UnixCommandRunner, pids, deadline())
                    .unwrap()
                    .len(),
                n
            );
        });
        let list = pids
            .iter()
            .map(u64::to_string)
            .collect::<Vec<_>>()
            .join(",");
        let args = ["-o", "pid=,ppid=", "-p", &list].map(Into::into);
        let parents_before = measure(|| {
            query_ps(&Ps, &args, deadline(), 64 * 1024).unwrap();
        });
        let parents_after = measure(|| {
            for pid in pids {
                assert_eq!(
                    UnixCommandRunner.process_parent(*pid, deadline()),
                    Some(u64::from(std::process::id()))
                );
            }
        });
        println!(
            "{n} PIDs: starts {before:.6} -> {after:.6} ms; parents {parents_before:.6} -> {parents_after:.6} ms"
        );
    }
}
