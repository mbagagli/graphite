//! Builds workspace, graph, topic, and analytics read projections.

use std::collections::{BTreeMap, HashMap, HashSet};

use rusqlite::{params, Connection};
use sha2::{Digest, Sha256};

use crate::error::AppResult;
use crate::model::{
    AnalyticsSnapshot, CitationAnalytics, CitationPaperMetric, CorpusDiagnostics,
    ExternalWorkMetric, GraphEdge, GraphNode, GraphProjectionRequest, GraphProjectionResult,
    LibrarySnapshot, ResearchGraph, ResearchItem, Topic, TopicMetric, WorkspacePayload, YearMetric,
};

use super::{
    load_collections, load_corpus_diagnostics, load_diagnostics, load_libraries,
    load_selected_items, meta_value, normalize_facet, stable_id, ResearchDatabase,
};

const MAX_GRAPH_LIMIT: u32 = 1_200;

impl ResearchDatabase {
    pub fn load_workspace(&self) -> AppResult<WorkspacePayload> {
        let connection = self.connect()?;
        let source = load_diagnostics(&connection)?;
        let active_source_id = meta_value(&connection, "active_source_id")?;
        let active_library_id = meta_value(&connection, "active_library_id")?;
        let libraries = match active_source_id.as_deref() {
            Some(id) => load_libraries(&connection, id)?,
            None => Vec::new(),
        };
        let collections = match active_library_id.as_deref() {
            Some(id) => load_collections(&connection, id)?,
            None => Vec::new(),
        };
        let (items, total_item_count) = match active_library_id.as_deref() {
            Some(id) => {
                let total = prepare_selected_item_ids(&connection, id, None, &[], None, 0)?;
                (load_selected_items(&connection)?, total)
            }
            None => {
                reset_selected_item_ids(&connection)?;
                (Vec::new(), 0)
            }
        };
        let (topics, item_topics) = derive_topics(&connection, &items)?;
        let mut graph = project_graph(&connection, &items, &topics, &item_topics)?;
        graph.total_node_count = total_item_count;
        graph.has_more = items.len() < total_item_count as usize;
        let corpus = load_corpus_diagnostics(&connection, active_library_id.as_deref())?;
        let citations =
            crate::citation::load_citation_diagnostics(&connection, active_library_id.as_deref())?;
        let citation_analytics =
            load_citation_analytics(&connection, active_library_id.as_deref(), &citations)?;
        let analytics = compute_analytics(
            &items,
            &topics,
            &item_topics,
            &graph,
            &corpus,
            citation_analytics,
        );
        let items = apply_item_topics(items, &item_topics);
        let snapshot = LibrarySnapshot {
            total_item_count,
            loaded_item_count: items.len() as u64,
            items,
            collections,
            topics,
            lenses: match active_library_id.as_deref() {
                Some(id) => super::lenses::load_lenses(&connection, id)?,
                None => Vec::new(),
            },
            libraries,
            active_library_id: active_library_id.clone(),
            provenance: if source.mode == "demo" {
                "real-demo"
            } else {
                "zotero-local-cache"
            }
            .to_owned(),
        };
        Ok(WorkspacePayload {
            snapshot,
            graph,
            analytics,
            source,
            corpus,
            extraction: crate::extraction::diagnostics::load_extraction_diagnostics(self)?,
            citations,
            semantic: crate::semantic::load_semantic_diagnostics(
                &connection,
                active_library_id.as_deref(),
                None,
            )?,
        })
    }

    pub fn graph_projection(
        &self,
        request: &GraphProjectionRequest,
    ) -> AppResult<GraphProjectionResult> {
        let connection = self.connect()?;
        let Some(library_id) = meta_value(&connection, "active_library_id")? else {
            return Ok(GraphProjectionResult {
                items: Vec::new(),
                topics: Vec::new(),
                graph: ResearchGraph {
                    nodes: Vec::new(),
                    edges: Vec::new(),
                    available_relations: Vec::new(),
                    total_node_count: 0,
                    has_more: false,
                },
            });
        };
        let limit = request.limit.map(|limit| limit.clamp(1, MAX_GRAPH_LIMIT));
        let offset = request.offset.unwrap_or(0);
        let total = prepare_selected_item_ids(
            &connection,
            &library_id,
            request.collection_id.as_deref(),
            &request.seed_item_ids,
            limit,
            offset,
        )?;
        let items = load_selected_items(&connection)?;
        let (topics, item_topics) = derive_topics(&connection, &items)?;
        let mut graph = project_graph(&connection, &items, &topics, &item_topics)?;
        graph.total_node_count = total;
        graph.has_more = (offset as u64 + items.len() as u64) < total;
        Ok(GraphProjectionResult {
            items: apply_item_topics(items, &item_topics),
            topics,
            graph,
        })
    }

