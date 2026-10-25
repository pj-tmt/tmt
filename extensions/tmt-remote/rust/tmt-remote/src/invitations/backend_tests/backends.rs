//! Two independent provider-port implementations; no scenario assertions.
use super::*;

pub(super) trait ScenarioBackend: Backend + Clone {
    fn peer(&self, principal: Vec<u8>) -> Self;
    fn lose_next_receipt(&self);
    fn writes(&self, part: Part) -> usize;
}

#[derive(Clone)]
pub(super) struct Mailbox {
    binding: BackendBinding,
    principal: Vec<u8>,
    rows: Arc<Mutex<BTreeMap<Key, Observation>>>,
    calls: Arc<Mutex<BTreeMap<u8, usize>>>,
    lose: Arc<Mutex<bool>>,
}
impl Mailbox {
    pub(super) fn new(binding: BackendBinding) -> Self {
        Self {
            principal: binding.owner_principal.clone(),
            binding,
            rows: Default::default(),
            calls: Default::default(),
            lose: Default::default(),
        }
    }
}
impl Backend for Mailbox {
    fn binding(&self) -> BackendBinding {
        self.binding.clone()
    }
    fn publish(
        &self,
        publication: &Publication,
        io: &IoBudget<'_>,
    ) -> Result<WriteOutcome, BackendFailure> {
        bounded(io)?;
        if publication.exchange.context.binding != self.binding {
            return Err(BackendFailure::Refused);
        }
        match &publication.payload {
            Payload::Candidate(submitted) if submitted.candidate.principal == self.principal => {
                let offer_key = (
                    publication.exchange.context.offer.clone(),
                    Part::Offer as u8,
                );
                let rows = self.rows.lock().unwrap();
                let Some(Observation {
                    publication:
                        Publication {
                            payload: Payload::Offer { gate_hash },
                            ..
                        },
                    ..
                }) = rows.get(&offer_key)
                else {
                    return Err(BackendFailure::Refused);
                };
                use sha2::Digest;
                if sha2::Sha256::digest(submitted.gate).as_slice() != gate_hash {
                    return Err(BackendFailure::Refused);
                }
            }
            Payload::Candidate(_) => return Err(BackendFailure::Refused),
            _ if self.principal != self.binding.owner_principal => {
                return Err(BackendFailure::Refused);
            }
            _ => {}
        }
        *self
            .calls
            .lock()
            .unwrap()
            .entry(publication.exchange.part as u8)
            .or_default() += 1;
        let mut rows = self.rows.lock().unwrap();
        let identity = key(&publication.exchange);
        if let Some(original) = rows.get(&identity) {
            if original.publication != *publication {
                return Err(BackendFailure::Refused);
            }
        } else {
            rows.insert(
                identity,
                Observation {
                    principal: self.principal.clone(),
                    publication: publication.clone(),
                },
            );
        }
        if publication.exchange.part == Part::Receipt
            && std::mem::take(&mut *self.lose.lock().unwrap())
        {
            Ok(WriteOutcome::Unconfirmed)
        } else {
            Ok(WriteOutcome::Acknowledged)
        }
    }
    fn observe(
        &self,
        exchange: &InvitationExchange,
        io: &IoBudget<'_>,
    ) -> Result<Option<Observation>, BackendFailure> {
        bounded(io)?;
        if exchange.context.binding != self.binding {
            return Err(BackendFailure::Refused);
        }
        Ok(self.rows.lock().unwrap().get(&key(exchange)).cloned())
    }
    fn withdraw(
        &self,
        context: &Context,
        io: &IoBudget<'_>,
    ) -> Result<WriteOutcome, BackendFailure> {
        bounded(io)?;
        if self.principal != self.binding.owner_principal {
            return Err(BackendFailure::Refused);
        }
        self.rows
            .lock()
            .unwrap()
            .remove(&(context.offer.clone(), Part::Offer as u8));
        Ok(WriteOutcome::Acknowledged)
    }
}
impl ScenarioBackend for Mailbox {
    fn peer(&self, principal: Vec<u8>) -> Self {
        Self {
            principal,
            ..self.clone()
        }
    }
    fn lose_next_receipt(&self) {
        *self.lose.lock().unwrap() = true;
    }
    fn writes(&self, part: Part) -> usize {
        self.calls
            .lock()
            .unwrap()
            .get(&(part as u8))
            .copied()
            .unwrap_or(0)
    }
}

