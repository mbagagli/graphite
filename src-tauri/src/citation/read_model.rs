//! Builds citation query projections and research-neighborhood read models.

use std::collections::HashSet;

use rusqlite::{params, Connection, OptionalExtension};

use crate::bibliography::{EXTRACTOR_VERSION, RESOLVER_VERSION};
use crate::database::{stable_id, ResearchDatabase};
use crate::error::{AppError, AppResult};
use crate::model::{
    BibliographicReferenceView, CitationCoverage, CitationDiagnostics, CitationEvidenceView,
    CitationItemPage, CitationItemSummary, CitationListRequest, CoCitedWorkLink, CoCitedWorkPage,
    EdgeEvidenceRequest, ExternalWorkPage, ExternalWorkSummary, GraphEdge, GraphNode,
    MissingWorksRequest, ReferencePage, ReferenceQueryRequest, RelationshipEvidence,
    ResearchNeighborhood, ResearchNeighborhoodRequest, ResolutionCandidate, StructuralPaperLink,
    StructuralPaperPage,
};

const DEFAULT_REFERENCE_PAGE: u32 = 40;
const MAX_REFERENCE_PAGE: u32 = 100;
const DEFAULT_NEIGHBORHOOD_NODES: u32 = 80;
const MAX_NEIGHBORHOOD_NODES: u32 = 180;
const MAX_NEIGHBORHOOD_EDGES: u32 = 360;

pub fn load_citation_diagnostics(
    connection: &Connection,
    library_id: Option<&str>,
) -> AppResult<CitationDiagnostics> {
    let Some(library_id) = library_id else {
        return Ok(empty_diagnostics());
    };
    let eligible_documents = connection.query_row(
        "SELECT COUNT(DISTINCT d.id)
         FROM documents d
         JOIN document_texts dt
           ON dt.document_id = d.id AND dt.is_active = 1
         WHERE d.library_id = ?1 AND d.research_item_id IS NOT NULL",
        [library_id],
        |row| row.get::<_, i64>(0),
    )? as u64;
    let (indexed_documents, no_section, partial, failed, references, with_doi) = connection
        .query_row(
            "SELECT COUNT(DISTINCT e.document_id),
                    COUNT(DISTINCT CASE WHEN e.status = 'no-reference-section'
                                        THEN e.document_id END),
                    COUNT(DISTINCT CASE WHEN e.status = 'partial-source-text'
                                        THEN e.document_id END),
                    COUNT(DISTINCT CASE WHEN e.status = 'failed'
                                        THEN e.document_id END),
                    COUNT(DISTINCT CASE WHEN r.is_current = 1 THEN r.id END),
                    COUNT(DISTINCT CASE WHEN r.is_current = 1
                                             AND r.normalized_doi IS NOT NULL
                                        THEN r.id END)
             FROM reference_extractions e
             LEFT JOIN bibliographic_references r ON r.extraction_id = e.id
             WHERE e.library_id = ?1 AND e.is_current = 1",
            [library_id],
            |row| {
                Ok((
                    row.get::<_, i64>(0)? as u64,
                    row.get::<_, i64>(1)? as u64,
                    row.get::<_, i64>(2)? as u64,
                    row.get::<_, i64>(3)? as u64,
                    row.get::<_, i64>(4)? as u64,
                    row.get::<_, i64>(5)? as u64,
                ))
            },
        )?;
    let (resolved, external, ambiguous, unresolved) = connection.query_row(
        "SELECT
            COUNT(DISTINCT CASE WHEN rr.matched_item_id IS NOT NULL
                                THEN rr.reference_id END),
            COUNT(DISTINCT CASE WHEN rr.status = 'external'
                                THEN rr.external_work_id END),
            COUNT(DISTINCT CASE WHEN rr.status = 'ambiguous'
                                THEN rr.reference_id END),
            COUNT(DISTINCT CASE WHEN rr.status IN ('unresolved', 'probable')
                                THEN rr.reference_id END)
         FROM reference_resolutions rr
         JOIN bibliographic_references r ON r.id = rr.reference_id
         JOIN reference_extractions e ON e.id = r.extraction_id
         WHERE r.library_id = ?1 AND r.is_current = 1 AND e.is_current = 1",
        [library_id],
        |row| {
            Ok((
                row.get::<_, i64>(0)? as u64,
                row.get::<_, i64>(1)? as u64,
                row.get::<_, i64>(2)? as u64,
                row.get::<_, i64>(3)? as u64,
            ))
        },
    )?;
    let citation_edges = connection.query_row(
        "SELECT COUNT(*) FROM citation_edges ce
         JOIN research_items i ON i.id = ce.source_item_id
         WHERE i.library_id = ?1",
        [library_id],
        |row| row.get::<_, i64>(0),
    )? as u64;
    let queued_resolutions = connection.query_row(
        "SELECT COUNT(*)
         FROM bibliographic_references r
         JOIN reference_extractions e ON e.id = r.extraction_id
         LEFT JOIN reference_resolutions rr ON rr.reference_id = r.id
         LEFT JOIN citation_resolution_queue q ON q.reference_id = r.id
         WHERE r.library_id = ?1 AND r.is_current = 1 AND e.is_current = 1
           AND (q.reference_id IS NOT NULL OR rr.reference_id IS NULL
                OR rr.resolver_version <> ?2)",
        params![library_id, RESOLVER_VERSION],
        |row| row.get::<_, i64>(0),
    )? as u64;
    let state = connection
        .query_row(
            "SELECT status, last_indexed_at, extractor_version,
                    resolver_version, last_error
             FROM citation_index_state WHERE library_id = ?1",
            [library_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                ))
            },
        )
        .optional()?;
    let (status, last_indexed_at, extractor_version, resolver_version, last_error) = state
        .unwrap_or((
            "not-indexed".to_owned(),
            None,
            EXTRACTOR_VERSION.to_owned(),
            RESOLVER_VERSION.to_owned(),
            None,
        ));
    Ok(CitationDiagnostics {
        status,
        eligible_documents,
        indexed_documents,
        no_reference_section: no_section,
        partial_documents: partial,
        failed_documents: failed,
        references_extracted: references,
        references_with_doi: with_doi,
        resolved_local: resolved,
        external_works: external,
        ambiguous_references: ambiguous,
        unresolved_references: unresolved,
        citation_edges,
        queued_documents: eligible_documents.saturating_sub(indexed_documents),
        queued_resolutions,
        last_indexed_at,
        extractor_version,
        resolver_version,
        last_error,
    })
}

