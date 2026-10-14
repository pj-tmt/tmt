//! Consented extension-owned skill admission and publication.

use super::{Fault, Request, invalid};
use crate::skill_installation;
use serde::Deserialize;
use serde_json::json;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SkillFileInput {
    path: String,
    content: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SkillInput {
    name: String,
    files: Vec<SkillFileInput>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SkillsInstallInput {
    owner: String,
    consent: bool,
    #[serde(default)]
    force: bool,
    skills: Vec<SkillInput>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SkillsRemoveInput {
    owner: String,
    consent: bool,
    /// Only these of the owner's skills; absent means all of them.
    skills: Option<Vec<String>>,
}

/// Skills write into the user's provider directories, so the caller must
/// have asked the user first and say so explicitly.
fn consented(consent: bool) -> Result<(), Fault> {
    if consent {
        Ok(())
    } else {
        Err(Fault::new(
            "API_CONSENT_REQUIRED",
            "Skill installation changes the user's agent directories; ask the user, then send consent: true.",
        ))
    }
}

pub(super) fn decode_install(input: &[u8]) -> Result<Request, Fault> {
    Ok({
        let value: SkillsInstallInput = serde_json::from_slice(input).map_err(|_| invalid())?;
        consented(value.consent)?;
        Request::SkillsInstall {
            owner: value.owner,
            force: value.force,
            skills: value
                .skills
                .into_iter()
                .map(|skill| skill_installation::OwnedSkill {
                    name: skill.name,
                    files: skill
                        .files
                        .into_iter()
                        .map(|file| (file.path, file.content.into_bytes()))
                        .collect(),
                })
                .collect(),
        }
    })
}

pub(super) fn decode_remove(input: &[u8]) -> Result<Request, Fault> {
    Ok({
        let value: SkillsRemoveInput = serde_json::from_slice(input).map_err(|_| invalid())?;
        consented(value.consent)?;
        Request::SkillsRemove {
            owner: value.owner,
            skills: value.skills,
        }
    })
}

fn skill_fault(failure: &skill_installation::OwnedFailure) -> Fault {
    let code = match skill_installation::refusal(&failure.cause) {
        Some(skill_installation::Refusal::Invalid(_)) => "SKILL_INVALID",
        Some(skill_installation::Refusal::Claimed { .. }) => "SKILL_OWNED_ELSEWHERE",
        Some(skill_installation::Refusal::Unmanaged(_)) => "SKILL_CONFLICT",
        None => "SKILL_INSTALL_FAILED",
    };
    Fault::detailed(code, failure.to_string())
}

pub(super) fn install(
    global: &std::path::Path,
    owner: &str,
    skills: &[skill_installation::OwnedSkill],
    force: bool,
) -> Result<Vec<u8>, Fault> {
    let env = skill_installation::ProviderEnvironment::capture().map_err(|_| {
        Fault::new(
            "SKILL_INSTALL_FAILED",
            "Could not determine the home directory for agent skills.",
        )
    })?;
    let report = skill_installation::install_owned(&env, global, owner, skills, force)
        .map_err(|failure| skill_fault(&failure))?;
    Ok(serde_json::to_vec(&json!({
        "owner": owner,
        "published": report.published.iter().map(|item| json!({
            "name": item.name,
            "agent": item.agent.map(|agent| agent.name()),
            "target": item.target,
            "changed": item.changed,
            "backup": item.backup,
        })).collect::<Vec<_>>(),
    }))
    .expect("installation report"))
}

pub(super) fn remove(
    global: &std::path::Path,
    owner: &str,
    skills: Option<Vec<String>>,
) -> Result<Vec<u8>, Fault> {
    skill_installation::remove_owned(global, owner, skills.as_deref())
        .map(|report| {
            serde_json::to_vec(&json!({
                "owner": owner,
                "removed": report.removed,
                "kept": report.kept,
            }))
            .expect("removal report")
        })
        .map_err(|failure| skill_fault(&failure))
}