    pub fn projection_for_item_ids(&self, item_ids: &[String]) -> AppResult<GraphProjectionResult> {
        let connection = self.connect()?;
        reset_selected_item_ids(&connection)?;
        for item_id in item_ids.iter().take(MAX_GRAPH_LIMIT as usize) {
            connection.execute(
                "INSERT OR IGNORE INTO selected_item_ids(id)
                 SELECT id FROM research_items
                 WHERE id = ?1 AND is_trashed = 0",
                [item_id],
            )?;
        }
        let items = load_selected_items(&connection)?;
        let (topics, item_topics) = derive_topics(&connection, &items)?;
        let mut graph = project_graph(&connection, &items, &topics, &item_topics)?;
        graph.total_node_count = items.len() as u64;
        Ok(GraphProjectionResult {
            items: apply_item_topics(items, &item_topics),
            topics,
            graph,
        })
    }
}

fn prepare_selected_item_ids(
    connection: &Connection,
    library_id: &str,
    collection_id: Option<&str>,
    seed_item_ids: &[String],
    limit: Option<u32>,
    offset: u32,
) -> AppResult<u64> {
    reset_selected_item_ids(connection)?;
    let total = connection.query_row(
        "SELECT COUNT(*)
         FROM research_items i
         WHERE i.library_id = ?1 AND i.is_trashed = 0
           AND (?2 IS NULL OR EXISTS (
                SELECT 1 FROM item_collections ic
                WHERE ic.item_id = i.id AND ic.collection_id = ?2
           ))",
        params![library_id, collection_id],
        |row| row.get::<_, i64>(0),
    )? as u64;
    for seed_id in seed_item_ids.iter().take(MAX_GRAPH_LIMIT as usize) {
        connection.execute(
            "INSERT OR IGNORE INTO selected_item_ids(id)
             SELECT id FROM research_items
             WHERE id = ?1 AND library_id = ?2 AND is_trashed = 0",
            params![seed_id, library_id],
        )?;
    }
    let seeded_count =
        connection.query_row("SELECT COUNT(*) FROM selected_item_ids", [], |row| {
            row.get::<_, u32>(0)
        })?;
    // SQLite's negative LIMIT selects the complete active library or collection.
    let remaining = limit.map_or(-1, |limit| i64::from(limit.saturating_sub(seeded_count)));
    connection.execute(
        "INSERT OR IGNORE INTO selected_item_ids(id)
         SELECT i.id
         FROM research_items i
         WHERE i.library_id = ?1 AND i.is_trashed = 0
           AND (?2 IS NULL OR EXISTS (
                SELECT 1 FROM item_collections ic
                WHERE ic.item_id = i.id AND ic.collection_id = ?2
           ))
         ORDER BY i.title COLLATE NOCASE, i.id
         LIMIT ?3 OFFSET ?4",
        params![library_id, collection_id, remaining, offset],
    )?;
    Ok(total)
}

fn reset_selected_item_ids(connection: &Connection) -> AppResult<()> {
    connection.execute_batch(
        "CREATE TEMP TABLE IF NOT EXISTS selected_item_ids (
            id TEXT PRIMARY KEY
         ) WITHOUT ROWID;
         DELETE FROM selected_item_ids;",
    )?;
    Ok(())
}

fn derive_topics(
    connection: &Connection,
    items: &[ResearchItem],
) -> AppResult<(Vec<Topic>, HashMap<String, String>)> {
    let library: Option<String> = meta_value(connection, "active_library_id")?;
    let (saved, assignments) = match library {
        Some(library) => crate::semantic::topics::load(connection, &library)?,
        None => (vec![crate::semantic::topics::unassigned()], HashMap::new()),
    };
    let topics = saved;
    let selected = items
        .iter()
        .map(|item| {
            (
                item.id.clone(),
                assignments
                    .get(&item.id)
                    .cloned()
                    .unwrap_or_else(|| "uncategorized".into()),
            )
        })
        .collect();
    Ok((topics, selected))
}

