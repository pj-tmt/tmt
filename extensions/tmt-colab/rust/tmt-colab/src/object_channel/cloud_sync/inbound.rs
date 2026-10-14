//! The inbound half of one active page: the cloud's authority is checked against the local
//! owner log, then each foreign stream's contiguous tail is admitted and applied by the serve's
//! own append path. A refusal of one stream never stops another, and nothing here writes a
//! position: the Store's receipts are the position.
use super::{
    PageRef,
    publisher::Fault,
    reader::{Reader, Tail},
};
use std::collections::HashSet;

mod store;
pub(crate) use store::StoreInbound;

/// The documents one pass applies before it yields; the next wake continues.
const PASS_ORIGINALS: usize = 64;

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
}

pub(crate) struct Puller {
    reader: Reader,
    /// The chains of the last authority read, which name the streams to follow.
    chains: Option<Vec<Vec<u8>>>,
    /// Streams stopped for this process: one original was refused or the provider has dropped
    /// the sequence this machine needs. Other streams continue.
    stopped: HashSet<String>,
}
impl Puller {
    pub(crate) fn new(reader: Reader) -> Self {
        Self {
            reader,
            chains: None,
            stopped: HashSet::new(),
        }
    }
    /// Forget the authority, so the next pass reads it again.
    pub(crate) fn refresh(&mut self) {
        self.chains = None;
    }
    /// One bounded pass. `Ok(true)` means more may remain and the next wake continues.
    pub(crate) fn pass(
        &mut self,
        inbound: &mut dyn Inbound,
        page: &PageRef,
        report: &dyn Fn(&str),
    ) -> Result<bool, Fault> {
        if self.chains.is_none() {
            let membership = self.reader.authority(page, "membership")?;
            inbound.membership(page, &membership)?;
            self.chains = Some(self.reader.authority(page, "chain")?);
        }
        let chains = self.chains.as_deref().unwrap_or_default();
        let streams = inbound.begin(page, chains)?;
        let (mut applied, mut more) = (0, false);
        for stream in streams {
            if self.stopped.contains(&stream) {
                continue;
            }
            let head = inbound.head(page, &stream)?;
            let originals = match self.reader.tail(page, &stream, head)? {
                Tail::CatchUpRequired => {
                    self.stopped.insert(stream);
                    report("Cloud sync stopped for one writer: only a fresh copy can catch up.");
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
                        return Err(Fault::Stuck("Cloud sync stopped: this page is full."));
                    }
                    Err(Reject::Refused) => {
                        self.stopped.insert(stream.clone());
                        report(
                            "Cloud sync refused an original from one writer and stopped following it.",
                        );
                        break;
                    }
                }
                if applied >= PASS_ORIGINALS {
                    return Ok(true);
                }
            }
        }
        Ok(more)
    }
}

#[cfg(test)]
mod tests;
