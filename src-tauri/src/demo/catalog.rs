//! Load the pinned public catalogue through the neutral source boundary.

use std::collections::{BTreeSet, HashSet};

use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::database::ResearchDatabase;
use crate::error::{AppError, AppResult};
use crate::knowledge::model::{
    ArtifactRole, DocumentArtifactRecord, KnowledgeItemRecord, KnowledgeKind, SourceDelta,
    SourceDiscoveryRequest, SourceGroupRecord, SourceProviderDescriptor,
};
use crate::knowledge::source::{ProviderFuture, SourceProvider};
use crate::knowledge::store::KnowledgeStore;

pub const SOURCE_ID: &str = "graphite-demo";
// Keep the source partition stable so existing PDFs and reading state survive.
pub const LIBRARY_ID: &str = "graphite-demo:seismology-ml-v1";
const INSTANCE_ID: &str = "source-instance:graphite-demo:seismology-ml-v1";
pub(super) const CATALOG_VERSION_KEY: &str = "real-demo-catalogue-version";

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub id: String,
    pub title: String,
    pub papers: Vec<Paper>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Paper {
    pub id: String,
    pub title: String,
    pub authors: Vec<String>,
    pub year: i32,
    pub journal: String,
    pub doi: String,
    pub source_url: String,
    pub pdf_url: String,
    pub sha256: String,
    pub pdf_bytes: u64,
    pub pdf_pages: u32,
    pub themes: Vec<String>,
    pub attribution: String,
    pub license: String,
    pub license_url: String,
}

pub fn manifest() -> AppResult<Manifest> {
    let manifest: Manifest = serde_json::from_str(include_str!("../../../demo/manifest.json"))?;
    let mut ids = HashSet::new();
    let mut dois = HashSet::new();
    let mut hashes = HashSet::new();
    if manifest.id != "seismology-ml-v2" || manifest.papers.len() != 60 {
        return Err(AppError::ApplicationData(
            "Invalid real-demo catalogue.".into(),
        ));
    }
    for paper in &manifest.papers {
        if paper.id.is_empty()
            || !paper
                .id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b".-".contains(&c))
            || paper.sha256.len() != 64
            || !paper.sha256.bytes().all(|c| c.is_ascii_hexdigit())
            || paper.pdf_bytes > 64 * 1024 * 1024
            || paper.pdf_bytes < 5
            || paper.pdf_pages == 0
            || !ids.insert(&paper.id)
            || !dois.insert(paper.doi.to_lowercase())
            || !hashes.insert(&paper.sha256)
            || !publisher_url(
                &reqwest::Url::parse(&paper.pdf_url)
                    .map_err(|_| AppError::ApplicationData("Invalid publisher URL.".into()))?,
            )
        {
            return Err(AppError::ApplicationData(
                "Invalid or duplicate real-demo record.".into(),
            ));
        }
    }
    Ok(manifest)
}

pub fn publisher_url(url: &reqwest::Url) -> bool {
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none()
        && matches!(
            url.host_str(),
            Some(
                "se.copernicus.org"
                    | "nhess.copernicus.org"
                    | "gmd.copernicus.org"
                    | "npg.copernicus.org"
                    | "gi.copernicus.org"
                    | "essd.copernicus.org"
                    | "esurf.copernicus.org"
                    | "seismica.library.mcgill.ca"
                    | "authors.library.caltech.edu"
                    | "link.springer.com"
                    | "flore.unifi.it"
                    | "raw.githubusercontent.com"
                    | "www.nature.com"
                    | "idp.nature.com"
                    | "www.frontiersin.org"
                    | "media.springernature.com"
                    | "static-content.springer-cdn.com"
            )
        )
}

const ROOT_COLLECTION: &str = "root";
const ROOT_COLLECTION_NAME: &str = "Machine learning in seismology";

