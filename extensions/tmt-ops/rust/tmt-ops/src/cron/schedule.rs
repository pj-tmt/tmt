//! Ops projection/error adapter over the shared stateless calendar owner.
use super::Error;
use serde_json::Value;
pub use tmt_job_calendar::ScheduleInput;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Schedule(tmt_job_calendar::Schedule);
impl From<tmt_job_calendar::Error> for Error {
    fn from(error: tmt_job_calendar::Error) -> Self {
        Self::new("SQUAD_CRON_SCHEDULE_INVALID", error.message)
    }
}
impl Schedule {
    pub fn local_zone() -> Result<String, Error> {
        tmt_job_calendar::Schedule::local_zone().map_err(Into::into)
    }
    pub fn parse(input: ScheduleInput<'_>, zone: &str, now_ms: i64) -> Result<Self, Error> {
        tmt_job_calendar::Schedule::parse(input, zone, now_ms)
            .map(Self)
            .map_err(Into::into)
    }
    pub fn next_after(&self, after_ms: i64) -> Result<Option<i64>, Error> {
        self.0.next_after(after_ms).map_err(Into::into)
    }
    pub fn document(&self) -> Value {
        self.0.document()
    }
    pub fn from_document(value: &Value) -> Result<Self, Error> {
        tmt_job_calendar::Schedule::from_document(value)
            .map(Self)
            .map_err(Into::into)
    }
    pub fn readable(&self) -> String {
        self.0.readable()
    }
}
