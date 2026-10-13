//! Confirmed workspace operations use public Core ports and the same Ops
//! membership service as CLI commands. Frozen room/identity IDs are authority;
//! display names never substitute a different target.
use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomTarget {
    pub id: String,
    pub name: String,
    pub revision: u64,
}
impl RoomTarget {
    fn admit(&self, core: &Core) -> Result<Squad, SquadError> {
        let shown = core.json(&["room", "show", &self.id])?;
        let room = &shown["room"];
        if room["id"] != self.id
            || room["name"] != squad::room_name(&self.name)
            || room["revision"] != self.revision
            || room["retired"] != false
        {
            return Err(SquadError::new(
                "ROOM_REVISION_CONFLICT",
                "The displayed squad changed; close and refresh before applying.",
            ));
        }
        Ok(Squad {
            name: self.name.clone(),
            room_id: self.id.clone(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Operation {
    Create {
        name: String,
        lead: Option<String>,
        members: Vec<String>,
    },
    Add {
        room: RoomTarget,
        member: String,
    },
    Lead {
        room: RoomTarget,
        member: Option<String>,
    },
    Remove {
        room: RoomTarget,
        member: String,
    },
    Assign {
        source: Option<RoomTarget>,
        destination: RoomTarget,
        member: String,
        move_member: bool,
    },
    RenameSquad {
        room: RoomTarget,
        name: String,
    },
}
fn failed(mut document: Value, stage: &str, error: SquadError) -> Outcome {
    document["failed"] = json!({"stage":stage,"status":"Unknown or partially applied","error":{"code":error.code,"message":error.message}});
    Outcome {
        document,
        complete: false,
    }
}
pub fn apply(
    core: &Core,
    config: &mut Config,
    operation: Operation,
) -> Result<Outcome, SquadError> {
    match operation {
        Operation::Create {
            name,
            lead: leader,
            members,
        } => {
            if !valid_name(&name) {
                return Err(name_invalid(&name));
            }
            let layout = config.layout(&name)?;
            // Resolve every requested identity before creating anything.
            let members = members
                .iter()
                .map(|id| saved(core, id).map(|identity| identity.id))
                .collect::<Result<Vec<_>, _>>()?;
            let leader = leader
                .map(|id| saved(core, &id).map(|identity| identity.id))
                .transpose()?;
            match core.json(&["room", "show", &squad::room_name(&name)]) {
                Err(error) if error.code == "ROOM_NOT_FOUND" => {}
                Err(error) => return Err(error),
                Ok(_) => {
                    return Err(SquadError::new(
                        "SQUAD_ALREADY_EXISTS",
                        "That squad already exists; nothing created.",
                    ));
                }
            }
            let created = init(core, config, &name, None)?;
            if created.document["created"] != true {
                return Err(SquadError::new(
                    "SQUAD_ALREADY_EXISTS",
                    "That squad now exists; no members were changed.",
                ));
            }
            let room_id = created.document["squad"]["roomId"]
                .as_str()
                .filter(|id| crate::config::uuid_like(id))
                .ok_or_else(|| {
                    SquadError::new(
                        "SQUAD_CORE_UNAVAILABLE",
                        "Core did not confirm the new room UUID; refresh before trying again.",
                    )
                })?;
            let squad = Squad {
                name: name.clone(),
                room_id: room_id.into(),
            };
            let mut document = json!({"created":created.document,"members":null,"lead":null});
            match add(core, &squad, layout, &members) {
                Ok(added) => {
                    document["members"] = added.document;
                    if !added.complete {
                        return Ok(Outcome {
                            document,
                            complete: false,
                        });
                    }
                }
                Err(error) => return Ok(failed(document, "members", error)),
            }
            match lead(core, &squad, leader.as_deref()) {
                Ok(changed) => {
                    document["lead"] = changed.document;
                    Ok(Outcome {
                        document,
                        complete: changed.complete,
                    })
                }
                Err(error) => Ok(failed(document, "lead", error)),
            }
        }
        Operation::Add { room, member } => {
            let squad = room.admit(core)?;
            add(core, &squad, config.layout(&squad.name)?, &[member])
        }
        Operation::Lead { room, member } => lead(core, &room.admit(core)?, member.as_deref()),
        Operation::Remove { room, member } => remove(core, &room.admit(core)?, &member),
        Operation::Assign {
            source,
            destination,
            member,
            move_member,
        } => {
            let source = source.map(|room| room.admit(core)).transpose()?;
            let destination = destination.admit(core)?;
            assign(
                core,
                source.as_ref(),
                &destination,
                &member,
                move_member,
                config.layout(&destination.name)?,
            )
        }
        Operation::RenameSquad { room, name } => {
            room.admit(core)?;
            rename_squad(
                core,
                config,
                &Rename {
                    room_id: room.id,
                    old: room.name,
                    new: name,
                    expected_revision: Some(room.revision),
                    repair: false,
                },
            )
        }
    }
}