fn apply_item_topics(
    mut items: Vec<ResearchItem>,
    item_topics: &HashMap<String, String>,
) -> Vec<ResearchItem> {
    for item in &mut items {
        item.topics = vec![item_topics
            .get(&item.id)
            .cloned()
            .unwrap_or_else(|| "uncategorized".to_owned())];
    }
    items
}

fn project_graph(
    connection: &Connection,
    items: &[ResearchItem],
    topics: &[Topic],
    item_topics: &HashMap<String, String>,
) -> AppResult<ResearchGraph> {
    let item_ids: HashSet<_> = items.iter().map(|item| item.id.clone()).collect();
    let topic_indexes: HashMap<_, _> = topics
        .iter()
        .enumerate()
        .map(|(index, topic)| (topic.id.clone(), index))
        .collect();
    let mut topic_counts: HashMap<String, usize> = HashMap::new();
    for topic_id in item_topics.values() {
        *topic_counts.entry(topic_id.clone()).or_default() += 1;
    }
    let mut nodes = Vec::with_capacity(items.len());
    for item in items {
        let topic_id = item_topics
            .get(&item.id)
            .cloned()
            .unwrap_or_else(|| "uncategorized".to_owned());
        let topic_index = *topic_indexes.get(&topic_id).unwrap_or(&0) as f64;
        let topic_angle = topic_index / topics.len().max(1) as f64 * std::f64::consts::TAU;
        let (jitter_x, jitter_y) = stable_jitter(&item.id);
        let cluster_span =
            7.0 + (topic_counts.get(&topic_id).copied().unwrap_or(1).min(600) as f64).sqrt() * 0.8;
        nodes.push(GraphNode {
            id: item.id.clone(),
            paper_id: Some(item.id.clone()),
            entity_kind: "research-item".to_owned(),
            label: item.title.clone(),
            detail: item.year.map(|year| year.to_string()),
            topic_id,
            x: topic_angle.cos() * 12.0 + jitter_x * cluster_span,
            y: topic_angle.sin() * 8.0 + jitter_y * cluster_span,
            importance: 0.35
                + (item.authors.len().min(6) as f64 * 0.035)
                + (item.tags.len().min(8) as f64 * 0.025)
                + (item.attachments.len().min(3) as f64 * 0.04),
        });
    }

    let mut edges = Vec::new();
    let mut seen = HashSet::new();
    let mut degrees = HashMap::new();
    let mut citation_statement = connection.prepare(
        "SELECT ce.id, ce.source_item_id, ce.target_item_id,
                ce.evidence_count, ce.strongest_category
         FROM citation_edges ce
         JOIN selected_item_ids source ON source.id = ce.source_item_id
         JOIN selected_item_ids target ON target.id = ce.target_item_id
         ORDER BY ce.evidence_count DESC, ce.id
         LIMIT 4000",
    )?;
    let citation_rows = citation_statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, u32>(3)?,
            row.get::<_, String>(4)?,
        ))
    })?;
    for row in citation_rows {
        let (id, source, target, evidence_count, category) = row?;
        if source == target
            || degrees.get(&source).copied().unwrap_or_default() >= 12
            || degrees.get(&target).copied().unwrap_or_default() >= 12
        {
            continue;
        }
        let key = format!("citation:{source}:{target}");
        if seen.insert(key) {
            edges.push(GraphEdge {
                id,
                source: source.clone(),
                target: target.clone(),
                relation: "citation".to_owned(),
                weight: 0.9 + (evidence_count.min(3) as f64 * 0.08),
                directed: true,
                evidence_count,
                evidence_label: Some(format!(
                    "{evidence_count} bibliography evidence / {category}"
                )),
            });
            *degrees.entry(source).or_default() += 1;
            *degrees.entry(target).or_default() += 1;
        }
    }
    add_group_edges(
        &mut edges,
        &mut seen,
        &mut degrees,
        group_by(items, |item| {
            item.authors
                .iter()
                .filter(|author| author.name != "Unknown creator")
                .map(|author| normalize_facet(&author.name))
                .collect()
        }),
        "shared-creator",
        0.78,
        80,
    );
    add_group_edges(
        &mut edges,
        &mut seen,
        &mut degrees,
        group_by(items, |item| item.collection_ids.clone()),
        "shared-collection",
        0.68,
        120,
    );
    add_group_edges(
        &mut edges,
        &mut seen,
        &mut degrees,
        group_by(items, |item| {
            item.tags.iter().map(|tag| normalize_facet(tag)).collect()
        }),
        "shared-tag",
        0.56,
        80,
    );

    let mut relation_statement = connection.prepare(
        "SELECT r.item_id, r.target
         FROM item_relations r
         JOIN selected_item_ids s ON s.id = r.item_id
         ORDER BY r.item_id, r.target",
    )?;
    let rows = relation_statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    let key_to_id: HashMap<_, _> = items
        .iter()
        .map(|item| (item.provenance.zotero_key.clone(), item.id.clone()))
        .collect();
    for row in rows {
        let (source, target) = row?;
        if !item_ids.contains(&source) {
            continue;
        }
        let key = target.rsplit('/').next().unwrap_or(&target);
        if let Some(target_id) = key_to_id.get(key) {
            add_edge(
                &mut edges,
                &mut seen,
                &mut degrees,
                &source,
                target_id,
                "zotero-related",
                (0.9, 10),
            );
        }
    }
    let mut available_relations: Vec<_> = edges
        .iter()
        .map(|edge| edge.relation.clone())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    available_relations.sort();
    Ok(ResearchGraph {
        nodes,
        edges,
        available_relations,
        total_node_count: items.len() as u64,
        has_more: false,
    })
}

