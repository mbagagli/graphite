use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use graphite_pdf_benchmark_common::{
    content_hash, read_json, validate_anchor_set, validate_manifest, write_json_atomic, AnchorSet,
    CorpusManifest, ZoteroTextSidecar,
};
use rusqlite::{Connection, OpenFlags};

fn main() {
    if let Err(error) = run() {
        eprintln!("zotero-cache-capture: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let arguments = env::args().skip(1).collect::<Vec<_>>();
    if arguments.len() != 4 {
        return Err(usage());
    }
    let manifest_path = PathBuf::from(&arguments[0]);
    let document_id = &arguments[1];
    let database_path = PathBuf::from(&arguments[2]);
    let attachment_key = &arguments[3];
    let manifest = validate_representative_manifest(&manifest_path)?;
    if !manifest
        .documents
        .iter()
        .any(|document| &document.id == document_id)
    {
        return Err(format!("manifest has no document {document_id}"));
    }
    if attachment_key.is_empty() {
        return Err("Zotero attachment identifier must be non-empty".to_owned());
    }
    let sidecar = capture_cached_zotero_sidecar(document_id, &database_path, attachment_key)?;
    let corpus_root = manifest_path
        .parent()
        .ok_or_else(|| "manifest has no parent directory".to_owned())?;
    let output = corpus_root
        .join("local")
        .join(format!("{document_id}.zotero-fulltext.json"));
    write_json_atomic(&output, &sidecar)?;
    let sidecar_hash = content_hash(
        &fs::read(&output).map_err(|error| format!("cannot verify captured sidecar: {error}"))?,
    );
    println!(
        "captured read-only production-cached Zotero sidecar local/{document_id}.zotero-fulltext.json; content_sha256={}; sidecar_sha256={sidecar_hash}",
        sidecar.content_sha256.as_deref().unwrap_or("unavailable")
    );
    Ok(())
}

fn usage() -> String {
    [
        "usage:",
        "  graphite-pdf-benchmark-zotero-cache-capture <representative-manifest> <document-id> <graphite-database> <attachment-key>",
        "",
        "Reads an existing Graphite Zotero full-text cache in SQLite read-only mode and writes",
        "a byte-faithful, neutral-ID sidecar under the ignored benchmark corpus/local directory.",
        "Use the main runner capture-zotero command when the live Zotero local API is available.",
    ]
    .join("\n")
}

fn validate_representative_manifest(path: &Path) -> Result<CorpusManifest, String> {
    let manifest: CorpusManifest = read_json(path)?;
    let root = path
        .parent()
        .ok_or_else(|| "manifest has no parent directory".to_owned())?;
    validate_manifest(&manifest, root).map_err(|errors| errors.join("\n"))?;
    if manifest.corpus_id != "graphite-scientific-pdf-representative-real-v1"
        || manifest.documents.len() != 20
    {
        return Err(
            "representative manifest must contain the frozen 20-document subset".to_owned(),
        );
    }
    for (index, document) in manifest.documents.iter().enumerate() {
        let expected_id = format!("paper{:02}", index + 11);
        if document.id != expected_id
            || document.file != format!("local/{expected_id}.pdf")
            || document.anchors != format!("anchors/{expected_id}.json")
        {
            return Err(format!(
                "representative document {} must use neutral identity {expected_id}",
                document.id
            ));
        }
        let anchors: AnchorSet = read_json(&root.join(&document.anchors))?;
        validate_anchor_set(&anchors, &document.id).map_err(|errors| errors.join("\n"))?;
    }
    Ok(manifest)
}

fn capture_cached_zotero_sidecar(
    document_id: &str,
    database_path: &Path,
    attachment_key: &str,
) -> Result<ZoteroTextSidecar, String> {
    let connection = Connection::open_with_flags(
        database_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|error| format!("cannot open Graphite database read-only: {error}"))?;
    let mut statement = connection
        .prepare(
            "SELECT source_version, content, indexed_pages, total_pages, \
                    indexed_chars, total_chars \
             FROM document_texts \
             WHERE zotero_attachment_key = ?1 AND provider = 'zotero-indexed' \
             ORDER BY source_version DESC, fetched_at DESC, id DESC \
             LIMIT 2",
        )
        .map_err(|error| format!("cannot prepare cached Zotero full-text query: {error}"))?;
    let rows = statement
        .query_map([attachment_key], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<u32>>(2)?,
                row.get::<_, Option<u32>>(3)?,
                row.get::<_, Option<i64>>(4)?,
                row.get::<_, Option<i64>>(5)?,
            ))
        })
        .map_err(|error| format!("cannot query cached Zotero full text: {error}"))?;
    let matches = rows
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("cannot read cached Zotero full text: {error}"))?;
    let selected = matches.first().cloned().ok_or_else(|| {
        "the read-only Graphite cache has no matching Zotero full text".to_owned()
    })?;
    if matches
        .get(1)
        .is_some_and(|other| other.0 == selected.0 && other.1 != selected.1)
    {
        return Err(
            "the read-only Graphite cache has conflicting latest Zotero full-text rows".to_owned(),
        );
    }
    let indexed_characters = nonnegative_count(selected.4, "indexed character")?;
    let total_characters = nonnegative_count(selected.5, "total character")?;
    Ok(ZoteroTextSidecar {
        schema_version: 2,
        document_id: Some(document_id.to_owned()),
        provider: "zotero-production-cache-fulltext-v1".to_owned(),
        source_version: selected.0,
        content_sha256: Some(content_hash(selected.1.as_bytes())),
        content: selected.1,
        indexed_pages: selected.2,
        total_pages: selected.3,
        indexed_characters,
        total_characters,
    })
}

