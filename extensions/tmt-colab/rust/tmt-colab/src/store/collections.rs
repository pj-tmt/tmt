//! Collection persistence. Caller-admitted writes use the existing owner writer transaction;
//! references and person Seen marks never modify pages, membership, originals or receipts.
use super::Store;
use crate::{
    Result,
    collections::{
        self, Collection, Command, Definition, Detail, Fault, Item, Outcome, Seen, Sort,
    },
};
use rusqlite::{Connection, OptionalExtension, params};

fn collection(connection: &Connection, id: &str) -> Result<Collection> {
    let value: Option<Vec<u8>> = connection
        .query_row("SELECT value FROM collections WHERE id=?", [id], |row| {
            row.get(0)
        })
        .optional()?;
    let value: Collection = serde_json::from_slice(&value.ok_or(Fault::NotFound)?)?;
    if !value.valid() || value.id != id {
        return Err(Fault::Invalid.into());
    }
    Ok(value)
}

pub(super) fn detail(connection: &Connection, id: &str) -> Result<Detail> {
    let collection = collection(connection, id)?;
    let mut rows = connection
        .prepare("SELECT value FROM collection_items WHERE collection=? ORDER BY position,item")?;
    let items = rows
        .query_map([id], |row| row.get::<_, Vec<u8>>(0))?
        .map(|value| Ok(serde_json::from_slice::<Item>(&value?)?))
        .collect::<Result<Vec<_>>>()?;
    if items.iter().any(|item| !item.valid()) || items.len() > collections::ITEMS {
        return Err(Fault::Invalid.into());
    }
    let mut rows = connection
        .prepare("SELECT target,at_ms FROM collection_seen WHERE collection=? ORDER BY target")?;
    let seen = rows
        .query_map([id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?
        .map(|row| -> Result<Seen> {
            let (id, at) = row?;
            let at_ms = u64::try_from(at)?;
            tmt_colab_model::values::time(at_ms)?;
            Ok(Seen { id, at_ms })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Detail {
        collection,
        items,
        seen,
    })
}

fn write_collection(connection: &Connection, value: &Collection) -> Result<()> {
    if !value.valid() {
        return Err(Fault::Invalid.into());
    }
    connection.execute(
        "UPDATE collections SET value=? WHERE id=?",
        params![serde_json::to_vec(value)?, value.id],
    )?;
    Ok(())
}

fn page_exists(connection: &Connection, page: &str) -> Result<bool> {
    Ok(connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM epoch_secrets WHERE page=?)",
        [page],
        |row| row.get(0),
    )?)
}

fn clear_seen(connection: &Connection, id: &str) -> Result<bool> {
    Ok(connection.execute("DELETE FROM collection_seen WHERE collection=?", [id])? > 0)
}

/// Runs only within label_transaction: capacity and target checks share its commit boundary.
pub(super) fn apply(
    connection: &Connection,
    command: &Command,
    person: bool,
    now: u64,
) -> Result<Outcome> {
    if !command.valid(person) {
        return Err(Fault::Invalid.into());
    }
    if let Command::Create { collection: value } = command {
        let previous: Option<Vec<u8>> = connection
            .query_row(
                "SELECT value FROM collections WHERE id=?",
                [&value.id],
                |row| row.get(0),
            )
            .optional()?;
        let changed = if let Some(previous) = previous {
            if serde_json::from_slice::<Collection>(&previous)? != *value {
                return Err(Fault::Conflict.into());
            }
            false
        } else {
            let count: i64 =
                connection.query_row("SELECT COUNT(*) FROM collections", [], |r| r.get(0))?;
            if count >= collections::COLLECTIONS as i64 {
                return Err(Fault::Capacity.into());
            }
            connection.execute(
                "INSERT INTO collections(id,value) VALUES (?,?)",
                params![value.id, serde_json::to_vec(value)?],
            )?;
            true
        };
        return Ok(Outcome {
            detail: detail(connection, &value.id)?,
            changed,
        });
    }
    let id = match command {
        Command::List | Command::Show { .. } | Command::Create { .. } => {
            return Err(Fault::Invalid.into());
        }
        Command::Add { id, .. }
        | Command::Remove { id, .. }
        | Command::Rename { id, .. }
        | Command::Maintainer { id, .. }
        | Command::Sort { id, .. }
        | Command::Query { id, .. }
        | Command::Archive { id, .. }
        | Command::Seen { id, .. }
        | Command::ClearSeen { id } => id,
    };
    let mut value = collection(connection, id)?;
    let before = value.clone();
    let changed = match command {
        Command::Add { item, .. } => {
            if !value.definition.manual() {
                return Err(Fault::Invalid.into());
            }
            let previous: Option<Vec<u8>> = connection
                .query_row(
                    "SELECT value FROM collection_items WHERE collection=? AND item=?",
                    params![id, item.id],
                    |row| row.get(0),
                )
                .optional()?;
            if let Some(previous) = previous {
                if serde_json::from_slice::<Item>(&previous)? != *item {
                    return Err(Fault::Conflict.into());
                }
                false
            } else {
                if !page_exists(connection, &item.page_id)? {
                    return Err(Fault::Invalid.into());
                }
                let (count, position): (i64, i64) = connection.query_row(
                    "SELECT COUNT(*),COALESCE(MAX(position)+1,0) FROM collection_items WHERE collection=?", [id], |r| Ok((r.get(0)?,r.get(1)?)),
                )?;
                let total: i64 =
                    connection
                        .query_row("SELECT COUNT(*) FROM collection_items", [], |r| r.get(0))?;
                if count >= collections::ITEMS as i64 || total >= collections::SPACE_ITEMS as i64 {
                    return Err(Fault::Capacity.into());
                }
                connection.execute(
                    "INSERT INTO collection_items(collection,item,position,value) VALUES (?,?,?,?)",
                    params![id, item.id, position, serde_json::to_vec(item)?],
                )?;
                true
            }
        }
        Command::Remove { item, .. } => {
            if !value.definition.manual() {
                return Err(Fault::Invalid.into());
            }
            connection.execute(
                "DELETE FROM collection_seen WHERE collection=? AND target=?",
                params![id, item],
            )?;
            connection.execute(
                "DELETE FROM collection_items WHERE collection=? AND item=?",
                params![id, item],
            )? > 0
        }
        Command::Rename { name, .. } => {
            value.name.clone_from(name);
            value != before
        }
        Command::Maintainer { maintainer, .. } => {
            value.maintainer.clone_from(maintainer);
            value != before
        }
        Command::Archive { archived, .. } => {
            value.archived = *archived;
            value != before
        }
        Command::Sort { sort, .. } => {
            match &mut value.definition {
                Definition::Manual { sort: current } => *current = *sort,
                Definition::Smart { search, .. } => {
                    search.sort = match sort {
                        Sort::Updated => None,
                        Sort::Title => Some(Sort::Title),
                        Sort::Manual => return Err(Fault::Invalid.into()),
                    }
                }
            }
            value != before
        }
        Command::Query {
            search, needs_you, ..
        } => {
            if value.definition.manual() {
                return Err(Fault::Invalid.into());
            }
            value.definition = Definition::Smart {
                search: search.clone(),
                needs_you: *needs_you,
            };
            if value != before {
                clear_seen(connection, id)?;
            }
            value != before
        }
        Command::Seen { target, .. } => {
            let exists = if value.definition.manual() {
                connection.query_row(
                    "SELECT EXISTS(SELECT 1 FROM collection_items WHERE collection=? AND item=?)",
                    params![id, target],
                    |r| r.get(0),
                )?
            } else {
                page_exists(connection, target)?
            };
            if !exists {
                return Err(Fault::Invalid.into());
            }
            tmt_colab_model::values::time(now)?;
            let now = i64::try_from(now)?;
            connection.execute("INSERT INTO collection_seen(collection,target,at_ms) VALUES (?,?,?) ON CONFLICT(collection,target) DO NOTHING", params![id,target,now])? > 0
        }
        Command::ClearSeen { .. } => clear_seen(connection, id)?,
        _ => return Err(Fault::Invalid.into()),
    };
    if value != before {
        write_collection(connection, &value)?;
    }
    Ok(Outcome {
        detail: detail(connection, id)?,
        changed,
    })
}

impl Store {
    pub fn collection_directory(&self) -> Result<Vec<Collection>> {
        let mut rows = self
            .connection
            .prepare("SELECT value FROM collections ORDER BY id")?;
        let values = rows
            .query_map([], |row| row.get::<_, Vec<u8>>(0))?
            .map(|value| Ok(serde_json::from_slice::<Collection>(&value?)?))
            .collect::<Result<Vec<_>>>()?;
        if values.len() > collections::COLLECTIONS || values.iter().any(|value| !value.valid()) {
            return Err(Fault::Invalid.into());
        }
        Ok(values)
    }
    /// Keep the definition, item positions and Seen marks in one read snapshot.
    pub fn collection_detail(&self, id: &str) -> Result<Detail> {
        tmt_colab_model::values::generated_id(id).map_err(|_| Fault::Invalid)?;
        let tx = self.connection.unchecked_transaction()?;
        let detail = detail(&tx, id)?;
        tx.commit()?;
        Ok(detail)
    }
}