fn group_by(
    items: &[ResearchItem],
    facets: impl Fn(&ResearchItem) -> Vec<String>,
) -> BTreeMap<String, Vec<String>> {
    let mut groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for item in items {
        for facet in facets(item) {
            if !facet.is_empty() {
                groups.entry(facet).or_default().push(item.id.clone());
            }
        }
    }
    groups
}

fn add_group_edges(
    edges: &mut Vec<GraphEdge>,
    seen: &mut HashSet<String>,
    degrees: &mut HashMap<String, usize>,
    groups: BTreeMap<String, Vec<String>>,
    relation: &str,
    weight: f64,
    maximum_group_size: usize,
) {
    for (_, mut item_ids) in groups {
        item_ids.sort();
        item_ids.dedup();
        if item_ids.len() > maximum_group_size {
            continue;
        }
        for pair in item_ids.windows(2) {
            add_edge(
                edges,
                seen,
                degrees,
                &pair[0],
                &pair[1],
                relation,
                (weight, 6),
            );
        }
    }
}

fn add_edge(
    edges: &mut Vec<GraphEdge>,
    seen: &mut HashSet<String>,
    degrees: &mut HashMap<String, usize>,
    source: &str,
    target: &str,
    relation: &str,
    encoding: (f64, usize),
) {
    if source == target {
        return;
    }
    let (left, right) = if source < target {
        (source, target)
    } else {
        (target, source)
    };
    let key = format!("{relation}:{left}:{right}");
    let (weight, degree_limit) = encoding;
    if degrees.get(left).copied().unwrap_or_default() >= degree_limit
        || degrees.get(right).copied().unwrap_or_default() >= degree_limit
    {
        return;
    }
    if seen.insert(key.clone()) {
        edges.push(GraphEdge {
            id: stable_id("edge", &[relation, left, right]),
            source: left.to_owned(),
            target: right.to_owned(),
            relation: relation.to_owned(),
            weight,
            directed: false,
            evidence_count: 1,
            evidence_label: None,
        });
        *degrees.entry(left.to_owned()).or_default() += 1;
        *degrees.entry(right.to_owned()).or_default() += 1;
    }
}

