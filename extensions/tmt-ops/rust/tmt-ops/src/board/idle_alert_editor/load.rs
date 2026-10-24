//! Configuration effects on the existing ordered action lane.
use crate::{
    config::Config,
    core::{Core, SquadError},
};

#[derive(Clone)]
pub(in crate::board) struct Task {
    pub owner: u64,
    pub room: String,
    pub name: String,
    pub save: Option<(Box<Config>, bool, String)>,
}
impl std::fmt::Debug for Task {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IdleAlertTask")
            .field("owner", &self.owner)
            .field("room", &self.room)
            .field("saving", &self.save.is_some())
            .finish()
    }
}
impl PartialEq for Task {
    fn eq(&self, other: &Self) -> bool {
        self.owner == other.owner
            && self.room == other.room
            && self.name == other.name
            && self.save.as_ref().map(|(_, on, after)| (on, after))
                == other.save.as_ref().map(|(_, on, after)| (on, after))
    }
}
impl Eq for Task {}
pub(in crate::board) struct Completed {
    pub task: Task,
    pub result: Result<Box<Config>, SquadError>,
    pub recovered: Option<Box<Config>>,
}
pub(in crate::board) fn complete(core: &Core, task: Task) -> Completed {
    let mut recovered = None;
    let result = (|| {
        let shown = core.json(&["room", "show", &task.room])?;
        let room = &shown["room"];
        if room["id"] != task.room
            || room["name"] != crate::squad::room_name(&task.name)
            || room["retired"] != false
        {
            return Err(SquadError::new(
                "ROOM_REVISION_CONFLICT",
                "The squad changed. Close and reopen Idle alert.",
            ));
        }
        if let Some((config, enabled, after)) = &task.save {
            let mut config = (**config).clone();
            let original = config.idle_alert(&task.name)?;
            let validated = config.preview_setting(Some(&task.name), "idle_alert.after", after)?;
            validated.preview_setting(
                Some(&task.name),
                "idle_alert.enabled",
                &enabled.to_string(),
            )?;
            let after_changed = original.after != validated.idle_alert(&task.name)?.after;
            if after_changed {
                config.set_setting(Some(&task.name), "idle_alert.after", after)?;
            }
            if original.enabled != *enabled
                && let Err(error) =
                    config.set_setting(Some(&task.name), "idle_alert.enabled", &enabled.to_string())
            {
                if after_changed {
                    let message = match Config::load(core) {
                        Ok(fresh) => {
                            recovered = Some(Box::new(fresh));
                            "After saved, Alert not changed".to_owned()
                        }
                        Err(read_error) => {
                            recovered = Some(Box::new(config));
                            format!(
                                "After saved, Alert not changed. Current setting unavailable: {}",
                                read_error.message
                            )
                        }
                    };
                    return Err(SquadError::new(&error.code, message));
                }
                return Err(error);
            }
            Ok(config)
        } else {
            let config = Config::load(core)?;
            config.idle_alert(&task.name)?;
            Ok(config)
        }
    })();
    Completed {
        task,
        result: result.map(Box::new),
        recovered,
    }
}
