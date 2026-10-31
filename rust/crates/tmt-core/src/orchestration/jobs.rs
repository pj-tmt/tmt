//! Agent-owned routine work. Calendar, persistence and dispatch remain ports.
use super::OrchRoutine;
use crate::{dispatch::canonical_id, exact_text::validate_exact_text, limits::MAX_JS_SAFE_INTEGER};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobError {
    Invalid,
    Conflict,
    NotFound,
    NoOwner,
    NotAllowed,
    Limit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobSchedule {
    Every {
        zone: String,
        seconds: i64,
        anchor_ms: i64,
        from: Option<String>,
    },
    At {
        zone: String,
        at: String,
        on: Option<String>,
    },
    Cron {
        zone: String,
        expression: String,
    },
}
impl JobSchedule {
    pub fn zone(&self) -> &str {
        match self {
            Self::Every { zone, .. } | Self::At { zone, .. } | Self::Cron { zone, .. } => zone,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyJob {
    pub room_id: String,
    pub cron_id: u64,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobPause {
    pub by: String,
    pub at_ms: i64,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeldJobSlot {
    pub revision: u64,
    pub at_ms: i64,
}
/// Mutable definition before the storage owner allocates its permanent IDs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobDefinition {
    pub owner_id: Option<String>,
    pub room_id: Option<String>,
    pub title: Option<String>,
    pub message: String,
    pub schedule: JobSchedule,
    pub pause: Option<JobPause>,
}
impl JobDefinition {
    pub fn with_identity(self, id: String, number: u64) -> OrchJob {
        OrchJob {
            id,
            number,
            revision: 1,
            owner_id: self.owner_id,
            room_id: self.room_id,
            title: self.title,
            message: self.message,
            schedule: self.schedule,
            pause: self.pause,
            pending: None,
            legacy: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrchJob {
    pub id: String,
    pub number: u64,
    pub revision: u64,
    pub owner_id: Option<String>,
    pub room_id: Option<String>,
    pub title: Option<String>,
    pub message: String,
    pub schedule: JobSchedule,
    pub pause: Option<JobPause>,
    pub pending: Option<HeldJobSlot>,
    pub legacy: Option<LegacyJob>,
}
impl OrchJob {
    /// Calendar codecs validate local times and cron fields before publication.
    pub fn validate(&self) -> Result<(), JobError> {
        self.validate_metadata()?;
        if self.message.trim().is_empty() || validate_exact_text(self.message.as_bytes()).is_err() {
            return Err(JobError::Invalid);
        }
        Ok(())
    }
    /// Bounded list readers validate metadata without loading the full body.
    pub fn validate_metadata(&self) -> Result<(), JobError> {
        let safe = |n: u64| n > 0 && n <= MAX_JS_SAFE_INTEGER;
        if !canonical_id(&self.id)
            || !safe(self.number)
            || !safe(self.revision)
            || self.owner_id.as_ref().is_some_and(|id| !canonical_id(id))
            || self.room_id.as_ref().is_some_and(|id| !canonical_id(id))
            || self.owner_id.is_none() && self.pause.is_none()
            || self.title.as_ref().is_some_and(|title| {
                title.trim().is_empty() || title.len() > 200 || title.chars().any(char::is_control)
            })
            || self.schedule.zone().is_empty()
            || self.schedule.zone().len() > 128
            || self.pause.as_ref().is_some_and(|pause| {
                pause.by.is_empty() || pause.at_ms.unsigned_abs() > MAX_JS_SAFE_INTEGER
            })
            || self.pending.is_some_and(|pending| {
                !safe(pending.revision) || pending.at_ms.unsigned_abs() > MAX_JS_SAFE_INTEGER
            })
            || self.legacy.as_ref().is_some_and(|legacy| {
                !safe(legacy.cron_id) || self.room_id.as_deref() != Some(legacy.room_id.as_str())
            })
        {
            return Err(JobError::Invalid);
        }
        if let JobSchedule::Every {
            seconds, anchor_ms, ..
        } = self.schedule
            && (seconds <= 0
                || seconds.checked_mul(1000).is_none()
                || anchor_ms.unsigned_abs() > MAX_JS_SAFE_INTEGER)
        {
            return Err(JobError::Invalid);
        }
        Ok(())
    }
    pub fn display_id(&self) -> String {
        format!("j{}", self.number)
    }
}

pub struct JobAdmission {
    pub expected_revision: u64,
    pub owner_active: bool,
    pub routine: OrchRoutine,
    pub slot_ms: i64,
    pub manual: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobDecision {
    Paused,
    NoOwner,
    Hold(HeldJobSlot),
    Dispatch { slot_ms: i64 },
}
/// A state hold coalesces to one observed slot. A manual pause is independent
/// of plans and wake; an explicit send neither clears it nor consumes catch-up.
pub fn admit_job(job: &OrchJob, input: &JobAdmission) -> Result<JobDecision, JobError> {
    job.validate()?;
    if job.revision != input.expected_revision
        || job
            .pending
            .is_some_and(|pending| pending.revision != job.revision)
    {
        return Err(JobError::Conflict);
    }
    if input.slot_ms.unsigned_abs() > MAX_JS_SAFE_INTEGER {
        return Err(JobError::Invalid);
    }
    if job.owner_id.is_none() || !input.owner_active {
        return Ok(JobDecision::NoOwner);
    }
    if input.manual {
        return Ok(JobDecision::Dispatch {
            slot_ms: input.slot_ms,
        });
    }
    if job.pause.is_some() {
        return Ok(JobDecision::Paused);
    }
    let pending = job.pending.unwrap_or(HeldJobSlot {
        revision: job.revision,
        at_ms: input.slot_ms,
    });
    if input.routine == OrchRoutine::Hold {
        return Ok(JobDecision::Hold(pending));
    }
    Ok(JobDecision::Dispatch {
        slot_ms: pending.at_ms,
    })
}

/// Canonical slot identity. Imported jobs retain their shipped room/c-id domain;
/// revisions are intentionally absent so changed intent cannot replay a slot.
pub fn job_operation_id(job: &OrchJob, slot_ms: i64) -> String {
    use sha2::{Digest, Sha256};
    let mut digest = Sha256::new();
    if let Some(legacy) = &job.legacy {
        digest.update(b"tmt-squad-cron-slot-v1\0");
        for value in [&legacy.room_id, &format!("c{}", legacy.cron_id)] {
            digest.update((value.len() as u64).to_be_bytes());
            digest.update(value.as_bytes());
        }
    } else {
        digest.update(b"tmt-agent-job-slot-v1\0");
        digest.update((job.id.len() as u64).to_be_bytes());
        digest.update(job.id.as_bytes());
    }
    digest.update(slot_ms.to_be_bytes());
    let mut bytes: [u8; 16] = digest.finalize()[..16].try_into().expect("SHA-256 prefix");
    bytes[6] = (bytes[6] & 0x0f) | 0x80;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    uuid::Uuid::from_bytes(bytes).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn job() -> OrchJob {
        OrchJob {
            id: "11111111-1111-4111-8111-111111111111".into(),
            number: 1,
            revision: 1,
            owner_id: Some("22222222-2222-4222-8222-222222222222".into()),
            room_id: None,
            title: Some("Queue review".into()),
            message: "Review the waiting queue.".into(),
            schedule: JobSchedule::Every {
                zone: "Asia/Tokyo".into(),
                seconds: 10,
                anchor_ms: 0,
                from: None,
            },
            pause: None,
            pending: None,
            legacy: None,
        }
    }
    fn at(slot_ms: i64, routine: OrchRoutine) -> JobAdmission {
        JobAdmission {
            expected_revision: 1,
            owner_active: true,
            routine,
            slot_ms,
            manual: false,
        }
    }
    #[test]
    fn imported_slot_keeps_the_shipped_operation_uuid() {
        let mut job = job();
        job.room_id = Some("11111111-1111-4111-8111-111111111111".into());
        job.legacy = Some(LegacyJob {
            room_id: job.room_id.clone().unwrap(),
            cron_id: 1,
        });
        // Frozen pre-cutover receipt from the Ops cron tick contract.
        assert_eq!(
            job_operation_id(&job, 1_700_000_000_000),
            "b17e22b9-5806-8721-984c-36c414c6f81b"
        );
        let original = job_operation_id(&job, 1_700_000_000_000);
        job.revision += 1;
        assert_eq!(job_operation_id(&job, 1_700_000_000_000), original);
    }
    #[test]
    fn routine_holds_one_slot_and_wakes_once_without_reconstructing_absence() {
        let mut job = job();
        let JobDecision::Hold(held) = admit_job(&job, &at(10_000, OrchRoutine::Hold)).unwrap()
        else {
            panic!("routine must hold");
        };
        job.pending = Some(held);
        assert_eq!(
            admit_job(&job, &at(90_000, OrchRoutine::Hold)).unwrap(),
            JobDecision::Hold(held)
        );
        assert_eq!(
            admit_job(&job, &at(100_000, OrchRoutine::Run)).unwrap(),
            JobDecision::Dispatch { slot_ms: 10_000 }
        );
        job.pending = None; // The storage owner clears only after canonical admission.
        assert_eq!(
            admit_job(&job, &at(110_000, OrchRoutine::Run)).unwrap(),
            JobDecision::Dispatch { slot_ms: 110_000 }
        );
    }
    #[test]
    fn manual_pause_survives_wake_and_explicit_send() {
        let mut job = job();
        job.pause = Some(JobPause {
            by: "user".into(),
            at_ms: 1,
        });
        assert_eq!(
            admit_job(&job, &at(10_000, OrchRoutine::Run)).unwrap(),
            JobDecision::Paused
        );
        let mut input = at(20_000, OrchRoutine::Hold);
        input.manual = true;
        assert_eq!(
            admit_job(&job, &input).unwrap(),
            JobDecision::Dispatch { slot_ms: 20_000 }
        );
        assert!(job.pause.is_some());
        assert_eq!(
            admit_job(&job, &at(30_000, OrchRoutine::Run)).unwrap(),
            JobDecision::Paused
        );
    }
    #[test]
    fn stale_revision_and_missing_owner_never_receive_an_old_slot() {
        let mut job = job();
        job.pending = Some(HeldJobSlot {
            revision: 1,
            at_ms: 10_000,
        });
        job.revision = 2;
        let mut input = at(20_000, OrchRoutine::Run);
        input.expected_revision = 2;
        assert_eq!(admit_job(&job, &input), Err(JobError::Conflict));
        job.pending = None;
        job.owner_id = None;
        job.pause = Some(JobPause {
            by: "owner retired".into(),
            at_ms: 1,
        });
        assert_eq!(admit_job(&job, &input).unwrap(), JobDecision::NoOwner);
        assert_eq!(job.display_id(), "j1");
    }
}