fn collection_groups(catalog: &Manifest) -> Vec<SourceGroupRecord> {
    let mut groups = vec![SourceGroupRecord {
        external_id: ROOT_COLLECTION.into(),
        source_revision: 1,
        name: ROOT_COLLECTION_NAME.into(),
        parent_external_id: None,
    }];
    let themes: BTreeSet<_> = catalog
        .papers
        .iter()
        .flat_map(|paper| paper.themes.iter())
        .collect();
    groups.extend(themes.into_iter().map(|theme| SourceGroupRecord {
        external_id: format!("theme:{theme}"),
        source_revision: 1,
        name: theme.clone(),
        parent_external_id: Some(ROOT_COLLECTION.into()),
    }));
    groups
}

pub struct DemoSourceProvider;

impl DemoSourceProvider {
    pub(super) fn delta(&self) -> AppResult<SourceDelta> {
        let catalog = manifest()?;
        Ok(SourceDelta {
            source_instance_id: INSTANCE_ID.into(),
            descriptor: self.descriptor(),
            checkpoint_before: 0,
            checkpoint_after: 1,
            knowledge_items: catalog.papers.iter().map(|paper| KnowledgeItemRecord {
                external_id: paper.id.clone(),
                external_kind: "journalArticle".into(),
                knowledge_kind: KnowledgeKind::Paper,
                source_revision: 1,
                title: paper.title.clone(),
                abstract_text: None,
                publication_year: Some(paper.year),
                source_metadata: serde_json::to_value(paper).expect("serializable catalogue"),
            }).collect(),
            artifacts: catalog.papers.iter().map(|paper| DocumentArtifactRecord {
                external_id: format!("pdf:{}", paper.id),
                parent_external_item_id: Some(paper.id.clone()),
                source_revision: 1,
                artifact_role: ArtifactRole::Attachment,
                media_type: "application/pdf".into(),
                title: format!("{} (publisher PDF)", paper.title),
                filename: Some(format!("{}.pdf", paper.id)),
                source_metadata: serde_json::json!({"linkMode":"internal-demo", "sha256":paper.sha256}),
            }).collect(),
            source_groups: collection_groups(&catalog), source_objects: Vec::new(),
            item_tombstones: Vec::new(), artifact_tombstones: Vec::new(),
            group_tombstones: Vec::new(), snapshot_complete: true,
            warnings: Vec::new(),
        })
    }
}

impl SourceProvider for DemoSourceProvider {
    fn descriptor(&self) -> SourceProviderDescriptor {
        SourceProviderDescriptor {
            provider_kind: "real-demo".into(),
            provider_version: "1".into(),
            display_name: "Real seismology papers".into(),
            read_only: true,
        }
    }

    fn discover<'a>(&'a self, _request: SourceDiscoveryRequest) -> ProviderFuture<'a, SourceDelta> {
        Box::pin(async move { self.delta() })
    }
}

