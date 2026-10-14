//! Sealed-document correlations on the existing object bus. Notices coalesce;
//! they wake a consumer but never advance its verified stream position.
use super::{Client, Error, Shared, locked};
use std::{
    collections::BTreeMap,
    sync::{Arc, mpsc},
    time::Instant,
};
use tmt_extension_objects::{Caps, ChannelFrame, Counter, sync};

type Reply = Result<sync::Outcome, Error>;
struct Pending {
    deadline: Instant,
    call: sync::Call,
    reply: mpsc::SyncSender<Reply>,
}
#[derive(Default)]
pub(super) struct State {
    pending: BTreeMap<Counter, Pending>,
    interests: BTreeMap<Counter, Option<sync::InterestNotice>>,
}
impl State {
    pub(super) fn stop(&mut self) {
        for (_, pending) in std::mem::take(&mut self.pending) {
            let _ = pending.reply.try_send(Err(Error::Unavailable));
        }
        self.interests.clear();
    }
    pub(super) fn pending_count(&self) -> usize {
        self.pending.len()
    }
}
impl Client {
    pub fn supports_sync(&self) -> Result<bool, Error> {
        Ok(self.0.bus()?.supports_sync())
    }
    /// One shared counter/send order across objects and documents. An uncertain
    /// timeout ends the generation; a later reply cannot satisfy a successor.
    pub fn sealed_request(
        &self,
        call: sync::Call,
        deadline: Instant,
    ) -> Result<(Counter, sync::Outcome), Error> {
        if Instant::now() >= deadline {
            return Err(Error::Unavailable);
        }
        let (sender, receiver) = mpsc::sync_channel(1);
        let request_id;
        {
            let mut next = locked(&self.0.next);
            let bus = self.0.bus()?;
            if !bus.supports_sync() {
                return Err(Error::Wire(tmt_extension_objects::Fault::UnsupportedSync));
            }
            request_id = Counter::new(*next).map_err(|_| Error::Unavailable)?;
            *next = next.checked_add(1).ok_or(Error::Unavailable)?;
            {
                let mut state = locked(&self.0.state);
                if state.pending.len() + state.sealed.pending_count() >= Caps::contract().requests {
                    return Err(Error::Unavailable);
                }
                if matches!(call, sync::Call::Subscribe { .. }) {
                    // Register before the send: a notice may immediately follow
                    // its result before the waiting caller is scheduled.
                    if state.sealed.interests.len() >= tmt_extension_objects::limits::SYNC_INTERESTS
                    {
                        return Err(Error::Unavailable);
                    }
                    state.sealed.interests.insert(request_id, None);
                }
                state.sealed.pending.insert(
                    request_id,
                    Pending {
                        deadline,
                        call: call.clone(),
                        reply: sender,
                    },
                );
            }
            let frame = ChannelFrame::Sync(sync::Frame::Request {
                version: 1,
                generation: self.generation().to_string(),
                request_id: request_id.to_string(),
                call,
            });
            if let Err(fault) = bus.send_channel_until(&frame, deadline) {
                let mut state = locked(&self.0.state);
                state.sealed.pending.remove(&request_id);
                state.sealed.interests.remove(&request_id);
                drop(state);
                if bus.fault().is_some() {
                    self.0.stop();
                }
                return Err(Error::Wire(fault));
            }
        }
        match receiver.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(reply) => reply.map(|outcome| (request_id, outcome)),
            Err(_) => {
                self.0.stop();
                Err(Error::Unavailable)
            }
        }
    }
    /// Wait without polling for one retained interest. Timeout carries no
    /// completion claim and does not cancel the subscription or its ledger slot.
    pub fn sealed_notice_until(
        &self,
        id: Counter,
        deadline: Instant,
    ) -> Result<Option<sync::InterestNotice>, Error> {
        let mut state = locked(&self.0.state);
        loop {
            if self.0.stopped.load(std::sync::atomic::Ordering::Acquire) {
                return Err(Error::Unavailable);
            }
            let notice = state
                .sealed
                .interests
                .get_mut(&id)
                .ok_or(Error::Unavailable)?;
            if notice.is_some() {
                return match notice {
                    Some(sync::InterestNotice::Terminal { .. }) => Ok(notice.clone()),
                    _ => Ok(notice.take()),
                };
            }
            let Some(left) = deadline.checked_duration_since(Instant::now()) else {
                return Ok(None);
            };
            let (next, _) = self
                .0
                .changed
                .wait_timeout(state, left)
                .unwrap_or_else(|p| p.into_inner());
            state = next;
        }
    }
    /// Taking a terminal notice does not release its retained interest. Only
    /// confirmed cancellation or generation close allows replacement.
    pub fn sealed_notice(&self, id: Counter) -> Result<Option<sync::InterestNotice>, Error> {
        if self.0.stopped.load(std::sync::atomic::Ordering::Acquire) {
            return Err(Error::Unavailable);
        }
        let mut state = locked(&self.0.state);
        let notice = state
            .sealed
            .interests
            .get_mut(&id)
            .ok_or(Error::Unavailable)?;
        match notice {
            Some(sync::InterestNotice::Terminal { .. }) => Ok(notice.clone()),
            _ => Ok(notice.take()),
        }
    }
}
pub(super) fn dispatch(shared: &Arc<Shared>, frame: sync::Frame) -> bool {
    let mut state = locked(&shared.state);
    match frame {
        sync::Frame::Result {
            request_id,
            outcome,
            ..
        } => {
            let Ok(id) = Counter::parse(&request_id) else {
                return false;
            };
            let Some(pending) = state.sealed.pending.remove(&id) else {
                return false;
            };
            match (&pending.call, &outcome) {
                (sync::Call::Subscribe { .. }, sync::Outcome::Error { .. }) => {
                    state.sealed.interests.remove(&id);
                }
                (
                    sync::Call::Cancel {
                        subscription_id, ..
                    },
                    sync::Outcome::Cancelled { id },
                ) if id == subscription_id => {
                    let Ok(subscription) = Counter::parse(id) else {
                        return false;
                    };
                    state.sealed.interests.remove(&subscription);
                }
                _ => {}
            }
            let answer = if Instant::now() < pending.deadline {
                Ok(outcome)
            } else {
                Err(Error::Unavailable)
            };
            let _ = pending.reply.try_send(answer);
        }
        sync::Frame::Notice {
            subscription_id,
            notice,
            ..
        } => {
            let Ok(id) = Counter::parse(&subscription_id) else {
                return false;
            };
            let Some(current) = state.sealed.interests.get_mut(&id) else {
                return false;
            };
            if !matches!(current, Some(sync::InterestNotice::Terminal { .. })) {
                *current = Some(notice);
            }
            shared.changed.notify_all();
        }
        _ => return false,
    }
    true
}