enum Event {
    Published(Box<Observation>),
    Withdrawn(String),
}
#[derive(Clone)]
pub(super) struct AppendLog {
    binding: BackendBinding,
    principal: Vec<u8>,
    events: Arc<Mutex<Vec<Event>>>,
    receipt_reads_hidden: Arc<Mutex<usize>>,
    receipt_writes: Arc<Mutex<usize>>,
    lose: Arc<Mutex<bool>>,
}
impl AppendLog {
    pub(super) fn new(binding: BackendBinding) -> Self {
        Self {
            principal: binding.owner_principal.clone(),
            binding,
            events: Default::default(),
            receipt_reads_hidden: Default::default(),
            receipt_writes: Default::default(),
            lose: Default::default(),
        }
    }
}
impl Backend for AppendLog {
    fn binding(&self) -> BackendBinding {
        self.binding.clone()
    }
    fn publish(
        &self,
        publication: &Publication,
        io: &IoBudget<'_>,
    ) -> Result<WriteOutcome, BackendFailure> {
        bounded(io)?;
        if publication.exchange.context.binding != self.binding {
            return Err(BackendFailure::Refused);
        }
        let mut events = self.events.lock().unwrap();
        if let Payload::Candidate(submitted) = &publication.payload {
            if submitted.candidate.principal != self.principal {
                return Err(BackendFailure::Refused);
            }
            let offered = events.iter().rev().find_map(|event| match event {
                Event::Published(row)
                    if row.publication.exchange.context == publication.exchange.context
                        && row.publication.exchange.part == Part::Offer =>
                {
                    Some(&row.publication.payload)
                }
                _ => None,
            });
            use sha2::Digest;
            if !matches!(offered, Some(Payload::Offer {gate_hash}) if sha2::Sha256::digest(submitted.gate).as_slice() == gate_hash)
            {
                return Err(BackendFailure::Refused);
            }
        } else if self.principal != self.binding.owner_principal {
            return Err(BackendFailure::Refused);
        }
        let original = events.iter().find_map(|event| match event {
            Event::Published(row)
                if row.publication.exchange == publication.exchange
                    && match (&row.publication.payload, &publication.payload) {
                        (Payload::Candidate(a), Payload::Candidate(b)) => {
                            a.candidate.machine == b.candidate.machine
                        }
                        _ => true,
                    } =>
            {
                Some(&row.publication)
            }
            _ => None,
        });
        if original.is_some_and(|original| *original != *publication) {
            return Err(BackendFailure::Refused);
        }
        if publication.exchange.part == Part::Receipt {
            *self.receipt_writes.lock().unwrap() += 1;
        }
        if original.is_none() {
            events.push(Event::Published(Box::new(Observation {
                principal: self.principal.clone(),
                publication: publication.clone(),
            })));
        }
        if publication.exchange.part == Part::Receipt
            && std::mem::take(&mut *self.lose.lock().unwrap())
        {
            *self.receipt_reads_hidden.lock().unwrap() = 1;
            Ok(WriteOutcome::Unconfirmed)
        } else {
            Ok(WriteOutcome::Acknowledged)
        }
    }
    fn observe(
        &self,
        exchange: &InvitationExchange,
        io: &IoBudget<'_>,
    ) -> Result<Option<Observation>, BackendFailure> {
        bounded(io)?;
        if exchange.context.binding != self.binding {
            return Err(BackendFailure::Refused);
        }
        if exchange.part == Part::Receipt {
            let mut hidden = self.receipt_reads_hidden.lock().unwrap();
            if *hidden > 0 {
                *hidden -= 1;
                return Ok(None);
            }
        }
        let events = self.events.lock().unwrap();
        if events.iter().any(
            |event| matches!(event, Event::Withdrawn(offer) if *offer == exchange.context.offer),
        ) {
            return Ok(None);
        }
        Ok(events.iter().find_map(|event| match event {
            Event::Published(row) if row.publication.exchange == *exchange => {
                Some(row.as_ref().clone())
            }
            _ => None,
        }))
    }
    fn withdraw(
        &self,
        context: &Context,
        io: &IoBudget<'_>,
    ) -> Result<WriteOutcome, BackendFailure> {
        bounded(io)?;
        if self.principal != self.binding.owner_principal {
            return Err(BackendFailure::Refused);
        }
        self.events
            .lock()
            .unwrap()
            .push(Event::Withdrawn(context.offer.clone()));
        Ok(WriteOutcome::Acknowledged)
    }
}
impl ScenarioBackend for AppendLog {
    fn peer(&self, principal: Vec<u8>) -> Self {
        Self {
            principal,
            ..self.clone()
        }
    }
    fn lose_next_receipt(&self) {
        *self.lose.lock().unwrap() = true;
    }
    fn writes(&self, part: Part) -> usize {
        if part == Part::Receipt {
            *self.receipt_writes.lock().unwrap()
        } else {
            0
        }
    }
}
