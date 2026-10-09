use std::sync::{Arc, atomic::AtomicBool};

/// Shutdown signals that set one stop flag; unregistered on drop. A launcher also stops on
/// SIGHUP, so closing its terminal before the handoff cancels the startup it still owns.
pub struct Signals {
    ids: Vec<signal_hook::SigId>,
}
impl Signals {
    pub fn register(stop: &Arc<AtomicBool>, launcher: bool) -> std::io::Result<Self> {
        let mut signals = Self { ids: Vec::new() };
        for signal in [signal_hook::consts::SIGINT, signal_hook::consts::SIGTERM]
            .into_iter()
            .chain(launcher.then_some(signal_hook::consts::SIGHUP))
        {
            signals
                .ids
                .push(signal_hook::flag::register(signal, Arc::clone(stop))?);
        }
        Ok(signals)
    }
}
impl Drop for Signals {
    fn drop(&mut self) {
        for id in self.ids.drain(..) {
            signal_hook::low_level::unregister(id);
        }
    }
}
