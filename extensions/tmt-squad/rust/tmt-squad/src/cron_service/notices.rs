//! Best-effort change announcements; never a durable outbox or a dispatch under lock.
use super::*;
use sha2::{Digest, Sha256};

pub(super) fn operation(job: &Job, action: &str, recipient: &str) -> String {
    let bytes = Sha256::digest(format!(
        "squad-cron\0{}\0{}\0{}\0{action}\0{recipient}",
        job.room_id,
        job.id(),
        job.revision
    ));
    let mut bytes: [u8; 16] = bytes[..16].try_into().expect("digest prefix");
    bytes[6] = (bytes[6] & 15) | 0x50;
    bytes[8] = (bytes[8] & 63) | 0x80;
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    )
}
pub(super) fn send(
    core: &Core,
    job: &Job,
    action: &str,
    recipient: &str,
    message: &str,
    actor: Option<&CronActor>,
) -> Result<(), SquadError> {
    let receipt = core.api_write(
        "dispatch.create",
        json!({
            "operationId": operation(job, action, recipient), "recipientIds": [recipient],
            "message": message, "kind": "announcement"
        }),
        actor.map(|actor| actor.id.as_str()),
    )?;
    if receipt["items"].as_array().is_none_or(|items| {
        items.len() != 1
            || items[0]["recipientId"] != recipient
            || items[0]["acceptance"] != "queued"
    }) {
        return Err(failure(
            "SQUAD_CRON_NOTICE_FAILED",
            "The job committed, but its change announcement was not queued.",
        ));
    }
    Ok(())
}
pub(super) fn line(job: &Job, detail: &str) -> String {
    format!(
        "▚ ⏱ {} {} · {}",
        job.squad,
        job.id(),
        tmt_cli_style::table::escape(detail)
    )
}
pub(super) fn changed(
    core: &Core,
    actor: &CronActor,
    before: Option<&Job>,
    job: &Job,
    action: &str,
) -> Vec<SquadError> {
    let mut warnings = Vec::new();
    let mut recipients = Vec::new();
    if action == "reassigned"
        && let Some(id) = before.and_then(|job| job.owner_id.as_deref())
    {
        recipients.push(id);
    }
    if let Some(id) = job.owner_id.as_deref()
        && !recipients.contains(&id)
    {
        recipients.push(id);
    }
    for recipient in recipients {
        if recipient == actor.id {
            continue;
        }
        let detail = match action {
            "added" => format!("{} · added by {}", schedule_text(&job.schedule), actor.name),
            "edited" => {
                let old = before.expect("edited job has a before value");
                let mut edits = Vec::new();
                if old.schedule != job.schedule {
                    edits.push(format!(
                        "{} → {}",
                        schedule_text(&old.schedule),
                        schedule_text(&job.schedule)
                    ));
                }
                if old.message != job.message {
                    edits.push("message changed".into());
                }
                format!("{} · edited by {}", edits.join(" · "), actor.name)
            }
            "reassigned" => {
                let name = job
                    .owner_id
                    .as_deref()
                    .and_then(|id| active_identity(core, id).ok())
                    .unwrap_or_else(|| "new owner".into());
                let message = if job.owner_id.as_deref() == Some(recipient) {
                    format!(" · message: {}", job.message)
                } else {
                    String::new()
                };
                format!(
                    "now owned by {name} · reassigned by {}{message}",
                    actor.name
                )
            }
            _ => format!("{action} by {}", actor.name),
        };
        if let Err(error) = send(
            core,
            job,
            action,
            recipient,
            &line(job, &detail),
            Some(actor),
        ) {
            warnings.push(error);
        }
    }
    warnings
}
