//! Persist user-authored, library-scoped paper sets independently of caches.

use std::collections::HashSet;

use chrono::Utc;
use rusqlite::{params, Connection, TransactionBehavior};
use serde::Deserialize;

use super::{meta_value, ResearchDatabase};
use crate::error::{AppError, AppResult};
use crate::model::ResearchLens;

const MAX_LENS_MEMBERS: usize = 1200;

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SaveLensRequest {
    pub id: Option<String>,
    pub revision: Option<i64>,
    pub library_id: String,
    pub name: String,
    pub description: String,
    pub paper_ids: Vec<String>,
}

fn invalid(message: &str) -> AppError {
    AppError::ApplicationData(message.to_owned())
}

fn require_library(connection: &Connection, library_id: &str) -> AppResult<()> {
    if meta_value(connection, "active_library_id")?.as_deref() != Some(library_id) {
        return Err(invalid(
            "The active library changed. Reopen the Lens editor.",
        ));
    }
    Ok(())
}

pub(super) fn load_lenses(
    connection: &Connection,
    library_id: &str,
) -> AppResult<Vec<ResearchLens>> {
    // Keep revision and membership in the same read snapshot. Writers
    // already hold a transaction and must not start a nested one.
    let _snapshot = if connection.is_autocommit() {
        Some(connection.unchecked_transaction()?)
    } else {
        None
    };
    let mut statement = connection.prepare(
        "SELECT id, name, question, revision FROM research_lenses
         WHERE library_id = ?1 ORDER BY name COLLATE NOCASE, id",
    )?;
    let mut lenses = statement
        .query_map([library_id], |row| {
            Ok(ResearchLens {
                id: row.get(0)?,
                name: row.get(1)?,
                description: row.get(2)?,
                revision: row.get(3)?,
                library_id: library_id.to_owned(),
                paper_ids: Vec::new(),
                unavailable_paper_ids: Vec::new(),
                groups: Vec::new(),
                saved: true,
                paper_titles: std::collections::HashMap::new(),
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let mut members = connection.prepare(
        "SELECT m.item_id, EXISTS(SELECT 1 FROM research_items i
            WHERE i.id = m.item_id AND i.library_id = ?2 AND i.is_trashed = 0),
            (SELECT i.title FROM research_items i WHERE i.id = m.item_id AND i.library_id = ?2)
         FROM research_lens_members m WHERE m.lens_id = ?1 ORDER BY m.position",
    )?;
    for lens in &mut lenses {
        let rows = members.query_map(params![lens.id, library_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, bool>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })?;
        for row in rows {
            let (id, available, title) = row?;
            if let Some(title) = title {
                lens.paper_titles.insert(id.clone(), title);
            }
            if !available {
                lens.unavailable_paper_ids.push(id.clone());
            }
            lens.paper_ids.push(id);
        }
    }
    Ok(lenses)
}

impl ResearchDatabase {
    pub fn list_lenses(&self, library_id: &str) -> AppResult<Vec<ResearchLens>> {
        let connection = self.connect()?;
        require_library(&connection, library_id)?;
        load_lenses(&connection, library_id)
    }

    /// Atomically save a fixed membership, rejecting stale or cross-library edits.
    pub fn save_lens(&self, request: &SaveLensRequest) -> AppResult<ResearchLens> {
        let name = request.name.trim();
        let question = request.description.trim();
        if name.is_empty() || name.chars().count() > 120 || question.chars().count() > 4000 {
            return Err(invalid(
                "Use a Lens name of 1-120 characters and a question of at most 4000 characters.",
            ));
        }
        if request.paper_ids.len() > MAX_LENS_MEMBERS {
            return Err(invalid(
                "A Lens can contain at most 1200 papers. Narrow the selection.",
            ));
        }
        let mut connection = self.connect()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        require_library(&transaction, &request.library_id)?;
        let previous = load_lenses(&transaction, &request.library_id)?
            .into_iter()
            .find(|lens| Some(&lens.id) == request.id.as_ref());
        if request.id.is_some() && previous.as_ref().map(|lens| lens.revision) != request.revision {
            return Err(invalid(
                "This Lens changed or was deleted. Close and reopen the editor before saving.",
            ));
        }
        if request.id.is_some() && previous.is_none()
            || request.id.is_none() && request.revision.is_some()
        {
            return Err(invalid("Lens identity is invalid. Reopen the editor."));
        }
        let mut unique = HashSet::new();
        let ids: Vec<_> = request
            .paper_ids
            .iter()
            .filter(|id| unique.insert((*id).clone()))
            .collect();
        for id in &ids {
            let available: bool = transaction.query_row(
                "SELECT EXISTS(SELECT 1 FROM research_items
                 WHERE id = ?1 AND library_id = ?2 AND is_trashed = 0)",
                params![id, request.library_id],
                |row| row.get(0),
            )?;
            if !available
                && !previous
                    .as_ref()
                    .is_some_and(|lens| lens.paper_ids.contains(id))
            {
                return Err(invalid("A selected paper is unavailable in this library. Refresh the selection before saving."));
            }
        }
        let id = match &previous {
            Some(lens) => lens.id.clone(),
            None => transaction.query_row(
                "SELECT 'lens:' || lower(hex(randomblob(16)))",
                [],
                |row| row.get::<_, String>(0),
            )?,
        };
        let now = Utc::now().to_rfc3339();
        transaction.execute(
            "INSERT INTO research_lenses(id, library_id, name, question, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?5)
             ON CONFLICT(id) DO UPDATE SET name = excluded.name, question = excluded.question,
                revision = research_lenses.revision + 1, updated_at = excluded.updated_at",
            params![id, request.library_id, name, question, now],
        )?;
        transaction.execute(
            "DELETE FROM research_lens_members WHERE lens_id = ?1",
            [&id],
        )?;
        for (position, item_id) in ids.iter().enumerate() {
            transaction.execute(
                "INSERT INTO research_lens_members VALUES (?1, ?2, ?3)",
                params![id, item_id, position as i64],
            )?;
        }
        let saved = load_lenses(&transaction, &request.library_id)?
            .into_iter()
            .find(|lens| lens.id == id)
            .ok_or_else(|| invalid("Saved Lens could not be read."))?;
        transaction.commit()?;
        Ok(saved)
    }

    pub fn delete_lens(&self, library_id: &str, id: &str, revision: i64) -> AppResult<()> {
        let mut connection = self.connect()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        require_library(&transaction, library_id)?;
        let changed = transaction.execute(
            "DELETE FROM research_lenses WHERE id = ?1 AND library_id = ?2 AND revision = ?3",
            params![id, library_id, revision],
        )?;
        if changed != 1 {
            return Err(invalid(
                "This Lens changed or was deleted. Reopen the editor.",
            ));
        }
        transaction.commit()?;
        Ok(())
    }
}
