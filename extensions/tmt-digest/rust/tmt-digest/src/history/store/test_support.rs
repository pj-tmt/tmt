//! Private store fixtures shared by settlement and valuation scenarios.
use super::Store;
use crate::{
    core::Error,
    history::{Window, model},
};
use serde_json::{Value, json};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

pub(super) const ID: &str = "10000000-0000-4000-8000-000000000001";
static NEXT: AtomicU64 = AtomicU64::new(0);
pub(super) struct Fixture(pub(super) PathBuf);
impl Fixture {
    pub(super) fn new() -> Self {
        // The store refuses a symlinked path, and macOS's temp dir is under the `/var` symlink.
        let path = fs::canonicalize(std::env::temp_dir())
            .unwrap()
            .join(format!(
                "digest-history-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    pub(super) fn store(&self) -> Store {
        Store::open(&self.0, true).unwrap().unwrap()
    }
}
impl Store {
    /// `status` at the built-in prices, for scenarios that are not about prices.
    pub(crate) fn status_built_in(
        &mut self,
        id: &str,
        window: Window,
        now: u64,
        observed_counter: Option<u64>,
    ) -> Result<Value, Error> {
        self.status(
            id,
            window,
            now,
            observed_counter,
            crate::pricing::built_in(),
        )
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
pub(super) fn event(seq: u64, time: u64, contexts: &[Option<u64>]) -> model::ChecklistSettlement {
    serde_json::from_value(json!({"checklistId":format!("20000000-0000-4000-8000-{seq:012}"),"settlementSequence":seq,"claimedAtMs":time-1,"settledAtMs":time,"outcome":"delivered","opportunity":"idle","sealedCount":contexts.len(),"retainedCount":contexts.len(),"membershipComplete":true,"nextItemAfter":null,"items":contexts.iter().enumerate().map(|(index,context)|json!({"sequence":index+1,"createdAtMs":time-2,"contextTokensAtArrival":context,"contextObservedAtMs":context.map(|_|time-3)})).collect::<Vec<_>>()})).unwrap()
}