fn compute_analytics(
    items: &[ResearchItem],
    topics: &[Topic],
    item_topics: &HashMap<String, String>,
    graph: &ResearchGraph,
    corpus: &CorpusDiagnostics,
    citation: CitationAnalytics,
) -> AnalyticsSnapshot {
    let mut year_counts = BTreeMap::new();
    let mut topic_counts: HashMap<String, u64> = HashMap::new();
    for item in items {
        if let Some(year) = item.year {
            *year_counts.entry(year).or_insert(0_u64) += 1;
        }
        let topic = item_topics
            .get(&item.id)
            .cloned()
            .unwrap_or_else(|| "uncategorized".to_owned());
        *topic_counts.entry(topic).or_insert(0_u64) += 1;
    }
    let mut degrees: HashMap<String, u64> = HashMap::new();
    for edge in &graph.edges {
        *degrees.entry(edge.source.clone()).or_default() += 1;
        *degrees.entry(edge.target.clone()).or_default() += 1;
    }
    let mut most_connected = items.to_vec();
    most_connected.sort_by(|left, right| {
        degrees
            .get(&right.id)
            .unwrap_or(&0)
            .cmp(degrees.get(&left.id).unwrap_or(&0))
            .then_with(|| left.title.cmp(&right.title))
    });
    most_connected.truncate(5);
    for item in &mut most_connected {
        item.citation_count = *degrees.get(&item.id).unwrap_or(&0) as u32;
    }
    AnalyticsSnapshot {
        by_topic: topics
            .iter()
            .map(|topic| TopicMetric {
                topic_id: topic.id.clone(),
                count: *topic_counts.get(&topic.id).unwrap_or(&0),
            })
            .collect(),
        by_year: year_counts
            .into_iter()
            .map(|(year, count)| YearMetric { year, count })
            .collect(),
        most_connected,
        pdf_coverage: if items.is_empty() {
            0.0
        } else {
            items
                .iter()
                .filter(|item| {
                    item.attachments
                        .iter()
                        .any(|attachment| attachment.media_type == "application/pdf")
                })
                .count() as f64
                / items.len() as f64
        },
        document_count: corpus.document_count,
        resolved_pdf_count: corpus.resolved_pdf_count,
        full_text_count: corpus.full_text_count,
        partial_text_count: corpus.partial_text_count,
        indexed_pages: corpus.indexed_pages,
        citation,
    }
}

fn load_citation_analytics(
    connection: &Connection,
    library_id: Option<&str>,
    diagnostics: &crate::model::CitationDiagnostics,
) -> AppResult<CitationAnalytics> {
    let Some(library_id) = library_id else {
        return Ok(CitationAnalytics {
            most_cited_local: Vec::new(),
            most_referencing: Vec::new(),
            frequently_referenced_external: Vec::new(),
            resolution_coverage: 0.0,
        });
    };
    let load_paper_metrics = |direction: &str| -> AppResult<Vec<CitationPaperMetric>> {
        let (item_column, library_column) = if direction == "incoming" {
            ("target_item_id", "target_item_id")
        } else {
            ("source_item_id", "source_item_id")
        };
        let sql = format!(
            "SELECT i.id, i.title, COUNT(*)
             FROM citation_edges ce
             JOIN research_items i ON i.id = ce.{item_column}
             WHERE i.library_id = ?1
             GROUP BY ce.{library_column}, i.title
             ORDER BY COUNT(*) DESC, i.title
             LIMIT 8"
        );
        let mut statement = connection.prepare(&sql)?;
        let rows = statement.query_map([library_id], |row| {
            Ok(CitationPaperMetric {
                item_id: row.get(0)?,
                title: row.get(1)?,
                count: row.get::<_, i64>(2)? as u64,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    };
    let mut external_statement = connection.prepare(
        "SELECT w.id, COALESCE(w.title, w.first_author, w.normalized_doi,
                              'Unresolved referenced work'),
                w.publication_year, w.normalized_doi,
                COUNT(DISTINCT r.source_item_id)
         FROM referenced_works w
         JOIN reference_resolutions rr ON rr.external_work_id = w.id
         JOIN bibliographic_references r ON r.id = rr.reference_id
         JOIN reference_extractions e ON e.id = r.extraction_id
         WHERE r.library_id = ?1 AND r.is_current = 1 AND e.is_current = 1
           AND rr.matched_item_id IS NULL
         GROUP BY w.id
         ORDER BY COUNT(DISTINCT r.source_item_id) DESC, 2
         LIMIT 8",
    )?;
    let external = external_statement
        .query_map([library_id], |row| {
            Ok(ExternalWorkMetric {
                work_id: row.get(0)?,
                title: row.get(1)?,
                year: row.get(2)?,
                doi: row.get(3)?,
                referenced_by_count: row.get::<_, i64>(4)? as u64,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let resolved = diagnostics.resolved_local;
    let total = diagnostics.references_extracted;
    Ok(CitationAnalytics {
        most_cited_local: load_paper_metrics("incoming")?,
        most_referencing: load_paper_metrics("outgoing")?,
        frequently_referenced_external: external,
        resolution_coverage: if total == 0 {
            0.0
        } else {
            resolved as f64 / total as f64
        },
    })
}

fn stable_jitter(value: &str) -> (f64, f64) {
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    let bytes = hasher.finalize();
    let x = (u16::from_be_bytes([bytes[0], bytes[1]]) as f64 / 65535.0) - 0.5;
    let y = (u16::from_be_bytes([bytes[2], bytes[3]]) as f64 / 65535.0) - 0.5;
    (x, y)
}