fn nonnegative_count(value: Option<i64>, label: &str) -> Result<Option<u64>, String> {
    value
        .map(u64::try_from)
        .transpose()
        .map_err(|_| format!("cached Zotero {label} count is negative"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_is_byte_faithful_and_read_only() {
        let directory = tempfile::tempdir().expect("temp directory");
        let database = directory.path().join("graphite.sqlite3");
        let content = "  Unicode mu\n\n3.2e-4\r\n";
        create_cache(&database, &[("text-1", 42, content)]);
        let before = content_hash(&fs::read(&database).expect("database bytes"));

        let sidecar = capture_cached_zotero_sidecar("paper11", &database, "ATTACHMENT")
            .expect("cached sidecar");

        assert_eq!(sidecar.content, content);
        assert_eq!(sidecar.document_id.as_deref(), Some("paper11"));
        assert_eq!(sidecar.source_version, 42);
        assert_eq!(
            sidecar.content_sha256,
            Some(content_hash(content.as_bytes()))
        );
        assert_eq!(
            before,
            content_hash(&fs::read(&database).expect("database bytes"))
        );
    }

    #[test]
    fn capture_rejects_conflicting_latest_rows() {
        let directory = tempfile::tempdir().expect("temp directory");
        let database = directory.path().join("graphite.sqlite3");
        create_cache(&database, &[("a", 42, "first"), ("b", 42, "second")]);

        let error = capture_cached_zotero_sidecar("paper11", &database, "ATTACHMENT")
            .expect_err("conflicting rows must fail");

        assert!(error.contains("conflicting latest"));
    }

    fn create_cache(database: &Path, rows: &[(&str, i64, &str)]) {
        let mut connection = Connection::open(database).expect("database");
        connection
            .execute_batch(
                "CREATE TABLE document_texts (\
                     id TEXT PRIMARY KEY, zotero_attachment_key TEXT NOT NULL, \
                     provider TEXT NOT NULL, source_version INTEGER NOT NULL, \
                     content TEXT NOT NULL, indexed_pages INTEGER, total_pages INTEGER, \
                     indexed_chars INTEGER, total_chars INTEGER, fetched_at TEXT NOT NULL\
                 );",
            )
            .expect("schema");
        let transaction = connection.transaction().expect("transaction");
        for (id, source_version, content) in rows {
            transaction
                .execute(
                    "INSERT INTO document_texts VALUES \
                     (?1, 'ATTACHMENT', 'zotero-indexed', ?2, ?3, 1, 1, 24, 24, \
                      '2026-08-28T00:00:00Z')",
                    (id, source_version, content),
                )
                .expect("cache row");
        }
        transaction.commit().expect("commit");
    }
}
