//! Candidate-side exchange of one exact native proof; never owner consent.
use super::{
    Candidate, Context, invalid,
    port::*,
    proof,
    service::{check, gate, offer_io, provider},
};
use crate::{canonical, crypto, error::RemoteError, limits, objects::IoBudget};
use ed25519_dalek::{Signer, SigningKey};
use sha2::{Digest, Sha256};
use std::time::{Duration, Instant};

/// One in-memory publication attempt. A lost answer permits only observation
/// of its immutable original, never another publish or a different candidate.
pub struct SubmissionAttempt<B> {
    backend: B,
    publication: Publication,
    snapshot: [u8; 32],
    response_key: Option<ReceiptKey>,
    receipt: Option<Vec<u8>>,
    deadline: Instant,
    attempted: bool,
    acknowledged: bool,
}
impl<B: Backend> SubmissionAttempt<B> {
    #[cfg(test)]
    pub(super) fn retains_response_key(&self) -> bool {
        self.response_key.is_some()
    }
    pub fn new(
        backend: B,
        context: Context,
        candidate: Candidate,
        code: &[u8; 16],
        signing: &SigningKey,
        now_ms: u64,
    ) -> Result<Self, RemoteError> {
        let started = Instant::now();
        context.validate()?;
        if backend.binding() != context.binding
            || context.issued_at_ms.saturating_sub(now_ms)
                > crate::session::CLOCK_SKEW.as_millis() as u64
            || now_ms >= context.expires_at_ms
            || signing.verifying_key().to_bytes() != candidate.public_key
        {
            return Err(invalid());
        }
        let transcript = proof::enrollment(&context, &candidate)?;
        let mac = crypto::enrollment_mac(code, &transcript);
        let possession = canonical::possession(&transcript, &mac).map_err(|_| invalid())?;
        let submission = Submission {
            candidate,
            gate: gate(&context, code)?,
            mac,
            signature: signing.sign(&possession).to_bytes(),
        };
        let response_key =
            ReceiptKey(crypto::response_key(code, &transcript).map_err(|_| invalid())?);
        let deadline = started + Duration::from_millis(context.expires_at_ms - now_ms);
        Ok(Self {
            backend,
            publication: Publication {
                exchange: InvitationExchange {
                    context,
                    part: Part::Candidate,
                },
                payload: Payload::Candidate(submission),
            },
            snapshot: Sha256::digest(&possession).into(),
            response_key: Some(response_key),
            receipt: None,
            deadline,
            attempted: false,
            acknowledged: false,
        })
    }
    pub fn submit(&mut self, io: &IoBudget<'_>) -> Result<WriteOutcome, RemoteError> {
        let io = offer_io(io, self.deadline);
        if let Err(error) = check(&io) {
            self.response_key.take();
            return Err(error);
        }
        if self.acknowledged {
            return Ok(WriteOutcome::Acknowledged);
        }
        if !self.attempted {
            self.attempted = true;
            let result = self
                .backend
                .publish(&self.publication, &io)
                .map_err(provider)?;
            check(&io)?;
            self.acknowledged = result == WriteOutcome::Acknowledged;
        } else {
            let observed = self
                .backend
                .observe(&self.publication.exchange, &io)
                .map_err(provider)?;
            check(&io)?;
            if let Some(observed) = observed {
                let Payload::Candidate(candidate) = &self.publication.payload else {
                    return Err(invalid());
                };
                if observed.principal != candidate.candidate.principal
                    || observed.publication != self.publication
                {
                    return Err(invalid());
                }
                self.acknowledged = true;
            }
        }
        Ok(if self.acknowledged {
            WriteOutcome::Acknowledged
        } else {
            WriteOutcome::Unconfirmed
        })
    }
    /// Returns authenticated original bytes. The caller must decode the existing
    /// receipt schema before adopting an outbound connection; this creates none.
    pub fn observe_receipt(&mut self, io: &IoBudget<'_>) -> Result<Option<Vec<u8>>, RemoteError> {
        let result = (|| {
            let io = offer_io(io, self.deadline);
            check(&io)?;
            if let Some(receipt) = &self.receipt {
                return Ok(Some(receipt.clone()));
            }
            if !self.attempted {
                return Err(invalid());
            }
            let exchange = InvitationExchange {
                context: self.publication.exchange.context.clone(),
                part: Part::Receipt,
            };
            let Some(observed) = self.backend.observe(&exchange, &io).map_err(provider)? else {
                return Ok(None);
            };
            check(&io)?;
            if observed.principal != exchange.context.binding.owner_principal
                || observed.publication.exchange != exchange
            {
                return Err(invalid());
            }
            let Payload::Receipt {
                snapshot,
                bytes,
                proof,
            } = observed.publication.payload
            else {
                return Err(invalid());
            };
            if snapshot != self.snapshot
                || bytes.is_empty()
                || bytes.len() > limits::CLOUD_ENROLLMENT_BYTES
            {
                return Err(invalid());
            }
            let key = self.response_key.as_ref().ok_or_else(invalid)?;
            crypto::verify_server_proof(&key.0, &bytes, &proof).map_err(|_| invalid())?;
            Ok(Some(bytes))
        })();
        if let Ok(Some(receipt)) = &result {
            self.receipt = Some(receipt.clone());
            self.response_key.take();
        }
        if result.is_err() {
            self.response_key.take();
        }
        result
    }
}
