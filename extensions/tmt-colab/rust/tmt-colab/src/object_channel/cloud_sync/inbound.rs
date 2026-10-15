//! The inbound half of one active page: the cloud's authority is checked against the local
//! owner log, then each foreign stream's contiguous tail is admitted and applied by the serve's
//! own append path. A refusal of one stream never stops another, and nothing here writes a
//! position: the Store's receipts are the position.
use super::{
    PageRef,
    publisher::Fault,
    reader::{Reader, Tail},
};
use std::{
    collections::HashSet,
    time::{Duration, Instant},
};

mod store;
pub(crate) use store::StoreInbound;

/// The documents one pass applies before it yields; the next wake continues.
const PASS_ORIGINALS: usize = 64;
/// How long a writer that joined without a membership change can wait to be found. The
/// authority interest watches membership records only, so a new device's chain is found by
/// reading the chains again after the log or the checkpoints change; and this machine's own
/// publications change them too, so that read is spaced instead of following every notice.
pub(crate) const CHAIN_RECHECK: Duration = if cfg!(test) {
    Duration::from_millis(300)
} else {
    Duration::from_secs(60)
};

/// Why one foreign original was not applied.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Reject {
    /// The page is gone or at another epoch.
    Moved,
    /// Not admitted: the author, role, epoch, namespace, signature or ciphertext is wrong, or
    /// the original contradicts what the stream already holds.
    Refused,
    /// The Store could not take it for now.
    Retry,
    /// The page cannot hold more.
    Full,
}
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Applied {
    New,
    Replay,
}
/// The Store side of an inbound pass.
pub(crate) trait Inbound {
    /// Compare the cloud's membership originals with this machine's owner log. A cloud log
    /// that is ahead of, forks from, or is not a chain from the owner stops the page with a
    /// line a person can read; it is never merged or replaced.
    fn membership(&mut self, page: &PageRef, cloud: &[Vec<u8>]) -> Result<(), Fault>;
    /// Prepare to admit against the owner log plus the cloud's chains, and name the streams of
    /// the chains this machine did not write.
    fn begin(&mut self, page: &PageRef, chains: &[Vec<u8>]) -> Result<Vec<String>, Fault>;
    /// The sequence of the stream's latest stored receipt; zero when it has none.
    fn head(&mut self, page: &PageRef, stream: &str) -> Result<u64, Fault>;
    fn admit(&mut self, page: &PageRef, original: &[u8]) -> Result<Applied, Reject>;
    /// The pass has read all it could: act on what it stored, such as the edit requests a link
    /// device left. Nothing to do for an inbound with no such follow-up.
    fn settle(&mut self, _: &PageRef) {}
}

pub(crate) struct Puller {
    reader: Reader,
    /// Whether the cloud's membership records were compared since the authority last changed.
    membership: bool,
    /// The chains of the last read, which name the streams to follow, and when they were read.
    chains: Option<(Vec<Vec<u8>>, Instant)>,
    /// The log or the checkpoints changed since the chains were read.
    chains_stale: bool,
    /// Streams stopped for this process on one page at one epoch: one original was refused or
    /// the provider has dropped the sequence this machine needs. Other streams, and the same
    /// writer on another page, continue.
    stopped: HashSet<(PageRef, String)>,
}
impl Puller {
    pub(crate) fn new(reader: Reader) -> Self {
        Self {
            reader,
            membership: false,
            chains: None,
            chains_stale: false,
            stopped: HashSet::new(),
        }
    }
    /// The authority changed, or another page is watched: the next pass reads all of it again.
    pub(crate) fn refresh(&mut self) {
        self.membership = false;
        self.chains = None;
        self.chains_stale = false;
    }
    /// The log or the checkpoints changed: a writer may have joined, so the chains are read
    /// again, once [`CHAIN_RECHECK`] has passed since they were last read.
    pub(crate) fn records_changed(&mut self) {
        self.chains_stale = true;
    }
    /// When the chains are next read, if they are waiting for their spacing to end.
    pub(crate) fn recheck_at(&self) -> Option<Instant> {
        match (&self.chains, self.chains_stale) {
            (Some((_, read)), true) => Some(*read + CHAIN_RECHECK),
            _ => None,
        }
    }
    /// One bounded pass. `Ok(true)` means more may remain and the next wake continues.
    pub(crate) fn pass(
        &mut self,
        inbound: &mut dyn Inbound,
        page: &PageRef,
        report: &dyn Fn(&str),
    ) -> Result<bool, Fault> {
        if !self.membership {
            let membership = self.reader.authority(page, "membership")?;
            inbound.membership(page, &membership)?;
            self.membership = true;
        }
        if self.chains.is_none() || self.recheck_at().is_some_and(|at| Instant::now() >= at) {
            self.chains = Some((self.reader.authority(page, "chain")?, Instant::now()));
            self.chains_stale = false;
        }
        let chains = self.chains.as_ref().map_or(&[][..], |(chains, _)| chains);
        let streams = inbound.begin(page, chains)?;
        let (mut applied, mut more) = (0, false);
        for stream in streams {
            let key = (page.clone(), stream);
            if self.stopped.contains(&key) {
                continue;
            }
            let stream = key.1.as_str();
            let head = inbound.head(page, stream)?;
            let originals = match self.reader.tail(page, stream, head)? {
                Tail::CatchUpRequired => {
                    self.stopped.insert(key);
                    report(&super::writer_line(
                        &page.page,
                        "it is too far behind; open a fresh copy of the page to catch up.",
                    ));
                    continue;
                }
                Tail::Malformed => {
                    self.stopped.insert(key);
                    report(&super::writer_line(
                        &page.page,
                        "one of its cloud records is malformed.",
                    ));
                    continue;
                }
                Tail::Originals(originals) => originals,
            };
            more |= !originals.is_empty();
            for (_, bytes) in originals {
                match inbound.admit(page, &bytes) {
                    Ok(_) => applied += 1,
                    Err(Reject::Moved) => return Err(Fault::Moved),
                    Err(Reject::Retry) => return Err(Fault::Retry),
                    Err(Reject::Full) => {
                        return Err(Fault::Stuck("This page has reached its size limit."));
                    }
                    Err(Reject::Refused) => {
                        self.stopped.insert(key);
                        report(&super::writer_line(
                            &page.page,
                            "its page original is not valid.",
                        ));
                        break;
                    }
                }
                if applied >= PASS_ORIGINALS {
                    return Ok(true);
                }
            }
        }
        inbound.settle(page);
        Ok(more)
    }
}

#[cfg(test)]
mod tests;