pub fn register(database: &ResearchDatabase) -> AppResult<()> {
    let catalog = manifest()?;
    let library_name = format!(
        "Machine learning in seismology: {} real papers",
        catalog.papers.len()
    );
    let connection = database.connect()?;
    connection.execute(
        "INSERT OR IGNORE INTO sources
        (id,server_id,api_version,compatibility,first_seen_at,last_seen_at,connection_status)
        VALUES (?1,?1,'1','internal',CURRENT_TIMESTAMP,CURRENT_TIMESTAMP,'online')",
        [SOURCE_ID],
    )?;
    connection.execute(
        "INSERT INTO libraries
        (id,source_id,kind,zotero_library_id,name,successful_version,available)
        VALUES (?1,?2,'user','real-demo',?3,0,1)
        ON CONFLICT(id) DO UPDATE SET name=excluded.name",
        params![LIBRARY_ID, SOURCE_ID, library_name],
    )?;
    connection.execute("INSERT OR IGNORE INTO source_instances
        (id,provider_kind,display_name,external_identity,state,legacy_source_id,legacy_library_id,created_at,updated_at)
        VALUES (?1,'real-demo',?2,?3,'active',?3,?4,CURRENT_TIMESTAMP,CURRENT_TIMESTAMP)",
        params![INSTANCE_ID,catalog.title,SOURCE_ID,LIBRARY_ID])?;
    let mut delta = DemoSourceProvider.delta()?;
    let current_items: HashSet<_> = delta
        .knowledge_items
        .iter()
        .map(|item| item.external_id.as_str())
        .collect();
    let current_artifacts: HashSet<_> = delta
        .artifacts
        .iter()
        .map(|item| item.external_id.as_str())
        .collect();
    for (query,current,removed) in [
        ("SELECT external_item_id FROM knowledge_item_source_bindings WHERE source_instance_id=?1 AND state='active'", &current_items, &mut delta.item_tombstones),
        ("SELECT external_artifact_id FROM document_artifact_source_bindings WHERE source_instance_id=?1 AND state='active'", &current_artifacts, &mut delta.artifact_tombstones),
    ] {
        let mut statement=connection.prepare(query)?;
        for id in statement.query_map([INSTANCE_ID],|row|row.get::<_,String>(0))? {
            let id=id?;
            if !current.contains(id.as_str()) {removed.push(id);}
        }
    }
    drop(connection);
    KnowledgeStore::new(database.clone()).apply_source_delta(SOURCE_ID, LIBRARY_ID, &delta)?;
    let mut connection = database.connect()?;
    let tx = connection.transaction()?;
    let group_keys: HashSet<_> = collection_groups(&catalog)
        .into_iter()
        .map(|group| group.external_id)
        .collect();
    let retired_groups = {
        let mut statement =
            tx.prepare("SELECT id,zotero_key FROM collections WHERE library_id=?1")?;
        let rows = statement.query_map([LIBRARY_ID], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        rows.collect::<Result<Vec<_>, _>>()?
    };
    for (id, key) in retired_groups {
        if !group_keys.contains(&key) {
            tx.execute("DELETE FROM item_collections WHERE collection_id=?1", [&id])?;
            tx.execute("DELETE FROM collections WHERE id=?1", [&id])?;
        }
    }
    for paper in &catalog.papers {
        let item_id: String = tx.query_row(
            "SELECT id FROM research_items WHERE library_id=?1 AND zotero_key=?2",
            params![LIBRARY_ID, paper.id],
            |row| row.get(0),
        )?;
        tx.execute(
            "UPDATE research_items SET doi=?2,url=?3,venue=?4,date_text=?5 WHERE id=?1",
            params![
                item_id,
                paper.doi,
                paper.source_url,
                paper.journal,
                paper.year.to_string()
            ],
        )?;
        // Reconcile only memberships owned by this demo source.
        tx.execute(
            "DELETE FROM item_collections WHERE item_id=?1 AND collection_id IN
            (SELECT id FROM collections WHERE library_id=?2)",
            params![item_id, LIBRARY_ID],
        )?;
        for key in std::iter::once(ROOT_COLLECTION.to_owned())
            .chain(paper.themes.iter().map(|theme| format!("theme:{theme}")))
        {
            let collection_id =
                crate::database::stable_id("collection", &[SOURCE_ID, LIBRARY_ID, &key]);
            tx.execute(
                "INSERT OR IGNORE INTO item_collections(item_id,collection_id,collection_key)
                VALUES (?1,?2,?3)",
                params![item_id, collection_id, key],
            )?;
        }
        tx.execute("DELETE FROM creators WHERE item_id=?1", [&item_id])?;
        for (position, author) in paper.authors.iter().enumerate() {
            let (family, given) = author.split_once(',').unwrap_or((author, ""));
            let name = format!("{} {}", given.trim(), family.trim())
                .trim()
                .to_owned();
            tx.execute("INSERT INTO creators (item_id,position,creator_type,first_name,last_name,display_name,normalized_display_name)
                VALUES (?1,?2,'author',?3,?4,?5,?6)", params![item_id,position as i64,given.trim(),family.trim(),name,name.to_lowercase()])?;
        }
        for theme in &paper.themes {
            tx.execute(
                "INSERT OR IGNORE INTO item_tags (item_id,tag,normalized_tag) VALUES (?1,?2,?3)",
                params![item_id, theme, theme.to_lowercase()],
            )?;
        }
        crate::database::refresh_search_content(&tx, &item_id)?;
    }
    crate::database::backfill_match_keys(&tx)?;
    let previous_catalog: Option<String> = tx
        .query_row(
            "SELECT value FROM app_meta WHERE key=?1",
            [CATALOG_VERSION_KEY],
            |row| row.get(0),
        )
        .optional()?;
    if previous_catalog.as_deref() != Some(catalog.id.as_str()) {
        // Existing external references may now have local targets.
        tx.execute(
            "INSERT OR IGNORE INTO citation_resolution_queue
                (reference_id,reason,queued_at)
             SELECT r.id,'demo-catalogue-changed',CURRENT_TIMESTAMP
             FROM bibliographic_references r
             JOIN reference_extractions e ON e.id=r.extraction_id
             WHERE r.library_id=?1 AND r.is_current=1 AND e.is_current=1",
            [LIBRARY_ID],
        )?;
        tx.execute(
            "INSERT INTO app_meta(key,value) VALUES (?1,?2)
             ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![CATALOG_VERSION_KEY, catalog.id],
        )?;
    }
    tx.commit()?;
    Ok(())
}

pub fn activate(database: &ResearchDatabase, demo: bool) -> AppResult<()> {
    let mut connection = database.connect()?;
    let tx = connection.transaction()?;
    if demo {
        let active: Option<String> = tx
            .query_row(
                "SELECT value FROM app_meta WHERE key='active_source_id'",
                [],
                |r| r.get(0),
            )
            .optional()?;
        if active.as_deref().is_some_and(|id| id != SOURCE_ID) {
            for (active, saved) in [
                ("active_source_id", "demo_previous_source"),
                ("active_library_id", "demo_previous_library"),
            ] {
                tx.execute(
                    "INSERT INTO app_meta(key,value) SELECT ?1,value FROM app_meta WHERE key=?2
                    ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                    params![saved, active],
                )?;
            }
        }
        for (key, value) in [
            ("active_source_id", SOURCE_ID),
            ("active_library_id", LIBRARY_ID),
        ] {
            tx.execute("INSERT INTO app_meta(key,value) VALUES (?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",params![key,value])?;
        }
    } else {
        let active: Option<String> = tx
            .query_row(
                "SELECT value FROM app_meta WHERE key='active_source_id'",
                [],
                |r| r.get(0),
            )
            .optional()?;
        if active.as_deref() == Some(SOURCE_ID) {
            tx.execute(
                "DELETE FROM app_meta WHERE key IN ('active_source_id','active_library_id')",
                [],
            )?;
            tx.execute("INSERT INTO app_meta(key,value) SELECT 'active_source_id',value FROM app_meta
                WHERE key='demo_previous_source' AND value IN (SELECT id FROM sources WHERE id<>?1)", [SOURCE_ID])?;
            tx.execute("INSERT INTO app_meta(key,value) SELECT 'active_library_id',value FROM app_meta
                WHERE key='demo_previous_library' AND value IN (SELECT id FROM libraries WHERE source_id=(SELECT value FROM app_meta WHERE key='active_source_id'))",[])?;
        }
    }
    tx.commit()?;
    Ok(())
}

pub fn is_active(database: &ResearchDatabase) -> AppResult<bool> {
    Ok(database
        .active_library_context()?
        .is_some_and(|(source, _, _)| source == SOURCE_ID))
}
