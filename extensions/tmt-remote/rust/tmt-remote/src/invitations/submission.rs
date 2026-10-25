//! Candidate-side exchange of one exact native proof; never owner consent.
use super::{
    Candidate, Context, invalid,
    port::*,
    proof,
    service::{check, gate, offer_io, provider},
};
use crate::{canonical, crypto, error::RemoteError, limits, objects::IoBudget};
use ed25519_dalek::{Signer, SigningKey};
use std::time::{Duration, Instant};

impl NativeSigner for SigningKey {
    fn public_key(&self) -> [u8; 32] {
        self.verifying_key().to_bytes()
    }
    fn sign_possession(&self, message: &[u8]) -> [u8; 64] {
        self.sign(message).to_bytes()
    }
}

/// One in-memory publication attempt. A lost answer permits only observation
/// of its immutable original, never another publish or a different candidate.
pub struct SubmissionAttempt<B> {
    backend: B,
    context: Context,
    submission: Submission,
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
        signing: &impl NativeSigner,
        now_ms: u64,
    ) -> Result<Self, RemoteError> {
        if signing.public_key() != candidate.public_key {
            return Err(invalid());
        }
        let transcript = proof::enrollment(&context, &candidate)?;
        let mac = crypto::enrollment_mac(code, &transcript);
        let possession = canonical::possession(&transcript, &mac).map_err(|_| invalid())?;
        let signature = signing.sign_possession(&possession);
        let submission = Submission {
            candidate,
            gate: gate(&context, code)?,
            mac,
            signature,
        };
        Self::from_signed(backend, context, submission, code, now_ms, false)
    }

    /// Restore a durably claimed original. Even if the provider has no visible
    /// row, this instance can only observe; it never publishes another candidate.
    pub fn resume_original(
        backend: B,
        context: Context,
        submission: Submission,
        code: &[u8; 16],
        now_ms: u64,
    ) -> Result<Self, RemoteError> {
        Self::from_signed(backend, context, submission, code, now_ms, true)
    }

    /// Public proof metadata for the existing private pending-connection record.
    /// A caller's codec must omit the gate capability, code and response key.
    pub fn signed_submission(&self) -> &Submission {
        &self.submission
    }

    fn publication(&self) -> Publication {
        Publication {
            exchange: InvitationExchange {
                context: self.context.clone(),
                part: Part::Candidate,
            },
            payload: Payload::Candidate(self.submission.clone()),
        }
    }

    fn from_signed(
        backend: B,
        context: Context,
        submission: Submission,
        code: &[u8; 16],
        now_ms: u64,
        attempted: bool,
    ) -> Result<Self, RemoteError> {
        let started = Instant::now();
        context.validate()?;
        if backend.binding() != context.binding || submission.gate != gate(&context, code)? {
            return Err(invalid());
        }
        let verified = super::verify_enrollment(
            &context,
            &submission.candidate,
            &submission.candidate.principal,
            code,
            &submission.mac,
            &submission.signature,
            now_ms,
        )?;
        let transcript = proof::enrollment(&context, &submission.candidate)?;
        let response_key =
            ReceiptKey(crypto::response_key(code, &transcript).map_err(|_| invalid())?);
        let deadline = started + Duration::from_millis(context.expires_at_ms - now_ms);
        Ok(Self {
            backend,
            context,
            submission,
            snapshot: *verified.digest(),
            response_key: Some(response_key),
            receipt: None,
            deadline,
            attempted,
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
        let publication = self.publication();
        if !self.attempted {
            self.attempted = true;
            let result = self.backend.publish(&publication, &io).map_err(provider)?;
            check(&io)?;
            self.acknowledged = result == WriteOutcome::Acknowledged;
        } else {
            let observed = self
                .backend
                .observe(&publication.exchange, &io)
                .map_err(provider)?;
            check(&io)?;
            if let Some(observed) = observed {
                if observed.principal != self.submission.candidate.principal
                    || observed.publication != publication
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
    /// Returns authenticated original bytes without adopting local authority.
    pub fn observe_receipt(&mut self, io: &IoBudget<'_>) -> Result<Option<Vec<u8>>, RemoteError> {
        let io = offer_io(io, self.deadline);
        if let Err(error) = check(&io) {
            self.response_key.take();
            return Err(error);
        }
        if let Some(receipt) = &self.receipt {
            return Ok(Some(receipt.clone()));
        }
        self.observe_receipt_with(&io, |bytes, _, _| Ok(bytes.to_vec()))
    }

    /// One consuming verified-receipt handoff. The adapter can derive its scoped
    /// capabilities and commit its local receipt adoption before the response
    /// key is erased; it never receives or persists the raw key. An error also
    /// erases it, so another attempt requires the caller's original code again.
    pub fn observe_receipt_with<T>(
        &mut self,
        io: &IoBudget<'_>,
        adopt: impl FnOnce(&[u8], &ReceiptKey, &IoBudget<'_>) -> Result<T, RemoteError>,
    ) -> Result<Option<T>, RemoteError> {
        let result = (|| {
            let io = offer_io(io, self.deadline);
            check(&io)?;
            if self.receipt.is_some() || self.response_key.is_none() {
                return Err(invalid());
            }
            let Some(bytes) = self.verified_receipt(&io)? else {
                return Ok(None);
            };
            let key = self.response_key.as_ref().ok_or_else(invalid)?;
            let adopted = adopt(&bytes, key, &io)?;
            Ok(Some((bytes, adopted)))
        })();
        match result {
            Ok(Some((bytes, adopted))) => {
                self.receipt = Some(bytes);
                self.response_key.take();
                Ok(Some(adopted))
            }
            Ok(None) => Ok(None),
            Err(error) => {
                self.response_key.take();
                Err(error)
            }
        }
    }

    fn verified_receipt(&self, io: &IoBudget<'_>) -> Result<Option<Vec<u8>>, RemoteError> {
        if !self.attempted {
            return Err(invalid());
        }
        let exchange = InvitationExchange {
            context: self.context.clone(),
            part: Part::Receipt,
        };
        let Some(observed) = self.backend.observe(&exchange, io).map_err(provider)? else {
            return Ok(None);
        };
        check(io)?;
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
    }
}