fn empty_diagnostics() -> CitationDiagnostics {
    CitationDiagnostics {
        status: "not-indexed".to_owned(),
        eligible_documents: 0,
        indexed_documents: 0,
        no_reference_section: 0,
        partial_documents: 0,
        failed_documents: 0,
        references_extracted: 0,
        references_with_doi: 0,
        resolved_local: 0,
        external_works: 0,
        ambiguous_references: 0,
        unresolved_references: 0,
        citation_edges: 0,
        queued_documents: 0,
        queued_resolutions: 0,
        last_indexed_at: None,
        extractor_version: EXTRACTOR_VERSION.to_owned(),
        resolver_version: RESOLVER_VERSION.to_owned(),
        last_error: None,
    }
}

pub fn reference_page(
    database: &ResearchDatabase,
    request: &ReferenceQueryRequest,
) -> AppResult<ReferencePage> {
    let connection = database.connect()?;
    let limit = request
        .limit
        .unwrap_or(DEFAULT_REFERENCE_PAGE)
        .clamp(1, MAX_REFERENCE_PAGE);
    let offset = request.offset.unwrap_or(0);
    let filter_condition = match request.filter.as_deref().unwrap_or("all") {
        "local" => " AND rr.matched_item_id IS NOT NULL",
        "external" => " AND rr.status = 'external'",
        "ambiguous" => " AND rr.status = 'ambiguous'",
        "unresolved" => " AND rr.status IN ('unresolved', 'probable')",
        _ => "",
    };
    let base = format!(
        " FROM bibliographic_references r
          JOIN reference_extractions e ON e.id = r.extraction_id
          JOIN reference_resolutions rr ON rr.reference_id = r.id
          LEFT JOIN research_items i ON i.id = rr.matched_item_id
          LEFT JOIN referenced_works w ON w.id = rr.external_work_id
          LEFT JOIN reference_resolution_overrides o ON o.reference_id = r.id
          WHERE r.source_item_id = ?1 AND r.is_current = 1
            AND e.is_current = 1{filter_condition}"
    );
    let total = connection.query_row(
        &format!("SELECT COUNT(*){base}"),
        [&request.item_id],
        |row| row.get::<_, i64>(0),
    )? as u64;
    let sql = format!(
        "SELECT r.id, r.ordinal, r.raw_text, r.original_doi,
                r.normalized_doi, r.parsed_title, r.parsed_year,
                r.first_author, r.parsed_venue, r.extraction_confidence,
                r.page_number, r.source_start, r.source_end,
                rr.status, rr.method, rr.confidence_category,
                rr.evidence_json,
                i.id, i.title,
                (SELECT c.display_name FROM creators c
                 WHERE c.item_id = i.id AND c.creator_type = 'author'
                 ORDER BY c.position LIMIT 1),
                i.publication_year, i.venue, i.doi,
                w.id, w.identity_kind, w.title, w.first_author,
                w.publication_year, w.venue, w.normalized_doi,
                (SELECT COUNT(DISTINCT br.source_item_id)
                 FROM reference_resolutions x
                 JOIN bibliographic_references br ON br.id = x.reference_id
                 JOIN reference_extractions bx ON bx.id = br.extraction_id
                 WHERE x.external_work_id = w.id AND br.is_current = 1
                   AND bx.is_current = 1),
                COALESCE(o.stale, 0)
         {base}
         ORDER BY r.ordinal, r.id
         LIMIT ?2 OFFSET ?3"
    );
    let mut statement = connection.prepare(&sql)?;
    let rows = statement.query_map(params![request.item_id, limit, offset], |row| {
        let evidence_json: String = row.get(16)?;
        Ok(BibliographicReferenceView {
            id: row.get(0)?,
            ordinal: row.get(1)?,
            raw_text: row.get(2)?,
            original_doi: row.get(3)?,
            normalized_doi: row.get(4)?,
            parsed_title: row.get(5)?,
            parsed_year: row.get(6)?,
            first_author: row.get(7)?,
            parsed_venue: row.get(8)?,
            extraction_confidence: row.get(9)?,
            page_number: row.get(10)?,
            source_start: row.get::<_, Option<i64>>(11)?.map(|value| value as u64),
            source_end: row.get::<_, Option<i64>>(12)?.map(|value| value as u64),
            resolution_status: row.get(13)?,
            resolution_method: row.get(14)?,
            confidence_category: row.get(15)?,
            resolution_evidence: serde_json::from_str(&evidence_json).unwrap_or_default(),
            matched_item: optional_item_summary(row, 17)?,
            external_work: optional_external_summary(row, 23)?,
            candidates: Vec::new(),
            override_stale: row.get(31)?,
        })
    })?;
    let mut references = rows.collect::<Result<Vec<_>, _>>()?;
    drop(statement);
    for reference in &mut references {
        reference.candidates = load_candidates(&connection, &reference.id)?;
    }
    Ok(ReferencePage {
        coverage: citation_coverage(&connection, &request.item_id)?,
        references,
        total,
        offset,
        limit,
    })
}

