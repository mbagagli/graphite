//! Verify that catalogue upgrades reconnect retained bibliography evidence.

use std::sync::Arc;

use rusqlite::params;

use crate::citation::CitationCoordinator;
use crate::database::ResearchDatabase;
use crate::model::{FullTextRecord, FullTextSyncDelta};

use super::catalog::{self, CATALOG_VERSION_KEY, LIBRARY_ID, SOURCE_ID};

// Four real references from the original NHESS ground-motion paper.
const REFERENCES: &str = "References\n\
[1] Jozinovic, D., Lomax, A., Stajduhar, I., and Michelini, A. (2021). \
Rapid prediction of earthquake ground shaking intensity using raw \
waveform data and a convolutional neural network. Geophys. J. Int., \
222, 1379-1389. https://doi.org/10.1093/GJI/GGAA233\n\
[2] Kim, S., Hwang, Y., Seo, H., and Kim, B. (2020). Ground motion \
amplification models for Japan using machine learning techniques. \
Soil Dyn. Earthq. Eng., 132, 106095. \
https://doi.org/10.1016/j.soildyn.2020.106095\n\
[3] Kubo, H., Kunugi, T., Suzuki, W., Suzuki, S., and Aoi, S. (2020). \
Hybrid predictor for ground-motion intensity with machine learning and \
conventional ground motion prediction equation. Sci. Rep., 10, 11871. \
https://doi.org/10.1038/s41598-020-68630-x\n\
[4] Luo, Y., Fan, X., Huang, R., Wang, Y., Yunus, A. P., and Havenith, \
H. B. (2020). Topographic and near-surface stratigraphic amplification \
of the seismic response of a mountain slope revealed by field \
monitoring and numerical simulations. Eng. Geol., 271, 105607. \
https://doi.org/10.1016/j.enggeo.2020.105607";

#[tokio::test]
async fn catalogue_upgrade_resolves_retained_references_once() {
    for previous_version in [None, Some("seismology-ml-v1")] {
        let directory = tempfile::tempdir().expect("isolated upgrade fixture");
        let database = ResearchDatabase::initialize(directory.path().join("research.sqlite3"))
            .expect("database");
        let manifest = catalog::manifest().expect("current real catalogue");
        catalog::register(&database).expect("seed real metadata");
        catalog::activate(&database, true).expect("activate demo partition");
        let mut connection = database.connect().expect("connection");
        let tx = connection.transaction().expect("v1 fixture transaction");
        // Retain the actual original 30, with no future catalogue targets.
        for paper in manifest.papers.iter().skip(30) {
            tx.execute(
                "DELETE FROM research_items WHERE library_id=?1
                 AND zotero_key=?2",
                params![LIBRARY_ID, paper.id],
            )
            .expect("remove later catalogue addition");
        }
        tx.execute("DELETE FROM app_meta WHERE key=?1", [CATALOG_VERSION_KEY])
            .expect("model an unmarked v1 cache");
        if let Some(version) = previous_version {
            tx.execute(
                "INSERT INTO app_meta(key,value) VALUES (?1,?2)",
                params![CATALOG_VERSION_KEY, version],
            )
            .expect("model an explicitly versioned v1 cache");
        }
        tx.commit().expect("commit v1 fixture");
        assert_eq!(
            database
                .load_workspace()
                .expect("v1 workspace")
                .snapshot
                .total_item_count,
            30
        );
        let (item_id, document_id): (String, String) = connection
            .query_row(
                "SELECT i.id,d.id FROM research_items i
                 JOIN documents d ON d.research_item_id=i.id
                 WHERE i.library_id=?1 AND i.zotero_key='nhess-22-947-2022'",
                [LIBRARY_ID],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("original real paper and PDF identity");
        connection
            .execute(
                "INSERT INTO reader_states(document_id,page_number,updated_at)
             VALUES (?1,3,CURRENT_TIMESTAMP)",
                [&document_id],
            )
            .expect("retained reading position");
        database
            .apply_fulltext_delta(
                SOURCE_ID,
                LIBRARY_ID,
                &FullTextSyncDelta {
                    successful_version: 1,
                    records: vec![FullTextRecord {
                        attachment_key: "pdf:nhess-22-947-2022".into(),
                        version: 1,
                        content: REFERENCES.into(),
                        indexed_pages: Some(1),
                        total_pages: Some(1),
                        indexed_chars: Some(REFERENCES.len() as u64),
                        total_chars: Some(REFERENCES.len() as u64),
                    }],
                    removed_attachment_keys: Vec::new(),
                    warning_count: 0,
                    status: "ready".into(),
                    last_error: None,
                },
            )
            .expect("retained bibliography excerpt");
        let citations =
            CitationCoordinator::new(database.clone(), Arc::new(tokio::sync::Mutex::new(())));
        let before = citations
            .index(None, Some(60))
            .await
            .expect("index v1 bibliography");
        assert_eq!(before.citations.references_extracted, 4);
        assert_eq!(before.citations.citation_edges, 0);
        let reference_id: String = connection
            .query_row(
                "SELECT r.id FROM bibliographic_references r
             JOIN reference_resolutions rr ON rr.reference_id=r.id
             WHERE r.source_item_id=?1
             AND r.normalized_doi='10.1038/s41598-020-68630-x'
             AND rr.status='external'",
                [&item_id],
                |row| row.get(0),
            )
            .expect("future demo paper is initially external");

        catalog::register(&database).expect("upgrade to v2 catalogue");
        let queued: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM citation_resolution_queue",
                [],
                |row| row.get(0),
            )
            .expect("upgrade queue");
        assert_eq!(queued, 4);
        let upgraded = citations
            .index(None, Some(60))
            .await
            .expect("the demo preparation index call resolves the upgrade");
        assert_eq!(upgraded.snapshot.total_item_count, 60);
        assert_eq!(upgraded.citations.queued_resolutions, 0);
        assert_eq!(upgraded.citations.references_extracted, 4);
        let local_target: String = connection
            .query_row(
                "SELECT i.zotero_key FROM reference_resolutions rr
             JOIN research_items i ON i.id=rr.matched_item_id
             WHERE rr.reference_id=?1 AND rr.status='exact-identifier'",
                [&reference_id],
                |row| row.get(0),
            )
            .expect("retained reference now resolves locally");
        assert_eq!(local_target, "s41598-020-68630-x");
        let edge_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM citation_edges ce
             JOIN research_items target ON target.id=ce.target_item_id
             WHERE ce.source_item_id=?1
             AND target.zotero_key='s41598-020-68630-x'",
                [&item_id],
                |row| row.get(0),
            )
            .expect("retained reference creates its genuine local edge");
        assert_eq!(edge_count, 1);
        assert_eq!(
            database
                .reader_state(&document_id)
                .expect("reading state")
                .expect("retained reading state")
                .page_number,
            3
        );
        assert_eq!(
            database
                .document_text(&document_id)
                .expect("text lookup")
                .expect("retained text")
                .content,
            REFERENCES
        );
        let source_version: i64 = connection
            .query_row(
                "SELECT source_version FROM research_items WHERE id=?1",
                [&item_id],
                |row| row.get(0),
            )
            .expect("retained source version");
        assert_eq!(source_version, 1);
        catalog::register(&database).expect("repeat v2 activation");
        let queued_again: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM citation_resolution_queue",
                [],
                |row| row.get(0),
            )
            .expect("drained queue stays empty");
        assert_eq!(queued_again, 0);
    }
}
