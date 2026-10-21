//! The inbound half of one active page: the cloud's authority is checked against the local
//! owner log, then each foreign stream's contiguous tail is admitted and applied by the serve's
//! own append path. A stream this machine holds nothing of starts from the cloud's checkpoints when
//! its log does not reach back to the beginning. A refusal of one stream never stops another, and
//! nothing here writes a position: the Store's stream position is the position.
use super::{
    PageRef,
    publisher::Fault,
    reader::{Boot, Checkpoints, Reader, Tail},
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

/// What a person reads when a writer's stream stops: the line after "stopped following one writer".
const CANNOT_OPEN: &str = "this machine cannot open this page's cloud copy yet. Keep editing it on the machine that created it.";
const MALFORMED: &str = "one of its cloud records is malformed.";
const NOT_VALID: &str = "its page original is not valid.";

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
    /// Install the checkpoint pair (content then own) of a stream this machine holds nothing of,
    /// with the update a fold binds, after the same admission as any original. An inbound that
    /// cannot install one refuses it.
    fn bootstrap(
        &mut self,
        _: &PageRef,
        _: &str,
        _anchor: Option<&[u8]>,
        _pair: &[Vec<u8>],
    ) -> Result<Applied, Reject> {
        Err(Reject::Refused)
    }
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
    /// The page's checkpoints, read at most once in a pass and only for a stream that needs to
    /// start from one.
    checkpoints: Option<Checkpoints>,
    /// Streams this machine holds nothing of whose checkpoints were looked for and are not all
    /// there yet, or not there at all. They are looked for again when the records change.
    unlisted: HashSet<(PageRef, String)>,
}
impl Puller {
    pub(crate) fn new(reader: Reader) -> Self {
        Self {
            reader,
            membership: false,
            chains: None,
            chains_stale: false,
            stopped: HashSet::new(),
            checkpoints: None,
            unlisted: HashSet::new(),
        }
    }
    /// The authority changed, or another page is watched: the next pass reads all of it again.
    pub(crate) fn refresh(&mut self) {
        self.membership = false;
        self.chains = None;
        self.chains_stale = false;
        self.unlisted.clear();
    }
    /// The log or the checkpoints changed: a writer may have joined, so the chains are read
    /// again, once [`CHAIN_RECHECK`] has passed since they were last read.
    pub(crate) fn records_changed(&mut self) {
        self.chains_stale = true;
        self.unlisted.clear();
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
        self.checkpoints = None;
        let (mut applied, mut more) = (0, false);
        for stream in streams {
            let key = (page.clone(), stream);
            if self.stopped.contains(&key) {
                continue;
            }
            let stream = key.1.as_str();
            let mut head = inbound.head(page, stream)?;
            let mut started = false;
            let originals = loop {
                let tail = self.reader.tail(page, stream, head)?;
                let empty = matches!(&tail, Tail::Originals(originals) if originals.is_empty());
                // A stream this machine holds nothing of starts from the cloud's checkpoints when
                // the provider no longer holds its prefix, or holds no log of it at all. One it
                // holds a prefix of cannot be joined to a pruned one, and stops as before.
                if head == 0 && !started && (empty || tail == Tail::CatchUpRequired) {
                    if self.unlisted.contains(&key) {
                        break None;
                    }
                    started = true;
                    match self.bootstrap(inbound, page, stream)? {
                        Started::Installed => {
                            head = inbound.head(page, stream)?;
                            continue;
                        }
                        Started::Waiting => {
                            self.unlisted.insert(key.clone());
                            break None;
                        }
                        Started::Unavailable if empty => {
                            self.unlisted.insert(key.clone());
                            break None;
                        }
                        Started::Unavailable => {
                            self.stopped.insert(key.clone());
                            report(&super::writer_line(&page.page, CANNOT_OPEN));
                            break None;
                        }
                        Started::Malformed => {
                            self.stopped.insert(key.clone());
                            report(&super::writer_line(&page.page, MALFORMED));
                            break None;
                        }
                        Started::Refused => {
                            self.stopped.insert(key.clone());
                            report(&super::writer_line(&page.page, NOT_VALID));
                            break None;
                        }
                    }
                }
                match tail {
                    Tail::CatchUpRequired => {
                        self.stopped.insert(key.clone());
                        report(&super::writer_line(&page.page, CANNOT_OPEN));
                        break None;
                    }
                    Tail::Malformed => {
                        self.stopped.insert(key.clone());
                        report(&super::writer_line(&page.page, MALFORMED));
                        break None;
                    }
                    Tail::Originals(originals) => break Some(originals),
                }
            };
            let Some(originals) = originals else {
                continue;
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
                        report(&super::writer_line(&page.page, NOT_VALID));
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

/// How a stream's start from the cloud's checkpoints ended.
enum Started {
    Installed,
    /// Part of its latest pair is still being published.
    Waiting,
    /// The cloud holds nothing it can start from.
    Unavailable,
    Malformed,
    /// The pair or its anchor was not admitted.
    Refused,
}
impl Puller {
    fn bootstrap(
        &mut self,
        inbound: &mut dyn Inbound,
        page: &PageRef,
        stream: &str,
    ) -> Result<Started, Fault> {
        if self.checkpoints.is_none() {
            self.checkpoints = Some(self.reader.checkpoints(page)?);
        }
        let checkpoints = self.checkpoints.as_ref().expect("read above");
        match self.reader.bootstrap(page, checkpoints, stream)? {
            Boot::Waiting => Ok(Started::Waiting),
            Boot::Unavailable => Ok(Started::Unavailable),
            Boot::Malformed => Ok(Started::Malformed),
            Boot::Pair { pair, anchor } => {
                match inbound.bootstrap(page, stream, anchor.as_deref(), &pair) {
                    Ok(_) => Ok(Started::Installed),
                    Err(Reject::Moved) => Err(Fault::Moved),
                    Err(Reject::Retry) => Err(Fault::Retry),
                    Err(Reject::Full) => Err(Fault::Stuck("This page has reached its size limit.")),
                    Err(Reject::Refused) => Ok(Started::Refused),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