pub fn cited_by_page(
    database: &ResearchDatabase,
    request: &CitationListRequest,
) -> AppResult<CitationItemPage> {
    let connection = database.connect()?;
    let limit = request.limit.unwrap_or(25).clamp(1, 100);
    let offset = request.offset.unwrap_or(0);
    let total = connection.query_row(
        "SELECT COUNT(*) FROM citation_edges WHERE target_item_id = ?1",
        [&request.item_id],
        |row| row.get::<_, i64>(0),
    )? as u64;
    let mut statement = connection.prepare(
        "SELECT i.id, i.title,
                (SELECT c.display_name FROM creators c
                 WHERE c.item_id = i.id AND c.creator_type = 'author'
                 ORDER BY c.position LIMIT 1),
                i.publication_year, i.venue, i.doi
         FROM citation_edges ce
         JOIN research_items i ON i.id = ce.source_item_id
         WHERE ce.target_item_id = ?1
         ORDER BY i.publication_year DESC, i.title
         LIMIT ?2 OFFSET ?3",
    )?;
    let items = statement
        .query_map(params![request.item_id, limit, offset], |row| {
            item_summary(row, 0)
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(CitationItemPage {
        items,
        total,
        offset,
        limit,
    })
}

pub fn shared_reference_papers(
    database: &ResearchDatabase,
    item_id: &str,
    limit: u32,
) -> AppResult<StructuralPaperPage> {
    let connection = database.connect()?;
    let links = load_coupling_links(&connection, item_id, limit.clamp(1, 50))?;
    let total = links.len() as u64;
    Ok(StructuralPaperPage { links, total })
}

pub fn co_cited_works(
    database: &ResearchDatabase,
    item_id: &str,
    limit: u32,
) -> AppResult<CoCitedWorkPage> {
    let connection = database.connect()?;
    let links = load_co_cited_links(&connection, item_id, limit.clamp(1, 50))?;
    let total = links.len() as u64;
    Ok(CoCitedWorkPage { links, total })
}

pub fn missing_works(
    database: &ResearchDatabase,
    request: &MissingWorksRequest,
) -> AppResult<ExternalWorkPage> {
    let connection = database.connect()?;
    let Some((_, _, library)) = database.active_library_context()? else {
        return Ok(ExternalWorkPage {
            works: Vec::new(),
            total: 0,
            offset: 0,
            limit: request.limit.unwrap_or(25).clamp(1, 100),
        });
    };
    let limit = request.limit.unwrap_or(25).clamp(1, 100);
    let offset = request.offset.unwrap_or(0);
    let total = connection.query_row(
        "SELECT COUNT(DISTINCT rr.external_work_id)
         FROM reference_resolutions rr
         JOIN bibliographic_references r ON r.id = rr.reference_id
         JOIN reference_extractions e ON e.id = r.extraction_id
         WHERE r.library_id = ?1 AND r.is_current = 1 AND e.is_current = 1
           AND rr.matched_item_id IS NULL AND rr.status = 'external'",
        [&library.id],
        |row| row.get::<_, i64>(0),
    )? as u64;
    let mut statement = connection.prepare(
        "SELECT w.id, w.identity_kind, w.title, w.first_author,
                w.publication_year, w.venue, w.normalized_doi,
                COUNT(DISTINCT r.source_item_id)
         FROM referenced_works w
         JOIN reference_resolutions rr ON rr.external_work_id = w.id
         JOIN bibliographic_references r ON r.id = rr.reference_id
         JOIN reference_extractions e ON e.id = r.extraction_id
         WHERE r.library_id = ?1 AND r.is_current = 1 AND e.is_current = 1
           AND rr.matched_item_id IS NULL AND rr.status = 'external'
         GROUP BY w.id
         ORDER BY COUNT(DISTINCT r.source_item_id) DESC,
                  COALESCE(w.title, w.normalized_doi, w.id)
         LIMIT ?2 OFFSET ?3",
    )?;
    let works = statement
        .query_map(params![library.id, limit, offset], |row| {
            external_summary(row, 0)
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ExternalWorkPage {
        works,
        total,
        offset,
        limit,
    })
}

pub fn relationship_evidence(
    database: &ResearchDatabase,
    request: &EdgeEvidenceRequest,
) -> AppResult<Option<RelationshipEvidence>> {
    let connection = database.connect()?;
    let edge = connection
        .query_row(
            "SELECT ce.source_item_id, ce.target_item_id, ce.evidence_count
             FROM citation_edges ce WHERE ce.id = ?1",
            [&request.edge_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, u32>(2)?,
                ))
            },
        )
        .optional()?;
    let Some((source_id, target_id, count)) = edge else {
        return Ok(None);
    };
    let source = load_item_summary(&connection, &source_id)?;
    let target = load_item_summary(&connection, &target_id)?;
    let mut statement = connection.prepare(
        "SELECT r.id, r.ordinal, r.raw_text, rr.method,
                rr.confidence_category, d.title, d.id, r.page_number,
                e.provider, e.extractor_version, rr.resolver_version
         FROM citation_evidence ce
         JOIN bibliographic_references r ON r.id = ce.reference_id
         JOIN reference_resolutions rr ON rr.reference_id = r.id
         JOIN reference_extractions e ON e.id = r.extraction_id
         JOIN documents d ON d.id = r.source_document_id
         WHERE ce.citation_edge_id = ?1
         ORDER BY r.ordinal",
    )?;
    let evidence = statement
        .query_map([&request.edge_id], |row| {
            Ok(CitationEvidenceView {
                reference_id: row.get(0)?,
                ordinal: row.get(1)?,
                raw_text: row.get(2)?,
                resolution_method: row.get(3)?,
                confidence_category: row.get(4)?,
                source_document_title: row.get(5)?,
                source_document_id: row.get(6)?,
                page_number: row.get(7)?,
                extractor_provider: row.get(8)?,
                extractor_version: row.get(9)?,
                resolver_version: row.get(10)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Some(RelationshipEvidence {
        edge_id: request.edge_id.clone(),
        relation: "citation".to_owned(),
        source,
        target,
        evidence_label: format!("{count} local bibliography evidence record(s)"),
        evidence,
    }))
}

pub fn neighborhood(
    database: &ResearchDatabase,
    request: &ResearchNeighborhoodRequest,
) -> AppResult<ResearchNeighborhood> {
    let seed = request.seed_item_ids.first().ok_or_else(|| {
        AppError::ApplicationData("A citation neighborhood requires a seed paper".to_owned())
    })?;
    let connection = database.connect()?;
    load_item_summary(&connection, seed)?;
    let max_nodes = request
        .max_nodes
        .unwrap_or(DEFAULT_NEIGHBORHOOD_NODES)
        .clamp(2, MAX_NEIGHBORHOOD_NODES);
    let max_edges = request
        .max_edges
        .unwrap_or(max_nodes * 2)
        .clamp(1, MAX_NEIGHBORHOOD_EDGES);
    let offset = request.offset.unwrap_or(0);
    let mut local_ids = vec![seed.clone()];
    let mut external_nodes = Vec::new();
    let mut focus_edges = Vec::new();
    let total = match request.mode.as_str() {
        "references" => load_reference_neighborhood(
            &connection,
            seed,
            max_nodes - 1,
            offset,
            &mut local_ids,
            &mut external_nodes,
            &mut focus_edges,
        )?,
        "cited-by" => load_cited_by_neighborhood(
            &connection,
            seed,
            max_nodes - 1,
            offset,
            &mut local_ids,
            &mut focus_edges,
        )?,
        "shared-references" => {
            let links = load_coupling_links(&connection, seed, max_nodes - 1)?;
            let total = links.len() as u64;
            for link in links {
                local_ids.push(link.item.id.clone());
                focus_edges.push(GraphEdge {
                    id: stable_id("coupling-edge", &[seed, &link.item.id]),
                    source: seed.clone(),
                    target: link.item.id,
                    relation: "bibliographic-coupling".to_owned(),
                    weight: 0.55 + link.normalized_strength * 0.7,
                    directed: false,
                    evidence_count: link.count as u32,
                    evidence_label: Some(link.label),
                });
            }
            total
        }
        "co-cited" => {
            let links = load_co_cited_links(&connection, seed, max_nodes - 1)?;
            let total = links.len() as u64;
            for link in links {
                if let Some(item) = link.local_item {
                    local_ids.push(item.id.clone());
                    focus_edges.push(GraphEdge {
                        id: stable_id("co-cited-edge", &[seed, &item.id]),
                        source: seed.clone(),
                        target: item.id,
                        relation: "co-cited".to_owned(),
                        weight: 0.55 + (link.count.min(12) as f64 * 0.04),
                        directed: false,
                        evidence_count: link.count as u32,
                        evidence_label: Some(link.label),
                    });
                } else if let Some(work) = link.external_work {
                    let node_id = work.id.clone();
                    external_nodes.push(external_node(&work, external_nodes.len()));
                    focus_edges.push(GraphEdge {
                        id: stable_id("co-cited-edge", &[seed, &node_id]),
                        source: seed.clone(),
                        target: node_id,
                        relation: "co-cited".to_owned(),
                        weight: 0.55 + (link.count.min(12) as f64 * 0.04),
                        directed: false,
                        evidence_count: link.count as u32,
                        evidence_label: Some(link.label),
                    });
                }
            }
            total
        }
        _ => load_combined_neighborhood(
            &connection,
            seed,
            max_nodes - 1,
            offset,
            &mut local_ids,
            &mut external_nodes,
            &mut focus_edges,
        )?,
    };
    local_ids.sort();
    local_ids.dedup();
    let mut projection = database.projection_for_item_ids(&local_ids)?;
    position_focus_nodes(&mut projection.graph.nodes, seed, &request.mode);
    projection.graph.nodes.extend(external_nodes);
    focus_edges.truncate(max_edges as usize);
    projection.graph.edges = focus_edges;
    projection.graph.available_relations = projection
        .graph
        .edges
        .iter()
        .map(|edge| edge.relation.clone())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    projection.graph.available_relations.sort();
    projection.graph.total_node_count = total + 1;
    projection.graph.has_more = total > (projection.graph.nodes.len().saturating_sub(1)) as u64;
    let visible_relationships = projection.graph.edges.len() as u64;
    Ok(ResearchNeighborhood {
        projection,
        mode: request.mode.clone(),
        total_relationships: total,
        hidden_relationships: total.saturating_sub(visible_relationships),
    })
}

fn citation_coverage(connection: &Connection, item_id: &str) -> AppResult<CitationCoverage> {
    let extraction_status = connection
        .query_row(
            "SELECT status FROM reference_extractions
             WHERE source_item_id = ?1 AND is_current = 1
             ORDER BY CASE status WHEN 'references-extracted' THEN 0 ELSE 1 END,
                      indexed_at DESC LIMIT 1",
            [item_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .unwrap_or_else(|| "not-indexed".to_owned());
    let (total, local, external, ambiguous, unresolved) = connection.query_row(
        "SELECT COUNT(*),
                SUM(CASE WHEN rr.matched_item_id IS NOT NULL THEN 1 ELSE 0 END),
                SUM(CASE WHEN rr.status = 'external' THEN 1 ELSE 0 END),
                SUM(CASE WHEN rr.status = 'ambiguous' THEN 1 ELSE 0 END),
                SUM(CASE WHEN rr.status IN ('unresolved', 'probable')
                         THEN 1 ELSE 0 END)
         FROM bibliographic_references r
         JOIN reference_extractions e ON e.id = r.extraction_id
         JOIN reference_resolutions rr ON rr.reference_id = r.id
         WHERE r.source_item_id = ?1 AND r.is_current = 1 AND e.is_current = 1",
        [item_id],
        |row| {
            Ok((
                row.get::<_, i64>(0)? as u64,
                row.get::<_, Option<i64>>(1)?.unwrap_or(0) as u64,
                row.get::<_, Option<i64>>(2)?.unwrap_or(0) as u64,
                row.get::<_, Option<i64>>(3)?.unwrap_or(0) as u64,
                row.get::<_, Option<i64>>(4)?.unwrap_or(0) as u64,
            ))
        },
    )?;
    let cited_by = connection.query_row(
        "SELECT COUNT(*) FROM citation_edges WHERE target_item_id = ?1",
        [item_id],
        |row| row.get::<_, i64>(0),
    )? as u64;
    Ok(CitationCoverage {
        item_id: item_id.to_owned(),
        extraction_status,
        total_references: total,
        resolved_local: local,
        external,
        ambiguous,
        unresolved,
        cited_by_local: cited_by,
    })
}

fn load_candidates(
    connection: &Connection,
    reference_id: &str,
) -> AppResult<Vec<ResolutionCandidate>> {
    let mut statement = connection.prepare(
        "SELECT i.id, i.title,
                (SELECT c.display_name FROM creators c
                 WHERE c.item_id = i.id AND c.creator_type = 'author'
                 ORDER BY c.position LIMIT 1),
                i.publication_year, i.venue, i.doi,
                c.category, c.score, c.evidence_json
         FROM reference_resolution_candidates c
         JOIN research_items i ON i.id = c.candidate_item_id
         WHERE c.reference_id = ?1 ORDER BY c.rank LIMIT 8",
    )?;
    let rows = statement.query_map([reference_id], |row| {
        let evidence_json: String = row.get(8)?;
        Ok(ResolutionCandidate {
            item: item_summary(row, 0)?,
            category: row.get(6)?,
            score: row.get(7)?,
            evidence: serde_json::from_str(&evidence_json).unwrap_or_default(),
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

fn load_coupling_links(
    connection: &Connection,
    item_id: &str,
    limit: u32,
) -> AppResult<Vec<StructuralPaperLink>> {
    let mut statement = connection.prepare(
        "WITH authoritative AS (
            SELECT r.source_item_id,
                   COALESCE('local:' || rr.matched_item_id,
                            'external:' || rr.external_work_id) AS work_key
            FROM bibliographic_references r
            JOIN reference_extractions e ON e.id = r.extraction_id
            JOIN reference_resolutions rr ON rr.reference_id = r.id
            WHERE r.is_current = 1 AND e.is_current = 1
              AND e.status = 'references-extracted'
              AND rr.status IN ('exact-identifier', 'strong-metadata',
                                'manual-resolved', 'external')
        ), seed AS (
            SELECT DISTINCT work_key FROM authoritative WHERE source_item_id = ?1
        ), shared AS (
            SELECT a.source_item_id, COUNT(DISTINCT a.work_key) AS shared_count
            FROM authoritative a JOIN seed s ON s.work_key = a.work_key
            WHERE a.source_item_id <> ?1 GROUP BY a.source_item_id
            HAVING shared_count >= 2
        ), totals AS (
            SELECT source_item_id, COUNT(DISTINCT work_key) AS reference_count
            FROM authoritative GROUP BY source_item_id
        ), seed_total AS (
            SELECT COUNT(*) AS reference_count FROM seed
        )
        SELECT i.id, i.title,
               (SELECT c.display_name FROM creators c
                WHERE c.item_id = i.id AND c.creator_type = 'author'
                ORDER BY c.position LIMIT 1),
               i.publication_year, i.venue, i.doi,
               shared.shared_count,
               CAST(shared.shared_count AS REAL) /
               MAX(1, seed_total.reference_count + totals.reference_count
                      - shared.shared_count)
        FROM shared
        JOIN research_items i ON i.id = shared.source_item_id
        JOIN totals ON totals.source_item_id = shared.source_item_id
        CROSS JOIN seed_total
        ORDER BY shared.shared_count DESC, 8 DESC, i.title
        LIMIT ?2",
    )?;
    let rows = statement.query_map(params![item_id, limit], |row| {
        let count = row.get::<_, i64>(6)? as u64;
        Ok(StructuralPaperLink {
            item: item_summary(row, 0)?,
            count,
            normalized_strength: row.get(7)?,
            label: format!("{count} shared references in your library corpus"),
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

fn load_co_cited_links(
    connection: &Connection,
    item_id: &str,
    limit: u32,
) -> AppResult<Vec<CoCitedWorkLink>> {
    let mut statement = connection.prepare(
        "WITH citing_sources AS (
            SELECT source_item_id FROM citation_edges WHERE target_item_id = ?1
        ), companions AS (
            SELECT rr.matched_item_id, rr.external_work_id,
                   COUNT(DISTINCT r.source_item_id) AS source_count
            FROM bibliographic_references r
            JOIN reference_extractions e ON e.id = r.extraction_id
            JOIN reference_resolutions rr ON rr.reference_id = r.id
            JOIN citing_sources cs ON cs.source_item_id = r.source_item_id
            WHERE r.is_current = 1 AND e.is_current = 1
              AND e.status = 'references-extracted'
              AND rr.status IN ('exact-identifier', 'strong-metadata',
                                'manual-resolved', 'external')
              AND (rr.matched_item_id IS NULL OR rr.matched_item_id <> ?1)
            GROUP BY COALESCE(rr.matched_item_id, rr.external_work_id)
        )
        SELECT i.id, i.title,
               (SELECT c.display_name FROM creators c
                WHERE c.item_id = i.id AND c.creator_type = 'author'
                ORDER BY c.position LIMIT 1),
               i.publication_year, i.venue, i.doi,
               w.id, w.identity_kind, w.title, w.first_author,
               w.publication_year, w.venue, w.normalized_doi,
               companions.source_count,
               (SELECT COUNT(DISTINCT br.source_item_id)
                FROM reference_resolutions rx
                JOIN bibliographic_references br ON br.id = rx.reference_id
                WHERE rx.external_work_id = w.id)
        FROM companions
        LEFT JOIN research_items i ON i.id = companions.matched_item_id
        LEFT JOIN referenced_works w ON w.id = companions.external_work_id
        ORDER BY companions.source_count DESC,
                 COALESCE(i.title, w.title, w.normalized_doi)
        LIMIT ?2",
    )?;
    let rows = statement.query_map(params![item_id, limit], |row| {
        let count = row.get::<_, i64>(13)? as u64;
        Ok(CoCitedWorkLink {
            local_item: optional_item_summary(row, 0)?,
            external_work: optional_external_summary(row, 6)?,
            count,
            label: format!("Co-cited in {count} paper(s) in your library"),
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

#[allow(clippy::too_many_arguments)]
fn load_reference_neighborhood(
    connection: &Connection,
    seed: &str,
    limit: u32,
    offset: u32,
    local_ids: &mut Vec<String>,
    external_nodes: &mut Vec<GraphNode>,
    edges: &mut Vec<GraphEdge>,
) -> AppResult<u64> {
    let total = connection.query_row(
        "SELECT COUNT(DISTINCT COALESCE(rr.matched_item_id,
                                        rr.external_work_id))
         FROM bibliographic_references r
         JOIN reference_extractions e ON e.id = r.extraction_id
         JOIN reference_resolutions rr ON rr.reference_id = r.id
         WHERE r.source_item_id = ?1 AND r.is_current = 1 AND e.is_current = 1
           AND e.status = 'references-extracted'
           AND rr.status IN ('exact-identifier', 'strong-metadata',
                             'manual-resolved', 'external')",
        [seed],
        |row| row.get::<_, i64>(0),
    )? as u64;
    let mut statement = connection.prepare(
        "WITH reference_targets AS (
            SELECT MIN(r.id) AS reference_id,
                   rr.matched_item_id,
                   rr.external_work_id,
                   MIN(rr.confidence_category) AS confidence_category,
                   MIN(r.ordinal) AS first_ordinal,
                   COUNT(*) AS evidence_count
            FROM bibliographic_references r
            JOIN reference_extractions e ON e.id = r.extraction_id
            JOIN reference_resolutions rr ON rr.reference_id = r.id
            WHERE r.source_item_id = ?1
              AND r.is_current = 1
              AND e.is_current = 1
              AND e.status = 'references-extracted'
              AND rr.status IN ('exact-identifier', 'strong-metadata',
                                'manual-resolved', 'external')
            GROUP BY rr.matched_item_id, rr.external_work_id
         )
         SELECT t.reference_id, t.matched_item_id, t.external_work_id,
                t.confidence_category,
                w.id, w.identity_kind, w.title, w.first_author,
                w.publication_year, w.venue, w.normalized_doi,
                (SELECT COUNT(DISTINCT br.source_item_id)
                 FROM reference_resolutions rx
                 JOIN bibliographic_references br ON br.id = rx.reference_id
                 WHERE rx.external_work_id = w.id),
                t.evidence_count
         FROM reference_targets t
         LEFT JOIN referenced_works w ON w.id = t.external_work_id
         ORDER BY CASE WHEN t.matched_item_id IS NULL THEN 1 ELSE 0 END,
                  t.first_ordinal
         LIMIT ?2 OFFSET ?3",
    )?;
    let rows = statement.query_map(params![seed, limit, offset], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, String>(3)?,
            optional_external_summary(row, 4)?,
            row.get::<_, u32>(12)?,
        ))
    })?;
    for row in rows {
        let (reference_id, local, external_id, category, external, evidence_count) = row?;
        let target = if let Some(local) = local {
            local_ids.push(local.clone());
            local
        } else if let Some(work) = external {
            let id = work.id.clone();
            external_nodes.push(external_node(&work, external_nodes.len()));
            id
        } else if let Some(id) = external_id {
            id
        } else {
            continue;
        };
        edges.push(GraphEdge {
            id: if target.starts_with("external-work_") {
                stable_id("external-citation", &[seed, &target, &reference_id])
            } else {
                stable_id("citation-edge", &[seed, &target])
            },
            source: seed.to_owned(),
            target,
            relation: "citation".to_owned(),
            weight: 1.0,
            directed: true,
            evidence_count,
            evidence_label: Some(category),
        });
    }
    Ok(total)
}

fn load_cited_by_neighborhood(
    connection: &Connection,
    seed: &str,
    limit: u32,
    offset: u32,
    local_ids: &mut Vec<String>,
    edges: &mut Vec<GraphEdge>,
) -> AppResult<u64> {
    let total = connection.query_row(
        "SELECT COUNT(*) FROM citation_edges WHERE target_item_id = ?1",
        [seed],
        |row| row.get::<_, i64>(0),
    )? as u64;
    let mut statement = connection.prepare(
        "SELECT id, source_item_id, evidence_count, strongest_category
         FROM citation_edges WHERE target_item_id = ?1
         ORDER BY evidence_count DESC, source_item_id
         LIMIT ?2 OFFSET ?3",
    )?;
    let rows = statement.query_map(params![seed, limit, offset], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, u32>(2)?,
            row.get::<_, String>(3)?,
        ))
    })?;
    for row in rows {
        let (id, source, count, category) = row?;
        local_ids.push(source.clone());
        edges.push(GraphEdge {
            id,
            source,
            target: seed.to_owned(),
            relation: "citation".to_owned(),
            weight: 1.0,
            directed: true,
            evidence_count: count,
            evidence_label: Some(category),
        });
    }
    Ok(total)
}

#[allow(clippy::too_many_arguments)]
fn load_combined_neighborhood(
    connection: &Connection,
    seed: &str,
    limit: u32,
    offset: u32,
    local_ids: &mut Vec<String>,
    external_nodes: &mut Vec<GraphNode>,
    edges: &mut Vec<GraphEdge>,
) -> AppResult<u64> {
    let outgoing_limit = limit.div_ceil(2);
    let outgoing = load_reference_neighborhood(
        connection,
        seed,
        outgoing_limit,
        offset,
        local_ids,
        external_nodes,
        edges,
    )?;
    let incoming = load_cited_by_neighborhood(
        connection,
        seed,
        limit.saturating_sub(outgoing_limit),
        offset,
        local_ids,
        edges,
    )?;
    Ok(outgoing + incoming)
}

fn external_node(work: &ExternalWorkSummary, index: usize) -> GraphNode {
    let angle = index as f64 * 2.399_963_229_728_653;
    let radius = 5.0 + (index % 5) as f64 * 0.55;
    let label = work
        .title
        .clone()
        .or_else(|| work.doi.clone())
        .unwrap_or_else(|| "Unresolved referenced work".to_owned());
    GraphNode {
        id: work.id.clone(),
        paper_id: None,
        entity_kind: "external-work".to_owned(),
        label: truncate_graph_label(&label, 64),
        detail: Some(
            [
                work.first_author.clone(),
                work.year.map(|year| year.to_string()),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" / "),
        ),
        topic_id: "external-reference".to_owned(),
        x: angle.cos() * radius,
        y: angle.sin() * radius,
        importance: 0.28 + (work.referenced_by_count.min(12) as f64 * 0.025),
    }
}

fn truncate_graph_label(value: &str, max_characters: usize) -> String {
    if value.chars().count() <= max_characters {
        return value.to_owned();
    }
    let mut shortened = value.chars().take(max_characters).collect::<String>();
    shortened.push_str("...");
    shortened
}

fn position_focus_nodes(nodes: &mut [GraphNode], seed: &str, mode: &str) {
    let others = nodes.iter().filter(|node| node.id != seed).count().max(1);
    let mut index = 0;
    for node in nodes {
        if node.id == seed {
            node.x = 0.0;
            node.y = 0.0;
            node.importance = node.importance.max(1.0);
            continue;
        }
        let angle = std::f64::consts::TAU * index as f64 / others as f64;
        let horizontal = match mode {
            "references" => -4.8,
            "cited-by" => 4.8,
            _ => angle.cos() * 5.2,
        };
        node.x = horizontal + angle.cos() * 1.4;
        node.y = angle.sin() * 4.2;
        index += 1;
    }
}

fn load_item_summary(connection: &Connection, item_id: &str) -> AppResult<CitationItemSummary> {
    connection
        .query_row(
            "SELECT i.id, i.title,
                    (SELECT c.display_name FROM creators c
                     WHERE c.item_id = i.id AND c.creator_type = 'author'
                     ORDER BY c.position LIMIT 1),
                    i.publication_year, i.venue, i.doi
             FROM research_items i WHERE i.id = ?1 AND i.is_trashed = 0",
            [item_id],
            |row| item_summary(row, 0),
        )
        .optional()?
        .ok_or_else(|| AppError::ApplicationData("The research item is unavailable".to_owned()))
}

fn item_summary(row: &rusqlite::Row<'_>, offset: usize) -> rusqlite::Result<CitationItemSummary> {
    Ok(CitationItemSummary {
        id: row.get(offset)?,
        title: row.get(offset + 1)?,
        first_author: row.get(offset + 2)?,
        year: row.get(offset + 3)?,
        venue: row.get(offset + 4)?,
        doi: row.get(offset + 5)?,
    })
}

fn optional_item_summary(
    row: &rusqlite::Row<'_>,
    offset: usize,
) -> rusqlite::Result<Option<CitationItemSummary>> {
    if row.get::<_, Option<String>>(offset)?.is_none() {
        return Ok(None);
    }
    item_summary(row, offset).map(Some)
}

fn external_summary(
    row: &rusqlite::Row<'_>,
    offset: usize,
) -> rusqlite::Result<ExternalWorkSummary> {
    Ok(ExternalWorkSummary {
        id: row.get(offset)?,
        identity_kind: row.get(offset + 1)?,
        title: row.get(offset + 2)?,
        first_author: row.get(offset + 3)?,
        year: row.get(offset + 4)?,
        venue: row.get(offset + 5)?,
        doi: row.get(offset + 6)?,
        referenced_by_count: row.get::<_, Option<i64>>(offset + 7)?.unwrap_or(0) as u64,
    })
}

fn optional_external_summary(
    row: &rusqlite::Row<'_>,
    offset: usize,
) -> rusqlite::Result<Option<ExternalWorkSummary>> {
    if row.get::<_, Option<String>>(offset)?.is_none() {
        return Ok(None);
    }
    external_summary(row, offset).map(Some)
}

/// Read the concrete works supporting a displayed structural connection.
pub fn connection_details(
    database: &ResearchDatabase,
    source_id: &str,
    target_id: &str,
    relation: &str,
) -> AppResult<Vec<String>> {
    let connection = database.connect()?;
    let authoritative = "WITH authoritative AS (
        SELECT r.source_item_id,
            COALESCE(rr.matched_item_id, rr.external_work_id) AS work_id,
            COALESCE(i.title, w.title, r.parsed_title, r.raw_text) AS title,
            r.raw_text
        FROM bibliographic_references r
        JOIN reference_extractions e ON e.id = r.extraction_id
        JOIN reference_resolutions rr ON rr.reference_id = r.id
        LEFT JOIN research_items i ON i.id = rr.matched_item_id
        LEFT JOIN referenced_works w ON w.id = rr.external_work_id
        WHERE r.is_current = 1 AND e.is_current = 1
          AND e.status = 'references-extracted'
          AND rr.status IN ('exact-identifier', 'strong-metadata',
                            'manual-resolved', 'external')
    ) ";
    let query = match relation {
        "bibliographic-coupling" => {
            "SELECT DISTINCT a.title FROM authoritative a
            JOIN authoritative b ON b.work_id = a.work_id
            WHERE a.source_item_id = ?1 AND b.source_item_id = ?2 ORDER BY a.title"
        }
        "co-cited" => {
            "SELECT DISTINCT i.title FROM authoritative a
            JOIN authoritative b ON b.source_item_id = a.source_item_id
            JOIN research_items i ON i.id = a.source_item_id
            WHERE a.work_id = ?1 AND b.work_id = ?2 ORDER BY i.title"
        }
        "citation" => {
            "SELECT DISTINCT raw_text FROM authoritative
            WHERE source_item_id = ?1 AND work_id = ?2 ORDER BY raw_text"
        }
        _ => return Ok(Vec::new()),
    };
    let mut statement = connection.prepare(&format!("{authoritative}{query}"))?;
    let rows = statement.query_map(params![source_id, target_id], |row| row.get(0))?;
    rows.collect::<Result<Vec<String>, _>>().map_err(Into::into)
}
