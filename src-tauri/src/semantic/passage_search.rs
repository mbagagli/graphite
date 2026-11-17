//! Rank scientific chunks lexically using the existing SQLite FTS5 engine.

use std::collections::HashSet;

use regex::Regex;
use rusqlite::{params, Connection};

use crate::error::AppResult;

const MAX_QUERY_TERMS: usize = 32;

/// Build quoted FTS terms, preferring explicit numeric or figure/table targets.
fn passage_query(question: &str) -> Option<String> {
    let decimal = Regex::new(r"\b\d+\.\d+\b").expect("decimal pattern");
    let numbers: Vec<_> = decimal
        .find_iter(question)
        .map(|value| format!("\"{}\"", value.as_str()))
        .collect();
    if !numbers.is_empty() {
        return Some(numbers.join(" AND "));
    }
    let reference = Regex::new(r"(?i)\b(table|figure)\s+(\d+)[a-z]?").expect("reference pattern");
    if let Some(capture) = reference.captures(question) {
        return Some(format!("\"{} {}\"", &capture[1], &capture[2]));
    }
    let words = Regex::new(r"[\p{L}\p{N}]+").expect("word pattern");
    let terms: Vec<_> = words
        .find_iter(question)
        .map(|word| word.as_str().to_lowercase())
        .filter(|word| {
            word.len() > 2
                && !matches!(
                    word.as_str(),
                    "the"
                        | "and"
                        | "are"
                        | "was"
                        | "were"
                        | "does"
                        | "what"
                        | "which"
                        | "how"
                        | "this"
                        | "that"
                        | "these"
                        | "those"
                        | "with"
                        | "from"
                        | "for"
                        | "its"
                        | "can"
                        | "give"
                        | "paper"
                        | "papers"
                        | "selected"
                        | "explain"
                        | "their"
                        | "each"
                        | "both"
                        | "about"
                        | "into"
                        | "has"
                        | "have"
                        | "been"
                        | "would"
                        | "could"
                        | "should"
                )
        })
        .take(MAX_QUERY_TERMS)
        .map(|word| format!("\"{word}\""))
        .collect();
    (!terms.is_empty()).then(|| terms.join(" OR "))
}

/// Filter eligible chunks before ranking, so unselected hits cannot crowd them out.
pub(super) fn lexical_chunk_keys(
    connection: &Connection,
    question: &str,
    profile_id: &str,
    library_id: &str,
    allowed_items: &HashSet<String>,
    limit: usize,
) -> AppResult<Vec<u64>> {
    let Some(query) = passage_query(question) else {
        return Ok(Vec::new());
    };
    if allowed_items.is_empty() {
        return Ok(Vec::new());
    }
    let mut allowed: Vec<_> = allowed_items.iter().collect();
    allowed.sort();
    let mut statement = connection.prepare(
        "SELECT sc.vector_key FROM semantic_chunk_fts
         JOIN semantic_chunks sc ON sc.rowid=semantic_chunk_fts.rowid
         JOIN semantic_vector_entries sve ON sve.vector_key=sc.vector_key
         WHERE semantic_chunk_fts MATCH ?1 AND sc.embedding_profile_id=?2
           AND sc.library_id=?3 AND sc.status='active' AND sve.active=1
           AND sc.item_id IN (SELECT value FROM json_each(?4))
         ORDER BY bm25(semantic_chunk_fts), sc.id LIMIT ?5",
    )?;
    let rows = statement.query_map(
        params![
            query,
            profile_id,
            library_id,
            serde_json::to_string(&allowed)?,
            limit as i64
        ],
        |row| Ok(row.get::<_, i64>(0)? as u64),
    )?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_targets_and_does_not_accept_fts_operators() {
        assert_eq!(
            passage_query("Find Table 4, experiment 2"),
            Some("\"Table 4\"".into())
        );
        assert_eq!(
            passage_query("Which table reports 0.6281?"),
            Some("\"0.6281\"".into())
        );
        assert_eq!(passage_query("What does this paper do?"), None);
        assert_eq!(passage_query("\" OR * NOT :"), Some("\"not\"".into()));
    }

    #[test]
    fn backfills_and_tracks_text_changes_with_scope_and_status_filters() {
        let connection = Connection::open_in_memory().unwrap();
        connection.execute_batch(
            "CREATE TABLE semantic_chunks(id TEXT, text TEXT, vector_key INTEGER,
             item_id TEXT, embedding_profile_id TEXT, library_id TEXT, status TEXT);
             CREATE TABLE semantic_vector_entries(vector_key INTEGER, active INTEGER);
             INSERT INTO semantic_chunks VALUES ('a','Table 4. Value 0.6281',1,'a','p','l','active');
             INSERT INTO semantic_chunks VALUES ('b','Table 4. Value 0.6281',2,'b','p','l','active');
             INSERT INTO semantic_vector_entries VALUES (1,1),(2,1);"
        ).unwrap();
        connection
            .execute_batch(include_str!("../migrations/015_passage_search.sql"))
            .unwrap();
        let scope = HashSet::from(["b".to_owned()]);
        let query =
            || lexical_chunk_keys(&connection, "value 0.6281", "p", "l", &scope, 1).unwrap();
        assert_eq!(query(), vec![2]);
        connection
            .execute("UPDATE semantic_chunks SET status='stale' WHERE id='b'", [])
            .unwrap();
        assert!(query().is_empty());
        connection
            .execute(
                "UPDATE semantic_chunks SET status='active', text='different value' WHERE id='b'",
                [],
            )
            .unwrap();
        assert!(query().is_empty());
        connection
            .execute("UPDATE semantic_chunks SET text='0.6281' WHERE id='b'", [])
            .unwrap();
        assert_eq!(query(), vec![2]);
        connection
            .execute(
                "UPDATE semantic_vector_entries SET active=0 WHERE vector_key=2",
                [],
            )
            .unwrap();
        assert!(query().is_empty());
        connection
            .execute("DELETE FROM semantic_chunks WHERE id='b'", [])
            .unwrap();
        let count: i64 = connection.query_row(
            "SELECT COUNT(*) FROM semantic_chunk_fts WHERE semantic_chunk_fts MATCH '\"0.6281\"'",
            [], |row| row.get(0)).unwrap();
        assert_eq!(count, 1);
    }
}
