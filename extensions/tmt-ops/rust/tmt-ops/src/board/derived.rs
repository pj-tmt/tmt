//! Disposable surface derivations retained by one immutable view. New snapshots
//! own a new cache; workspace rows have no legacy grid or boxed-list cache.

#[derive(Default)]
pub(super) struct Derived {
    /// HOME's painted sections, each held with the inputs it was painted for.
    pub home: super::home::Scenes,
}
